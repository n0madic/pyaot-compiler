//! Slab allocator for small GC-managed objects
//!
//! Replaces system malloc for objects ≤ 64 bytes with bump-pointer allocation
//! from pre-allocated 4KB pages. Objects are grouped by size class for zero
//! fragmentation within a class.
//!
//! ## Size Classes
//!
//! | Class | Slot Size | Typical Objects |
//! |-------|-----------|-----------------|
//! | 0     | 24 bytes  | IntObj, FloatObj, BoolObj |
//! | 1     | 32 bytes  | SetObj, short strings (≤8 chars) |
//! | 2     | 48 bytes  | InstanceObj (1 field), strings (≤24 chars) |
//! | 3     | 64 bytes  | ListObj, DictObj, strings (≤40 chars) |
//!
//! ## Allocation Strategy
//!
//! 1. Try free list (O(1) pop from linked list of recycled slots)
//! 2. Try bump allocation from current page (O(1) pointer increment)
//! 3. Allocate new 4KB page (rare, amortized over ~64-170 allocations)
//!
//! ## GC Integration
//!
//! Slab-allocated objects are NOT stored in `GcState.objects` Vec.
//! Instead, sweep iterates slab pages directly, eliminating Vec
//! reallocation overhead for small objects.
//!
//! Free slots are identified by `ObjHeader.size == 0`.

use crate::object::{Obj, TypeTagKind};
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::cell::UnsafeCell;

const PAGE_SIZE: usize = 4096;
const NUM_SIZE_CLASSES: usize = 4;
const SIZE_CLASSES: [usize; NUM_SIZE_CLASSES] = [24, 32, 48, 64];

/// Maximum object size handled by the slab allocator
pub const SLAB_MAX_SIZE: usize = 64;

/// Information about an allocated page
struct PageInfo {
    /// Pointer to the page data
    ptr: *mut u8,
    /// High-water mark: how many bytes have been bump-allocated
    /// Only slots at offsets 0..allocated_up_to have ever been used
    allocated_up_to: usize,
}

/// A single size class with its own pages and free list
struct SizeClass {
    slot_size: usize,
    /// Number of usable slots per page
    slots_per_page: usize,
    /// Bump cursor: next free byte in current page
    cursor: *mut u8,
    /// End of usable area in current page
    cursor_end: *mut u8,
    /// Free list head (linked through first 8 bytes of free slots)
    free_head: *mut u8,
    /// All pages for this size class
    pages: Vec<PageInfo>,
}

impl SizeClass {
    const fn new(slot_size: usize) -> Self {
        let slots_per_page = PAGE_SIZE / slot_size;
        Self {
            slot_size,
            slots_per_page,
            cursor: std::ptr::null_mut(),
            cursor_end: std::ptr::null_mut(),
            free_head: std::ptr::null_mut(),
            pages: Vec::new(),
        }
    }

    /// Allocate a slot from this size class
    #[inline]
    unsafe fn alloc(&mut self) -> *mut u8 {
        // 1. Try free list (recycled slots)
        if !self.free_head.is_null() {
            let ptr = self.free_head;
            self.free_head = *(ptr as *const *mut u8);
            return ptr;
        }

        // 2. Try bump allocation from current page
        if self.cursor < self.cursor_end {
            let ptr = self.cursor;
            self.cursor = self.cursor.add(self.slot_size);
            if let Some(last) = self.pages.last_mut() {
                last.allocated_up_to += self.slot_size;
            }
            return ptr;
        }

        // 3. Allocate new page
        self.alloc_new_page()
    }

    /// Allocate a new page and return the first slot
    #[cold]
    unsafe fn alloc_new_page(&mut self) -> *mut u8 {
        let layout =
            Layout::from_size_align(PAGE_SIZE, std::mem::align_of::<Obj>()).expect("slab: layout");
        let page = alloc_zeroed(layout);
        if page.is_null() {
            panic!("Out of memory (slab page allocation)");
        }

        let usable_bytes = self.slots_per_page * self.slot_size;
        self.pages.push(PageInfo {
            ptr: page,
            allocated_up_to: self.slot_size, // first slot about to be used
        });

        // Set cursor to second slot
        self.cursor = page.add(self.slot_size);
        self.cursor_end = page.add(usable_bytes);

        // Return first slot
        page
    }

