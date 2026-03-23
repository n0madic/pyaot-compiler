//! Precise garbage collector with shadow stack
//!
//! This implements a mark-sweep GC with explicit root tracking via shadow stack.
//!
//! ## GC Stress Testing
//!
//! To enable aggressive GC stress testing (collect on every allocation):
//! ```bash
//! RUSTFLAGS="--cfg gc_stress_test" cargo build --release
//! RUSTFLAGS="--cfg gc_stress_test" cargo test --workspace
//! ```
//!
//! This mode is useful for exposing GC bugs that only manifest under memory pressure.
//!
//! ## Alignment Safety
//!
//! All heap objects are allocated and deallocated using `align_of::<Obj>()`.
//! This is safe because all object types use `#[repr(C)]` and have `ObjHeader`
//! as their first field. The compile-time assertions below verify this invariant.

use crate::cell::{cell_get_ptr_for_gc, CellObj};
use crate::class_attrs::get_class_attr_pointers;
use crate::globals::get_global_pointers;
use crate::object::{
    BoolObj, BytesObj, ChainIterObj, CompletedProcessObj, DictObj, FileObj, FilterIterObj,
    FloatObj, GeneratorObj, ISliceIterObj, InstanceObj, IntObj, IteratorKind, IteratorObj, ListObj,
    MapIterObj, MatchObj, Obj, ObjHeader, SetObj, StrObj, TupleObj, TypeTagKind, Zip3IterObj,
    ZipIterObj, ZipNIterObj, TOMBSTONE,
};
// Note: BytesObj has no object children (like StrObj), so no special handling needed in mark_object
use std::alloc::{alloc, dealloc, Layout};
use std::ptr;
use std::sync::{Mutex, Once};

// ============================================================================
// Compile-time alignment verification
// ============================================================================
//
// All heap objects are allocated with `align_of::<Obj>()`. This is only safe
// if all object types have the same alignment. These static assertions verify
// this invariant at compile time. If you add a new object type with higher
// alignment requirements (e.g., SIMD types), you must update gc_alloc/sweep
// to track per-object alignment.

/// Compile-time assertion helper (triggers compile error if false)
macro_rules! const_assert_align {
    ($obj_ty:ty) => {
        const _: () = {
            if std::mem::align_of::<$obj_ty>() != std::mem::align_of::<Obj>() {
                panic!(concat!(
                    "Alignment mismatch: ",
                    stringify!($obj_ty),
                    " has different alignment than Obj. ",
                    "This would cause undefined behavior in gc_alloc/dealloc."
                ));
            }
        };
    };
}

// Verify alignment for all object types
const_assert_align!(IntObj);
const_assert_align!(FloatObj);
const_assert_align!(BoolObj);
const_assert_align!(StrObj);
const_assert_align!(BytesObj);
const_assert_align!(ListObj);
const_assert_align!(TupleObj);
const_assert_align!(DictObj);
const_assert_align!(SetObj);
const_assert_align!(InstanceObj);
const_assert_align!(IteratorObj);
const_assert_align!(GeneratorObj);
const_assert_align!(FileObj);
const_assert_align!(MatchObj);
const_assert_align!(CompletedProcessObj);
const_assert_align!(CellObj);
const_assert_align!(MapIterObj);
const_assert_align!(FilterIterObj);
const_assert_align!(ZipIterObj);
const_assert_align!(ChainIterObj);
const_assert_align!(ISliceIterObj);
const_assert_align!(crate::hashlib::HashObj);
const_assert_align!(crate::stringio::StringIOObj);
const_assert_align!(crate::stringio::BytesIOObj);

/// Shadow stack frame for precise GC
#[repr(C)]
pub struct ShadowFrame {
    pub prev: *mut ShadowFrame,
    pub nroots: usize,
    pub roots: *mut *mut Obj,
}

/// Global GC state
struct GcState {
    /// Top of shadow stack
    stack_top: *mut ShadowFrame,
    /// All allocated objects
    objects: Vec<*mut Obj>,
    /// Total allocated bytes
    total_allocated: usize,
    /// Threshold for triggering collection
    threshold: usize,
    /// Shadow stack depth (for leak detection)
    stack_depth: usize,
}

