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
use std::sync::atomic::{AtomicPtr, Ordering};

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
#[cfg(feature = "stdlib-crypto")]
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

// Compile-time assertions: ShadowFrame layout must match codegen constants
const _: () = assert!(
    std::mem::size_of::<ShadowFrame>() == pyaot_core_defs::layout::SHADOW_FRAME_SIZE as usize,
    "ShadowFrame size does not match layout::SHADOW_FRAME_SIZE"
);
const _: () = assert!(
    std::mem::offset_of!(ShadowFrame, nroots)
        == pyaot_core_defs::layout::SHADOW_FRAME_NROOTS_OFFSET as usize,
    "ShadowFrame nroots offset does not match layout::SHADOW_FRAME_NROOTS_OFFSET"
);
const _: () = assert!(
    std::mem::offset_of!(ShadowFrame, roots)
        == pyaot_core_defs::layout::SHADOW_FRAME_ROOTS_OFFSET as usize,
    "ShadowFrame roots offset does not match layout::SHADOW_FRAME_ROOTS_OFFSET"
);

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
    /// Guard against re-entrant collection (e.g., __del__ allocating during sweep)
    collecting: bool,
}

// Safety: GcState is accessed from a single thread (AOT-compiled Python is single-threaded).
// The AtomicPtr provides the necessary Sync for the static, but actual access is unsynchronized.
unsafe impl Send for GcState {}
unsafe impl Sync for GcState {}

static GC_STATE_PTR: AtomicPtr<GcState> = AtomicPtr::new(ptr::null_mut());

/// Get a mutable reference to the GC state.
///
/// # Safety
/// Must only be called after `init()` and from a single thread.
#[inline(always)]
unsafe fn gc_state() -> &'static mut GcState {
    let ptr = GC_STATE_PTR.load(Ordering::Acquire);
    debug_assert!(!ptr.is_null(), "gc_state() called before init()");
    if ptr.is_null() {
        eprintln!("FATAL: GC accessed before initialization. Call rt_init() first.");
        std::process::abort();
    }
    &mut *ptr
}

/// Initialize the GC (idempotent - safe to call multiple times)
pub fn init() {
    let state = Box::into_raw(Box::new(GcState {
        stack_top: ptr::null_mut(),
        objects: Vec::new(),
        total_allocated: 0,
        threshold: 1024 * 1024,
        stack_depth: 0,
        collecting: false,
    }));
    // Use compare_exchange to prevent double-init race (TOCTOU)
    if GC_STATE_PTR
        .compare_exchange(ptr::null_mut(), state, Ordering::Release, Ordering::Relaxed)
        .is_err()
    {
        // Another init already completed — drop our allocation
        unsafe {
            drop(Box::from_raw(state));
        }
    }
}