    /// Free all pages (called during shutdown)
    unsafe fn free_all_pages(&mut self) {
        let layout =
            Layout::from_size_align(PAGE_SIZE, std::mem::align_of::<Obj>()).expect("slab: layout");
        for page_info in &self.pages {
            dealloc(page_info.ptr, layout);
        }
        self.pages.clear();
        self.cursor = std::ptr::null_mut();
        self.cursor_end = std::ptr::null_mut();
        self.free_head = std::ptr::null_mut();
    }
}

/// The slab allocator manages multiple size classes
pub struct SlabAllocator {
    classes: [SizeClass; NUM_SIZE_CLASSES],
}

impl SlabAllocator {
    const fn new() -> Self {
        Self {
            classes: [
                SizeClass::new(SIZE_CLASSES[0]),
                SizeClass::new(SIZE_CLASSES[1]),
                SizeClass::new(SIZE_CLASSES[2]),
                SizeClass::new(SIZE_CLASSES[3]),
            ],
        }
    }

    /// Allocate a slot for the given object size
    ///
    /// # Safety
    /// Must be called from a single thread.
    #[inline]
    pub unsafe fn alloc(&mut self, size: usize) -> *mut u8 {
        let class_idx = size_to_class_index(size);
        self.classes[class_idx].alloc()
    }

    /// Sweep all slab pages: finalize unmarked objects, rebuild free lists.
    /// Returns the number of bytes freed.
    ///
    /// ## Re-entrancy safety
    ///
    /// `finalize_object` may invoke user `__del__` methods which can call
    /// `gc_alloc → slab.alloc`. To prevent double-finalization (where a freshly
    /// allocated slot at a higher offset gets re-visited by the same sweep
    /// loop), this implementation uses a three-phase strategy:
    ///
    /// 1. **Mark phase**: walk all slots and collect `(obj_ptr, type_tag)`
    ///    pairs for unmarked slots into a side `to_finalize` vector. Free
    ///    slots get linked into a `pre_existing_free` list (in-band, through
    ///    the first 8 bytes of each free slot). For reachable slots, clear
    ///    the GC mark.
    /// 2. **Finalize phase**: iterate the `to_finalize` vector, dispatching
    ///    each finalizer by the saved tag. The tag is read once before any
    ///    free-list link write, so it survives even though the slot's first
    ///    8 bytes are overwritten in phase 3. During phase 2 the slab
    ///    `free_head` is null, so any reentrant `slab.alloc` falls through
    ///    to bump allocation above the snapshot sweep limit.
    /// 3. **Free phase**: zero each finalized slot's header size and link
    ///    every freed slot (both pre-existing and newly finalized) into the
    ///    fresh `free_head` via the slot's first 8 bytes.
    ///
    /// Index-based page iteration (rather than `iter_mut`) avoids aliasing UB
    /// when `__del__` reentrantly allocates a new page (which would push to
    /// `class.pages` and invalidate any outstanding iterator).
    ///
    /// # Safety
    /// Must be called during GC sweep phase.
    pub unsafe fn sweep(&mut self) -> usize {
        let mut bytes_freed = 0;

        for class_idx in 0..NUM_SIZE_CLASSES {
            bytes_freed += self.sweep_class(class_idx);
        }

        bytes_freed
    }