static mut GC_STATE: Option<Mutex<GcState>> = None;
static GC_INIT: Once = Once::new();

/// Initialize the GC (idempotent - safe to call multiple times)
pub fn init() {
    GC_INIT.call_once(|| {
        unsafe {
            GC_STATE = Some(Mutex::new(GcState {
                stack_top: ptr::null_mut(),
                objects: Vec::new(),
                total_allocated: 0,
                threshold: 1024 * 1024, // 1MB initial threshold
                stack_depth: 0,
            }));
        }
    });
}

/// Shutdown the GC (free all objects)
pub fn shutdown() {
    unsafe {
        if let Some(ref gc) = GC_STATE {
            let mut state = gc
                .lock()
                .expect("GC_STATE mutex poisoned - another thread panicked");
            for obj_ptr in &state.objects {
                let obj = &**obj_ptr;
                let layout =
                    Layout::from_size_align_unchecked(obj.header.size, std::mem::align_of::<Obj>());
                dealloc(*obj_ptr as *mut u8, layout);
            }
            state.objects.clear();
        }
    }
}

fn with_gc_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut GcState) -> R,
{
    unsafe {
        let gc = (*std::ptr::addr_of!(GC_STATE))
            .as_ref()
            .expect("GC not initialized");
        let mut state = gc
            .lock()
            .expect("GC_STATE mutex poisoned - another thread panicked");
        f(&mut state)
    }
}

/// Push a shadow frame onto the stack
///
/// # Safety
/// `frame` must be a valid pointer to a `ShadowFrame` that will remain valid
/// until `gc_pop` is called.
#[no_mangle]
pub unsafe extern "C" fn gc_push(frame: *mut ShadowFrame) {
    if frame.is_null() {
        return;
    }
    with_gc_state(|state| {
        state.stack_depth += 1;

        // Detect shadow stack leaks: if depth exceeds 10000, likely a leak from exception paths
        if state.stack_depth > 10000 {
            panic!(
                "Shadow stack depth limit exceeded ({}). This likely indicates a memory leak \
                from exception paths not properly calling gc_pop()",
                state.stack_depth
            );
        }

        (*frame).prev = state.stack_top;
        state.stack_top = frame;
    });
}

/// Pop a shadow frame from the stack
#[no_mangle]
pub extern "C" fn gc_pop() {
    with_gc_state(|state| unsafe {
        if !state.stack_top.is_null() {
            state.stack_top = (*state.stack_top).prev;
            state.stack_depth = state.stack_depth.saturating_sub(1);
        } else {
            // Double-pop detected! This is a bug in exception handling or codegen.
            // In debug mode, panic immediately to catch the bug.
            // In release mode, log to stderr to help diagnose the issue without crashing.
            #[cfg(debug_assertions)]
            panic!("gc_pop() called with empty shadow stack - double-pop detected!");
            #[cfg(not(debug_assertions))]
            eprintln!(
                "WARNING: gc_pop() called with empty shadow stack. This indicates a bug in \
                 exception handling or mismatched push/pop calls."
            );
        }
    });
}

/// Get current shadow stack depth (for debugging/testing)
#[no_mangle]
pub extern "C" fn gc_get_stack_depth() -> usize {
    with_gc_state(|state| state.stack_depth)
}

/// Allocate an object
#[no_mangle]
pub extern "C" fn gc_alloc(size: usize, type_tag: u8) -> *mut Obj {
    // Validate type tag BEFORE allocation to avoid memory leak on invalid input
    let validated_type_tag = TypeTagKind::from_tag(type_tag)
        .unwrap_or_else(|| panic!("gc_alloc: invalid type tag {}", type_tag));

    with_gc_state(|state| {
        // GC stress testing mode: collect on every allocation
        // Enable with: RUSTFLAGS="--cfg gc_stress_test" cargo build
        #[cfg(gc_stress_test)]
        {
            collect_impl(state);
        }

        // Check if we should collect (normal mode)
        #[cfg(not(gc_stress_test))]
        {
            if state.total_allocated > state.threshold {
                collect_impl(state);
            }
        }

        unsafe {
            let layout = Layout::from_size_align_unchecked(size, std::mem::align_of::<Obj>());
            let ptr = alloc(layout) as *mut Obj;
            if ptr.is_null() {
                panic!("Out of memory");
            }

            // Initialize header with pre-validated type tag
            (*ptr).header = ObjHeader {
                type_tag: validated_type_tag,
                marked: false,
                size,
            };

            // Verify size is large enough to hold the object header
            debug_assert!(
                size >= std::mem::size_of::<ObjHeader>(),
                "gc_alloc: size {} is smaller than ObjHeader ({})",
                size,
                std::mem::size_of::<ObjHeader>()
            );

            // Zero the rest
            ptr::write_bytes(
                (ptr as *mut u8).add(std::mem::size_of::<ObjHeader>()),
                0,
                size - std::mem::size_of::<ObjHeader>(),
            );

            state.objects.push(ptr);
            state.total_allocated += size;

            ptr
        }
    })
}