/// Shutdown the GC (free all objects)
pub fn shutdown() {
    let state_ptr = GC_STATE_PTR.load(Ordering::Acquire);
    if state_ptr.is_null() {
        return;
    }

    // Null out state pointer first to prevent post-shutdown access
    GC_STATE_PTR.store(ptr::null_mut(), Ordering::Release);

    unsafe {
        let state = &mut *state_ptr;

        // Finalize and free large objects tracked in Vec
        for obj_ptr in &state.objects {
            crate::slab::finalize_object_pub(*obj_ptr);
            let obj = &**obj_ptr;
            let layout = Layout::from_size_align(obj.header.size, std::mem::align_of::<Obj>())
                .expect("Invalid layout during GC shutdown");
            dealloc(*obj_ptr as *mut u8, layout);
        }
        state.objects.clear();

        // Free all slab pages (small objects freed in bulk)
        crate::slab::slab().shutdown();

        // Reconstitute the Box to properly drop GcState (frees Vec buffer + struct)
        drop(Box::from_raw(state_ptr));
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
    let state = gc_state();
    state.stack_depth += 1;

    // Detect shadow stack leaks: if depth exceeds 10000, likely a leak from exception paths
    #[cfg(debug_assertions)]
    if state.stack_depth > 10000 {
        panic!(
            "Shadow stack depth limit exceeded ({}). This likely indicates a memory leak \
            from exception paths not properly calling gc_pop()",
            state.stack_depth
        );
    }

    (*frame).prev = state.stack_top;
    state.stack_top = frame;
}

/// Pop a shadow frame from the stack
#[no_mangle]
pub extern "C" fn gc_pop() {
    unsafe {
        let state = gc_state();
        if !state.stack_top.is_null() {
            state.stack_top = (*state.stack_top).prev;
            state.stack_depth = state.stack_depth.saturating_sub(1);
        } else {
            // Double-pop detected! This is a bug in exception handling or codegen.
            #[cfg(debug_assertions)]
            panic!("gc_pop() called with empty shadow stack - double-pop detected!");
            #[cfg(not(debug_assertions))]
            {
                eprintln!(
                    "WARNING: gc_pop() called with empty shadow stack. This indicates a bug in \
                     exception handling or mismatched push/pop calls."
                );
                state.stack_depth = 0;
            }
        }
    }
}

/// Get current shadow stack depth (for debugging/testing)
#[no_mangle]
pub extern "C" fn gc_get_stack_depth() -> usize {
    unsafe { gc_state().stack_depth }
}

/// Allocate an object
#[no_mangle]
pub extern "C" fn gc_alloc(size: usize, type_tag: u8) -> *mut Obj {
    // Validate type tag BEFORE allocation to avoid memory leak on invalid input
    let validated_type_tag = TypeTagKind::from_tag(type_tag)
        .unwrap_or_else(|| panic!("gc_alloc: invalid type tag {}", type_tag));

    assert!(
        size >= std::mem::size_of::<ObjHeader>(),
        "gc_alloc: size {} is smaller than ObjHeader ({})",
        size,
        std::mem::size_of::<ObjHeader>()
    );

    unsafe {
        let state = gc_state();

        // Skip collection if we're already inside a collect cycle (prevents
        // re-entrant collection from __del__ finalizers that allocate).
        if !state.collecting {
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
        }

        let ptr = if crate::slab::is_slab_size(size) {
            // Small object: allocate from slab (bump pointer, no Vec push)
            crate::slab::slab().alloc(size) as *mut Obj
        } else {
            // Large object: allocate from system malloc, track in Vec
            let layout = Layout::from_size_align(size, std::mem::align_of::<Obj>())
                .expect("gc_alloc: invalid layout");
            let p = alloc(layout) as *mut Obj;
            if p.is_null() {
                panic!("Out of memory");
            }
            state.objects.push(p);
            p
        };

        // Initialize header with pre-validated type tag
        (*ptr).header = ObjHeader {
            type_tag: validated_type_tag,
            marked: false,
            size,
        };

        // Zero the rest
        ptr::write_bytes(
            (ptr as *mut u8).add(std::mem::size_of::<ObjHeader>()),
            0,
            size - std::mem::size_of::<ObjHeader>(),
        );

        state.total_allocated = state.total_allocated.saturating_add(size);

        ptr
    }
}

/// Trigger garbage collection
#[no_mangle]
pub extern "C" fn gc_collect() {
    unsafe {
        collect_impl(gc_state());
    }
}

/// Get the current stack top (for exception handling)
/// Returns the current top of the shadow stack
pub fn get_stack_top() -> *mut ShadowFrame {
    unsafe { gc_state().stack_top }
}

/// Unwind the shadow stack to the given frame
/// Used by exception handling to restore GC state when unwinding
///
/// This counts how many frames are being popped and updates stack_depth accordingly.
pub fn unwind_to(target: *mut ShadowFrame) {
    unsafe {
        let state = gc_state();
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

        // In both debug and release: abort if target not found (stack corruption)
        if current.is_null() && !target.is_null() {
            eprintln!(
                "FATAL: unwind_to: target frame not found in shadow stack. Stack corruption detected."
            );
            std::process::abort();
        }
    }
}

/// Internal collection implementation
fn collect_impl(state: &mut GcState) {
    state.collecting = true;

    // Mark phase
    mark_roots(state);

    // Sweep phase
    sweep(state);

    // Adjust threshold
    state.threshold = state.total_allocated.saturating_mul(2);
    if state.threshold < 1024 * 1024 {
        state.threshold = 1024 * 1024;
    }

    state.collecting = false;
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
                mark_object(pyaot_core_defs::Value::from_ptr(root_ptr));
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

    // Mark exception instance pointers stored in thread-local ExceptionState.
    // Exception instances survive longjmp (which unwinds the shadow stack) because
    // they are stored in Rust heap-allocated ExceptionObject, not in GC roots.
    mark_exception_pointers();

    // Mark process-wide singletons cached in `sys` module statics
    // (`sys.argv`, `sys.path`). They are not reachable through globals or
    // the shadow stack, so without explicit marking the next collection
    // would free the underlying `ListObj` and a subsequent `sys.path`
    // read would dereference a dangling pointer.
    for ptr in crate::sys::get_sys_module_roots() {
        mark_object(pyaot_core_defs::Value::from_ptr(ptr));
    }
}

/// Mark all heap objects stored in exception state (current/handling exceptions)
fn mark_exception_pointers() {
    for ptr in crate::exceptions::get_exception_pointers() {
        mark_object(pyaot_core_defs::Value::from_ptr(ptr));
    }
}

/// Mark all heap objects stored in global variables
fn mark_global_pointers() {
    for ptr in get_global_pointers() {
        mark_object(pyaot_core_defs::Value::from_ptr(ptr));
    }
}

/// Mark all heap objects stored in class attributes
fn mark_class_attr_pointers() {
    for ptr in get_class_attr_pointers() {
        mark_object(pyaot_core_defs::Value::from_ptr(ptr));
    }
}

/// Mark a `Value` and, if it's a heap pointer, recursively mark its children.
///
/// Phase 2 S2.6: the GC's canonical entrypoint is now `Value`-typed. An
/// immediate `Value` (Int / Bool / None) self-describes as non-pointer
/// via `Value::is_ptr()` and returns immediately. A pointer `Value`
/// unwraps to `*mut Obj`; the heuristic address filter below survives
/// from the pre-Value world to guard against garbage bit patterns that
/// still flow through containers whose storage isn't yet Value-backed
/// (tuples, dicts, sets, instances, generator locals — all flip in
/// S2.7 together with codegen tagging raw function pointers).
fn mark_object(v: pyaot_core_defs::Value) {
    unsafe {
        if !v.is_ptr() {
            return;
        }
        let obj = v.unwrap_ptr::<Obj>();

        // Heuristic address filter: the tag bit says "pointer", but the
        // pre-S2.7 storage (tuple.data / dict entries / set entries /
        // instance fields / generator locals) holds raw `*mut Obj` bits
        // that may still be garbage (small-int cast, 4-byte-aligned
        // code pointer, etc.). Reject anything that clearly isn't a
        // live heap allocation. The proper fix — per-slot Value tagging
        // — lands in S2.7.
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
                // Phase 2 S2.3: list storage is `[Value]`. Walk each slot
                // and let `mark_object` do the rest — immediates self-
                // describe as non-pointers via `is_ptr()` and return
                // immediately, heap pointers recurse as before.
                let len = (*list).len;
                let data = (*list).data;
                if data.is_null() && len > 0 {
                    eprintln!(
                        "FATAL: List heap corruption - null data pointer with len={}",
                        len
                    );
                    std::process::abort();
                }
                for i in 0..len {
                    mark_object(*data.add(i));
                }
            }
            TypeTagKind::Tuple => {
                let tuple = obj as *mut TupleObj;
                let mask = (*tuple).heap_field_mask;
                // S2.6 narrow scope: tuple storage is still raw `*mut Obj`;
                // `heap_field_mask` stays until S2.7 flips storage +
                // codegen together (see §2.3 Amendment 2). Wrap each
                // raw slot via `Value::from_ptr` before recursing so
                // `mark_object` can apply its uniform Value-typed API.
                if mask != 0 {
                    let len = (*tuple).len;
                    let trace_count = len.min(64);
                    let data = (*tuple).data.as_ptr();
                    for i in 0..trace_count {
                        if mask & (1u64 << i) != 0 {
                            let elem = *data.add(i);
                            mark_object(pyaot_core_defs::Value::from_ptr(elem));
                        }
                    }
                }
            }
            TypeTagKind::Dict => {
                let dict = obj as *mut DictObj;
                let entries = (*dict).entries;
                let entries_len = (*dict).entries_len;
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
                        mark_object(pyaot_core_defs::Value::from_ptr(key));
                        mark_object(pyaot_core_defs::Value::from_ptr((*entry).value));
                    }
                }
            }
            TypeTagKind::Set => {
                let set = obj as *mut SetObj;
                let capacity = (*set).capacity;
                let entries = (*set).entries;
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
                    if elem != TOMBSTONE {
                        mark_object(pyaot_core_defs::Value::from_ptr(elem));
                    }
                }
            }
            TypeTagKind::Instance => {
                let instance = obj as *mut InstanceObj;
                let field_count = (*instance).field_count;
                let trace_count = field_count.min(64);
                let fields = (*instance).fields.as_ptr();
                // Only mark fields that are heap objects (pointers), not raw
                // int/float/bool values. `ClassInfo.heap_field_mask` tells us
                // which fields are heap types; stays until S2.7 (see §2.3
                // Amendment 2).
                let mask = crate::vtable::get_class_heap_field_mask((*instance).class_id);
                for i in 0..trace_count {
                    if mask & (1u64 << i) != 0 {
                        let field = *fields.add(i);
                        mark_object(pyaot_core_defs::Value::from_ptr(field));
                    }
                }
            }
            TypeTagKind::Iterator => {
                let iterator = obj as *mut IteratorObj;
                let kind = IteratorKind::try_from((*iterator).kind);

                // Local helper — keeps the per-variant code terse while
                // still wrapping each raw field pointer for the
                // Value-typed mark_object API.
                let mark_ptr = |p: *mut Obj| mark_object(pyaot_core_defs::Value::from_ptr(p));

                match kind {
                    Ok(IteratorKind::Map) => {
                        let map_iter = obj as *mut MapIterObj;
                        mark_ptr((*map_iter).inner_iter);
                        mark_ptr((*map_iter).captures);
                    }
                    Ok(IteratorKind::Filter) => {
                        let filter_iter = obj as *mut FilterIterObj;
                        mark_ptr((*filter_iter).inner_iter);
                        mark_ptr((*filter_iter).captures);
                    }
                    Ok(IteratorKind::Zip) => {
                        let zip_iter = obj as *mut ZipIterObj;
                        mark_ptr((*zip_iter).iter1);
                        mark_ptr((*zip_iter).iter2);
                    }
                    Ok(IteratorKind::Chain) => {
                        let chain_iter = obj as *mut ChainIterObj;
                        mark_ptr((*chain_iter).iters);
                    }
                    Ok(IteratorKind::ISlice) => {
                        let islice_iter = obj as *mut ISliceIterObj;
                        mark_ptr((*islice_iter).inner_iter);
                    }
                    Ok(IteratorKind::Zip3) => {
                        let zip3_iter = obj as *mut Zip3IterObj;
                        mark_ptr((*zip3_iter).iter1);
                        mark_ptr((*zip3_iter).iter2);
                        mark_ptr((*zip3_iter).iter3);
                    }
                    Ok(IteratorKind::ZipN) => {
                        let zipn_iter = obj as *mut ZipNIterObj;
                        // iters is a ListObj containing the iterators.
                        mark_ptr((*zipn_iter).iters);
                    }
                    _ => {
                        // Standard iterators (List/Tuple/Dict/String/Range/
                        // Set/Bytes/Enumerate) keep their source in one slot.
                        mark_ptr((*iterator).source);
                    }
                }
            }
            TypeTagKind::Cell => {
                let cell = obj as *mut CellObj;
                if let Some(ptr) = cell_get_ptr_for_gc(cell) {
                    mark_object(pyaot_core_defs::Value::from_ptr(ptr));
                }
            }
            TypeTagKind::Generator => {
                // Generators store local variables with precise type
                // information via `type_tags` (retained until S2.7). Each
                // heap-pointer local is wrapped as Value before the
                // recursive mark.
                use crate::object::LOCAL_TYPE_PTR;

                let gen = obj as *mut GeneratorObj;
                let num_locals = (*gen).num_locals;
                let locals = (*gen).locals.as_ptr();
                let type_tags = (*gen).type_tags;

                if !type_tags.is_null() {
                    for i in 0..num_locals as usize {
                        if *type_tags.add(i) == LOCAL_TYPE_PTR {
                            let ptr = *locals.add(i) as *mut Obj;
                            mark_object(pyaot_core_defs::Value::from_ptr(ptr));
                        }
                        // Other tags (LOCAL_TYPE_RAW_INT / RAW_FLOAT /
                        // RAW_BOOL) are raw values — skip.
                    }
                }

                // Trace sent_value only if its type tag says heap pointer.
                if (*gen).sent_value_tag == LOCAL_TYPE_PTR {
                    let sent_ptr = (*gen).sent_value as *mut Obj;
                    mark_object(pyaot_core_defs::Value::from_ptr(sent_ptr));
                }
            }
            TypeTagKind::File => {
                let file = obj as *mut FileObj;
                mark_object(pyaot_core_defs::Value::from_ptr((*file).name));
            }
            TypeTagKind::Match => {
                let match_obj = obj as *mut MatchObj;
                mark_object(pyaot_core_defs::Value::from_ptr((*match_obj).groups));
                mark_object(pyaot_core_defs::Value::from_ptr((*match_obj).original));
            }
            TypeTagKind::CompletedProcess => {
                let cp_obj = obj as *mut CompletedProcessObj;
                mark_object(pyaot_core_defs::Value::from_ptr((*cp_obj).args));
                mark_object(pyaot_core_defs::Value::from_ptr((*cp_obj).stdout));
                mark_object(pyaot_core_defs::Value::from_ptr((*cp_obj).stderr));
            }
            TypeTagKind::ParseResult => {
                let pr_obj = obj as *mut crate::object::ParseResultObj;
                mark_object(pyaot_core_defs::Value::from_ptr((*pr_obj).scheme));
                mark_object(pyaot_core_defs::Value::from_ptr((*pr_obj).netloc));
                mark_object(pyaot_core_defs::Value::from_ptr((*pr_obj).path));
                mark_object(pyaot_core_defs::Value::from_ptr((*pr_obj).params));
                mark_object(pyaot_core_defs::Value::from_ptr((*pr_obj).query));
                mark_object(pyaot_core_defs::Value::from_ptr((*pr_obj).fragment));
            }
            TypeTagKind::HttpResponse => {
                let hr_obj = obj as *mut crate::object::HttpResponseObj;
                mark_object(pyaot_core_defs::Value::from_ptr((*hr_obj).url));
                mark_object(pyaot_core_defs::Value::from_ptr((*hr_obj).headers));
                mark_object(pyaot_core_defs::Value::from_ptr((*hr_obj).body));
            }
            TypeTagKind::Request => {
                let req_obj = obj as *mut crate::object::RequestObj;
                mark_object(pyaot_core_defs::Value::from_ptr((*req_obj).url));
                mark_object(pyaot_core_defs::Value::from_ptr((*req_obj).data));
                mark_object(pyaot_core_defs::Value::from_ptr((*req_obj).headers));
                mark_object(pyaot_core_defs::Value::from_ptr((*req_obj).method));
            }
            // DefaultDict and Counter use the same dict layout — mark entries
            TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                let dict = obj as *mut DictObj;
                let entries = (*dict).entries;
                let entries_len = (*dict).entries_len;
                if entries.is_null() && entries_len > 0 {
                    eprintln!(
                        "FATAL: DefaultDict/Counter heap corruption - null entries pointer with entries_len={}",
                        entries_len
                    );
                    std::process::abort();
                }
                for i in 0..entries_len {
                    let entry = entries.add(i);
                    let key = (*entry).key;
                    if !key.is_null() {
                        mark_object(pyaot_core_defs::Value::from_ptr(key));
                        mark_object(pyaot_core_defs::Value::from_ptr((*entry).value));
                    }
                }
            }
            TypeTagKind::Deque => {
                let deque = obj as *mut crate::object::DequeObj;
                if (*deque).elem_tag == 0 {
                    // ELEM_HEAP_OBJ
                    let data = (*deque).data;
                    let head = (*deque).head;
                    let len = (*deque).len;
                    let cap = (*deque).capacity;
                    if !data.is_null() && cap > 0 {
                        for i in 0..len {
                            let idx = (head + i) % cap;
                            let elem = *data.add(idx);
                            mark_object(pyaot_core_defs::Value::from_ptr(elem));
                        }
                    }
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

    // 1. Sweep slab-allocated objects (small objects tracked via slab pages)
    unsafe {
        let bytes_freed = crate::slab::slab().sweep();
        state.total_allocated = state.total_allocated.saturating_sub(bytes_freed);
    }

    // 2. Sweep large objects (tracked in state.objects Vec)
    state.objects.retain(|obj_ptr| unsafe {
        let obj = &mut **obj_ptr;
        if !obj.is_marked() {
            // Finalize objects before freeing to release auxiliary allocations
            crate::slab::finalize_object_pub(*obj_ptr);
            // Free this object
            let layout = Layout::from_size_align(obj.header.size, std::mem::align_of::<Obj>())
                .expect("sweep: invalid layout during dealloc");
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