    /// Sweep a single size class. See [`Self::sweep`] for the re-entrancy
    /// strategy.
    #[inline]
    unsafe fn sweep_class(&mut self, class_idx: usize) -> usize {
        let slot_size = self.classes[class_idx].slot_size;
        let slots_per_page = self.classes[class_idx].slots_per_page;

        // Reset free list — it will be rebuilt in the free phase.
        self.classes[class_idx].free_head = std::ptr::null_mut();

        // Snapshot page geometry. Reentrant allocations from `__del__` may
        // push new pages, but we only sweep snapshot range.
        let num_pages = self.classes[class_idx].pages.len();
        let cursor = self.classes[class_idx].cursor;

        // Pre-existing free slots stay linked in-band through their first
        // 8 bytes (same layout as `free_head`). For to-finalize slots we
        // snapshot `(obj_ptr, type_tag)` into a side vector because the
        // free-list link in phase 3 overwrites the slot's first 8 bytes,
        // including the `type_tag` byte at offset 0 — finalizer dispatch
        // can no longer trust the header after that point.
        let mut to_finalize: Vec<(*mut Obj, TypeTagKind)> = Vec::new();
        let mut pre_existing_free_head: *mut u8 = std::ptr::null_mut();
        let mut bytes_freed = 0usize;

        // PHASE 1: scan slots; record finalize targets, link pre-existing frees.
        for page_idx in 0..num_pages {
            let page_ptr = self.classes[class_idx].pages[page_idx].ptr;
            let allocated_up_to = self.classes[class_idx].pages[page_idx].allocated_up_to;

            let sweep_limit = if page_idx == num_pages - 1 {
                // Current (last) page may have a cursor higher than
                // allocated_up_to under gc_stress_test (collection between
                // cursor bump and allocated_up_to update).
                let cursor_offset =
                    if cursor >= page_ptr && cursor <= page_ptr.add(slots_per_page * slot_size) {
                        cursor.offset_from(page_ptr) as usize
                    } else {
                        allocated_up_to
                    };
                cursor_offset.max(allocated_up_to)
            } else {
                allocated_up_to
            };

            // Persist the limit so future sweeps see the same range.
            self.classes[class_idx].pages[page_idx].allocated_up_to = sweep_limit;

            let mut offset = 0;
            while offset < sweep_limit {
                let obj_ptr = page_ptr.add(offset) as *mut Obj;
                let header = &mut (*obj_ptr).header;

                if header.size == 0 {
                    // Pre-existing free slot — link into pre_existing list.
                    *(obj_ptr as *mut *mut u8) = pre_existing_free_head;
                    pre_existing_free_head = obj_ptr as *mut u8;
                } else if !(*obj_ptr).is_marked() {
                    // Unmarked live object — capture tag while the header
                    // is still intact, defer finalize.
                    bytes_freed += header.size;
                    to_finalize.push((obj_ptr, header.type_tag));
                } else {
                    // Reachable — clear mark for next cycle.
                    (*obj_ptr).set_marked(false);
                }

                offset += slot_size;
            }
        }

        // PHASE 2: finalize. No mut borrow on `class` is held across the call;
        // `__del__` may reentrantly call `slab().alloc(...)`. Reentrant allocs
        // see `free_head == null` and bump-allocate above the snapshot range.
        for &(obj_ptr, tag) in &to_finalize {
            finalize_object_by_tag(obj_ptr, tag);
        }

        // PHASE 3: zero sizes and rebuild free list. Walk the pre-existing
        // in-band list, then push the freshly finalized slots.
        let mut new_free_head = std::ptr::null_mut::<u8>();

        let mut p = pre_existing_free_head;
        while !p.is_null() {
            let next = *(p as *const *mut u8);
            *(p as *mut *mut u8) = new_free_head;
            new_free_head = p;
            p = next;
        }

        for &(obj_ptr, _) in &to_finalize {
            (*obj_ptr).header.size = 0;
            *(obj_ptr as *mut *mut u8) = new_free_head;
            new_free_head = obj_ptr as *mut u8;
        }

        self.classes[class_idx].free_head = new_free_head;

        bytes_freed
    }

    /// Free all pages (called during shutdown)
    pub unsafe fn shutdown(&mut self) {
        for class in &mut self.classes {
            class.free_all_pages();
        }
    }
}