/// Trigger garbage collection
#[no_mangle]
pub extern "C" fn gc_collect() {
    with_gc_state(|state| {
        collect_impl(state);
    });
}

/// Get the current stack top (for exception handling)
/// Returns the current top of the shadow stack
pub fn get_stack_top() -> *mut ShadowFrame {
    with_gc_state(|state| state.stack_top)
}

/// Unwind the shadow stack to the given frame
/// Used by exception handling to restore GC state when unwinding
///
/// This counts how many frames are being popped and updates stack_depth accordingly.
pub fn unwind_to(target: *mut ShadowFrame) {
    with_gc_state(|state| unsafe {
        // Count how many frames we're unwinding
        let mut frames_popped = 0;
        let mut current = state.stack_top;

        // Walk the stack until we reach the target or null
        while !current.is_null() && current != target {
            frames_popped += 1;
            current = (*current).prev;
        }

        // Update stack top and depth
        state.stack_top = target;
        state.stack_depth = state.stack_depth.saturating_sub(frames_popped);

        #[cfg(debug_assertions)]
        {
            // In debug mode, verify we actually found the target
            if current.is_null() && !target.is_null() {
                panic!(
                    "unwind_to: target frame not found in shadow stack. \
                    This indicates stack corruption or mismatched push/pop."
                );
            }
        }
    });
}

/// Internal collection implementation
fn collect_impl(state: &mut GcState) {
    // Mark phase
    mark_roots(state);

    // Sweep phase
    sweep(state);

    // Adjust threshold
    state.threshold = state.total_allocated.saturating_mul(2);
    if state.threshold < 1024 * 1024 {
        state.threshold = 1024 * 1024;
    }
}

/// Mark all reachable objects
fn mark_roots(state: &mut GcState) {
    // Mark objects reachable from shadow stack
    unsafe {
        let mut frame = state.stack_top;
        while !frame.is_null() {
            let nroots = (*frame).nroots;
            let roots = (*frame).roots;

            // Validate roots pointer before accessing
            if roots.is_null() && nroots > 0 {
                eprintln!(
                    "FATAL: Shadow stack corruption - null roots pointer with nroots={}",
                    nroots
                );
                std::process::abort();
            }

            for i in 0..nroots {
                let root_ptr = *roots.add(i);
                if !root_ptr.is_null() {
                    mark_object(root_ptr);
                }
            }
            frame = (*frame).prev;
        }
    }

    // Mark objects stored in global variables
    mark_global_pointers();

    // Mark objects stored in class attributes
    mark_class_attr_pointers();

    // Mark small integer pool and boolean singleton pool objects.
    // Pool objects are registered in state.objects like any other allocation, but
    // they are not reachable via the shadow stack or globals.  sweep() clears
    // the mark bit on every surviving object, so we must re-mark them here on
    // every collection cycle to prevent them from being freed.
    crate::boxing::mark_pools();
}

/// Mark all heap objects stored in global variables
fn mark_global_pointers() {
    for ptr in get_global_pointers() {
        if !ptr.is_null() {
            mark_object(ptr);
        }
    }
}

/// Mark all heap objects stored in class attributes
fn mark_class_attr_pointers() {
    for ptr in get_class_attr_pointers() {
        if !ptr.is_null() {
            mark_object(ptr);
        }
    }
}

/// Mark an object and its children
fn mark_object(obj: *mut Obj) {
    unsafe {
        // Skip null pointers and obviously-invalid addresses.
        // Raw int/bool values (e.g., 0x1 for True) can appear in ELEM_HEAP_OBJ
        // containers due to elem_tag mismatches in *args tuples and closure captures.
        // Also validates alignment — Obj requires 8-byte alignment, so code pointers
        // (4-byte aligned) and small integers are safely skipped.
        if obj.is_null()
            || (obj as usize) < 0x1000
            || !(obj as usize).is_multiple_of(std::mem::align_of::<Obj>())
        {
            return;
        }
        if (*obj).is_marked() {
            return;
        }

        (*obj).set_marked(true);

        // Mark children based on type
        match (*obj).type_tag() {
            TypeTagKind::List => {
                let list = obj as *mut ListObj;
                // Only traverse elements if they are heap objects
                // Raw int/bool elements are NOT pointers and must NOT be dereferenced
                if (*list).elem_tag == 0 {
                    // ELEM_HEAP_OBJ
                    let len = (*list).len;
                    let data = (*list).data;
                    // Validate data pointer before accessing
                    if data.is_null() && len > 0 {
                        eprintln!(
                            "FATAL: List heap corruption - null data pointer with len={}",
                            len
                        );
                        std::process::abort();
                    }
                    for i in 0..len {
                        let elem = *data.add(i);
                        if !elem.is_null() {
                            mark_object(elem);
                        }
                    }
                }
            }
            TypeTagKind::Tuple => {
                let tuple = obj as *mut TupleObj;
                let mask = (*tuple).heap_field_mask;
                // Skip if no fields need tracing (ELEM_RAW_INT or mask == 0)
                if mask != 0 {
                    let len = (*tuple).len;
                    let data = (*tuple).data.as_ptr();
                    for i in 0..len {
                        // Only trace fields marked as heap pointers in the bitmask.
                        // Use checked_shl to prevent shift overflow when i >= 64.
                        if mask & 1u64.checked_shl(i as u32).unwrap_or(0) != 0 {
                            let elem = *data.add(i);
                            if !elem.is_null() {
                                mark_object(elem);
                            }
                        }
                    }
                }
            }
            TypeTagKind::Dict => {
                let dict = obj as *mut DictObj;
                let entries = (*dict).entries;
                let entries_len = (*dict).entries_len;
                // Validate entries pointer before accessing
                if entries.is_null() && entries_len > 0 {
                    eprintln!(
                        "FATAL: Dict heap corruption - null entries pointer with entries_len={}",
                        entries_len
                    );
                    std::process::abort();
                }
                for i in 0..entries_len {
                    let entry = entries.add(i);
                    let key = (*entry).key;
                    if !key.is_null() {
                        mark_object(key);
                        let value = (*entry).value;
                        if !value.is_null() {
                            mark_object(value);
                        }
                    }
                }
            }
            TypeTagKind::Set => {
                let set = obj as *mut SetObj;
                let capacity = (*set).capacity;
                let entries = (*set).entries;
                // Validate entries pointer before accessing
                if entries.is_null() && capacity > 0 {
                    eprintln!(
                        "FATAL: Set heap corruption - null entries pointer with capacity={}",
                        capacity
                    );
                    std::process::abort();
                }
                for i in 0..capacity {
                    let entry = entries.add(i);
                    let elem = (*entry).elem;
                    if !elem.is_null() && elem != TOMBSTONE {
                        mark_object(elem);
                    }
                }
            }
            TypeTagKind::Instance => {
                let instance = obj as *mut InstanceObj;
                let field_count = (*instance).field_count;
                let fields = (*instance).fields.as_ptr();
                // Only mark fields that are heap objects (pointers), not raw int/float/bool values.
                // The heap_field_mask tells us which fields are heap types.
                let mask = crate::vtable::get_class_heap_field_mask((*instance).class_id);
                for i in 0..field_count {
                    // Use checked_shl to prevent shift overflow when i >= 64.
                    if mask & 1u64.checked_shl(i as u32).unwrap_or(0) != 0 {
                        let field = *fields.add(i);
                        if !field.is_null() {
                            mark_object(field);
                        }
                    }
                }
            }
            TypeTagKind::Iterator => {
                let iterator = obj as *mut IteratorObj;
                let kind = IteratorKind::try_from((*iterator).kind);

                match kind {
                    Ok(IteratorKind::Map) => {
                        let map_iter = obj as *mut MapIterObj;
                        if !(*map_iter).inner_iter.is_null() {
                            mark_object((*map_iter).inner_iter);
                        }
                        if !(*map_iter).captures.is_null() {
                            mark_object((*map_iter).captures);
                        }
                    }
                    Ok(IteratorKind::Filter) => {
                        let filter_iter = obj as *mut FilterIterObj;
                        if !(*filter_iter).inner_iter.is_null() {
                            mark_object((*filter_iter).inner_iter);
                        }
                        if !(*filter_iter).captures.is_null() {
                            mark_object((*filter_iter).captures);
                        }
                    }
                    Ok(IteratorKind::Zip) => {
                        let zip_iter = obj as *mut ZipIterObj;
                        if !(*zip_iter).iter1.is_null() {
                            mark_object((*zip_iter).iter1);
                        }
                        if !(*zip_iter).iter2.is_null() {
                            mark_object((*zip_iter).iter2);
                        }
                    }
                    Ok(IteratorKind::Chain) => {
                        let chain_iter = obj as *mut ChainIterObj;
                        if !(*chain_iter).iters.is_null() {
                            mark_object((*chain_iter).iters);
                        }
                    }
                    Ok(IteratorKind::ISlice) => {
                        let islice_iter = obj as *mut ISliceIterObj;
                        if !(*islice_iter).inner_iter.is_null() {
                            mark_object((*islice_iter).inner_iter);
                        }
                    }
                    Ok(IteratorKind::Zip3) => {
                        let zip3_iter = obj as *mut Zip3IterObj;
                        if !(*zip3_iter).iter1.is_null() {
                            mark_object((*zip3_iter).iter1);
                        }
                        if !(*zip3_iter).iter2.is_null() {
                            mark_object((*zip3_iter).iter2);
                        }
                        if !(*zip3_iter).iter3.is_null() {
                            mark_object((*zip3_iter).iter3);
                        }
                    }
                    Ok(IteratorKind::ZipN) => {
                        let zipn_iter = obj as *mut ZipNIterObj;
                        // iters is a ListObj containing the iterators
                        if !(*zipn_iter).iters.is_null() {
                            mark_object((*zipn_iter).iters);
                        }
                    }
                    _ => {
                        // Standard iterators (List, Tuple, Dict, String, Range, Set, Bytes, Enumerate)
                        let source = (*iterator).source;
                        if !source.is_null() {
                            mark_object(source);
                        }
                    }
                }
            }
            TypeTagKind::Cell => {
                let cell = obj as *mut CellObj;
                if let Some(ptr) = cell_get_ptr_for_gc(cell) {
                    mark_object(ptr);
                }
            }
            TypeTagKind::Generator => {
                // Generators store local variables with precise type information
                // Use type_tags array for precise GC tracking
                use crate::object::LOCAL_TYPE_PTR;

                let gen = obj as *mut GeneratorObj;
                let num_locals = (*gen).num_locals;
                let locals = (*gen).locals.as_ptr();
                let type_tags = (*gen).type_tags;

                // Only trace locals that are marked as heap pointers
                if !type_tags.is_null() {
                    for i in 0..num_locals as usize {
                        let tag = *type_tags.add(i);
                        if tag == LOCAL_TYPE_PTR {
                            // This local is a heap pointer, trace it
                            let ptr = *locals.add(i) as *mut Obj;
                            if !ptr.is_null() {
                                mark_object(ptr);
                            }
                        }
                        // Other tags (LOCAL_TYPE_RAW_INT, LOCAL_TYPE_RAW_FLOAT, LOCAL_TYPE_RAW_BOOL)
                        // are raw values, not pointers - skip them
                    }
                }
            }
            TypeTagKind::File => {
                // File objects have a name field pointing to a StrObj
                let file = obj as *mut FileObj;
                let name = (*file).name;
                if !name.is_null() {
                    mark_object(name);
                }
            }
            TypeTagKind::Match => {
                // Match objects have groups (tuple) and original (string) fields
                let match_obj = obj as *mut MatchObj;
                let groups = (*match_obj).groups;
                if !groups.is_null() {
                    mark_object(groups);
                }
                let original = (*match_obj).original;
                if !original.is_null() {
                    mark_object(original);
                }
            }
            TypeTagKind::CompletedProcess => {
                // CompletedProcess objects have args, stdout, and stderr fields
                let cp_obj = obj as *mut CompletedProcessObj;
                let args = (*cp_obj).args;
                if !args.is_null() {
                    mark_object(args);
                }
                let stdout = (*cp_obj).stdout;
                if !stdout.is_null() {
                    mark_object(stdout);
                }
                let stderr = (*cp_obj).stderr;
                if !stderr.is_null() {
                    mark_object(stderr);
                }
            }
            TypeTagKind::ParseResult => {
                // ParseResult objects have scheme, netloc, path, params, query, fragment fields
                let pr_obj = obj as *mut crate::object::ParseResultObj;
                if !(*pr_obj).scheme.is_null() {
                    mark_object((*pr_obj).scheme);
                }
                if !(*pr_obj).netloc.is_null() {
                    mark_object((*pr_obj).netloc);
                }
                if !(*pr_obj).path.is_null() {
                    mark_object((*pr_obj).path);
                }
                if !(*pr_obj).params.is_null() {
                    mark_object((*pr_obj).params);
                }
                if !(*pr_obj).query.is_null() {
                    mark_object((*pr_obj).query);
                }
                if !(*pr_obj).fragment.is_null() {
                    mark_object((*pr_obj).fragment);
                }
            }
            TypeTagKind::HttpResponse => {
                // HttpResponse objects have url, headers, and body fields
                let hr_obj = obj as *mut crate::object::HttpResponseObj;
                if !(*hr_obj).url.is_null() {
                    mark_object((*hr_obj).url);
                }
                if !(*hr_obj).headers.is_null() {
                    mark_object((*hr_obj).headers);
                }
                if !(*hr_obj).body.is_null() {
                    mark_object((*hr_obj).body);
                }
            }
            // Hash, StringIO, BytesIO have no object children to trace
            TypeTagKind::Hash | TypeTagKind::StringIO | TypeTagKind::BytesIO => {}
            // Other types don't have object children
            _ => {}
        }
    }
}