/// Map an object size to the appropriate size class index
#[inline(always)]
fn size_to_class_index(size: usize) -> usize {
    if size <= 24 {
        0
    } else if size <= 32 {
        1
    } else if size <= 48 {
        2
    } else {
        // size <= 64, enforced by caller
        3
    }
}

/// Check if a size can be served by the slab allocator
#[inline(always)]
pub fn is_slab_size(size: usize) -> bool {
    size <= SLAB_MAX_SIZE
}

/// Finalize an object before freeing (release auxiliary allocations).
///
/// `tag` is passed explicitly because slab sweep may overwrite the slot's
/// header (first 8 bytes) before this is called; the caller is responsible
/// for snapshotting the tag while the header is still intact.
unsafe fn finalize_object_by_tag(obj_ptr: *mut Obj, tag: TypeTagKind) {
    match tag {
        TypeTagKind::File => {
            crate::file::file_finalize(obj_ptr);
        }
        TypeTagKind::List => {
            crate::list::list_finalize(obj_ptr);
        }
        TypeTagKind::Dict | TypeTagKind::Counter => {
            crate::dict::dict_finalize(obj_ptr);
        }
        TypeTagKind::DefaultDict => {
            // Factory tag is packed into entries_capacity — no external registry to clean up.
            crate::dict::dict_finalize(obj_ptr);
        }
        TypeTagKind::Deque => {
            crate::deque::deque_finalize(obj_ptr);
        }
        TypeTagKind::Set => {
            crate::set::set_finalize(obj_ptr);
        }
        TypeTagKind::Generator => {
            // §F.7b: no per-slot tag array to free; GeneratorObj is fully GC-managed.
        }
        TypeTagKind::StringBuilder => {
            crate::string::string_builder_finalize(obj_ptr);
        }
        TypeTagKind::StringIO => {
            crate::stringio::stringio_finalize(obj_ptr);
        }
        TypeTagKind::BytesIO => {
            crate::stringio::bytesio_finalize(obj_ptr);
        }
        TypeTagKind::BigInt => {
            // Drop the owned `BigInt` (its digit `Vec`) before the slot is freed
            // (precedent: List/Dict own Rust allocations finalized here).
            std::ptr::drop_in_place(&mut (*(obj_ptr as *mut crate::object::BigIntObj)).value);
        }
        TypeTagKind::Instance => {
            // Call __del__ if registered for this class.
            // Use catch_unwind to prevent panics from corrupting the slab free list.
            let instance = obj_ptr as *mut crate::object::InstanceObj;
            let class_id = (*instance).class_id;
            let del_fn = crate::vtable::get_del_func(class_id);
            if !del_fn.is_null() {
                let del_fn: extern "C" fn(i64) -> i64 = std::mem::transmute(del_fn);
                let obj_i64 = obj_ptr as i64;
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    del_fn(obj_i64);
                }));
            }
        }
        _ => {}
    }
}

/// Public finalize function for use by `gc.rs` sweep of large objects.
/// Reads `type_tag` from the header (caller guarantees the header is intact).
///
/// # Safety
/// `obj_ptr` must be a valid pointer to a GC-managed object whose header has
/// not been overwritten by a free-list link or similar.
pub unsafe fn finalize_object_pub(obj_ptr: *mut Obj) {
    finalize_object_by_tag(obj_ptr, (*obj_ptr).type_tag());
}

// ============================================================================
// Global slab allocator instance
// ============================================================================

struct SlabHolder(UnsafeCell<SlabAllocator>);
unsafe impl Sync for SlabHolder {}

static SLAB: SlabHolder = SlabHolder(UnsafeCell::new(SlabAllocator::new()));

/// Get a mutable reference to the global slab allocator
///
/// # Safety
/// Must only be called from a single thread.
#[inline(always)]
pub unsafe fn slab() -> &'static mut SlabAllocator {
    &mut *SLAB.0.get()
}