/// Sweep unmarked objects
fn sweep(state: &mut GcState) {
    // Prune string pool BEFORE clearing marks and freeing objects
    // This allows us to check which strings are still reachable
    unsafe {
        crate::string::prune_string_pool();
    }

    state.objects.retain(|obj_ptr| unsafe {
        let obj = &mut **obj_ptr;
        if !obj.is_marked() {
            // Finalize objects before freeing to release auxiliary allocations
            match obj.type_tag() {
                TypeTagKind::File => {
                    // Close unclosed files
                    crate::file::file_finalize(*obj_ptr);
                }
                TypeTagKind::List => {
                    // Free list data array
                    crate::list::list_finalize(*obj_ptr);
                }
                TypeTagKind::Dict => {
                    // Free dict entries array
                    crate::dict::dict_finalize(*obj_ptr);
                }
                TypeTagKind::Set => {
                    // Free set entries array
                    crate::set::set_finalize(*obj_ptr);
                }
                TypeTagKind::Generator => {
                    // Free generator type_tags array
                    crate::generator::finalize_generator(*obj_ptr);
                }
                TypeTagKind::StringBuilder => {
                    // Free StringBuilder buffer
                    crate::string::string_builder_finalize(*obj_ptr);
                }
                TypeTagKind::StringIO => {
                    crate::stringio::stringio_finalize(*obj_ptr);
                }
                TypeTagKind::BytesIO => {
                    crate::stringio::bytesio_finalize(*obj_ptr);
                }
                _ => {}
            }
            // Free this object
            let layout =
                Layout::from_size_align_unchecked(obj.header.size, std::mem::align_of::<Obj>());
            state.total_allocated = state.total_allocated.saturating_sub(obj.header.size);
            dealloc(*obj_ptr as *mut u8, layout);
            false
        } else {
            // Unmark for next collection
            obj.set_marked(false);
            true
        }
    });
}
