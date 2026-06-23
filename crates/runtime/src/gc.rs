//! Precise mark-sweep GC with shadow-stack roots.
//! GC stress: `RUSTFLAGS="--cfg gc_stress_test"` collects on every allocation.

use crate::cell::{cell_get_ptr_for_gc, CellObj};
use crate::class_attrs::get_class_attr_pointers;
use crate::globals::get_global_pointers;
use crate::object::{
    ChainIterObj, CompletedProcessObj, DequeObj, DictObj, FileObj, FilterIterObj, GeneratorObj,
    HttpResponseObj, ISliceIterObj, InstanceObj, IteratorKind, IteratorObj, ListObj, MapIterObj,
    MatchObj, Obj, ObjHeader, ParseResultObj, RequestObj, SetObj, TupleObj, TypeTagKind,
    Zip3IterObj, ZipIterObj, ZipNIterObj, TOMBSTONE,
};
use pyaot_core_defs::Value;
use std::alloc::{alloc, dealloc, Layout};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

#[repr(C)]
pub struct ShadowFrame {
    pub prev: *mut ShadowFrame,
    pub nroots: usize,
    pub roots: *mut *mut Obj,
}
const _: () = assert!(
    std::mem::size_of::<ShadowFrame>() == pyaot_core_defs::layout::SHADOW_FRAME_SIZE as usize
        && std::mem::offset_of!(ShadowFrame, nroots)
            == pyaot_core_defs::layout::SHADOW_FRAME_NROOTS_OFFSET as usize
        && std::mem::offset_of!(ShadowFrame, roots)
            == pyaot_core_defs::layout::SHADOW_FRAME_ROOTS_OFFSET as usize,
);

struct GcState {
    stack_top: *mut ShadowFrame,
    objects: Vec<*mut Obj>,
    total_allocated: usize,
    threshold: usize,
    collecting: bool,
}
unsafe impl Send for GcState {}
unsafe impl Sync for GcState {}
static GC_STATE_PTR: AtomicPtr<GcState> = AtomicPtr::new(ptr::null_mut());

#[inline(always)]
unsafe fn gc_state() -> &'static mut GcState {
    let p = GC_STATE_PTR.load(Ordering::Acquire);
    if p.is_null() {
        eprintln!("FATAL: GC accessed before init");
        std::process::abort();
    }
    &mut *p
}

pub fn init() {
    let s = Box::into_raw(Box::new(GcState {
        stack_top: ptr::null_mut(),
        objects: Vec::new(),
        total_allocated: 0,
        threshold: 1024 * 1024,
        collecting: false,
    }));
    if GC_STATE_PTR
        .compare_exchange(ptr::null_mut(), s, Ordering::Release, Ordering::Relaxed)
        .is_err()
    {
        unsafe { drop(Box::from_raw(s)) };
    }
}

pub fn shutdown() {
    unsafe {
        let p = GC_STATE_PTR.swap(ptr::null_mut(), Ordering::AcqRel);
        if p.is_null() {
            return;
        }
        let s = &mut *p;
        for o in &s.objects {
            crate::slab::finalize_object_pub(*o);
            let layout = Layout::from_size_align((**o).header.size, std::mem::align_of::<Obj>())
                .expect("shutdown layout");
            dealloc(*o as *mut u8, layout);
        }
        s.objects.clear();
        crate::slab::slab().shutdown();
        drop(Box::from_raw(p));
    }
}

#[no_mangle]
pub unsafe extern "C" fn gc_push(frame: *mut ShadowFrame) {
    if frame.is_null() {
        return;
    }
    let s = gc_state();
    (*frame).prev = s.stack_top;
    s.stack_top = frame;
}

#[no_mangle]
pub extern "C" fn gc_pop() {
    unsafe {
        let s = gc_state();
        if !s.stack_top.is_null() {
            s.stack_top = (*s.stack_top).prev;
        }
    }
}

pub fn get_stack_top() -> *mut ShadowFrame {
    unsafe { gc_state().stack_top }
}

/// Pop every shadow frame that lives BELOW `sp` (exception unwinding).
///
/// Shadow frames are stack slots of the functions that pushed them, and the
/// machine stack grows down — so when an unwind abandons all frames deeper
/// than the handler's, exactly the shadow frames at addresses `< sp` belong
/// to dead functions. The handler function's own shadow frame (an address
/// within its frame, `>= sp`) survives.
pub fn unwind_below(sp: usize) {
    unsafe {
        let s = gc_state();
        let mut cur = s.stack_top;
        while !cur.is_null() && (cur as usize) < sp {
            cur = (*cur).prev;
        }
        s.stack_top = cur;
    }
}

#[no_mangle]
pub extern "C" fn gc_alloc(size: usize, type_tag: u8) -> *mut Obj {
    let tag = TypeTagKind::from_tag(type_tag)
        .unwrap_or_else(|| panic!("gc_alloc: invalid type tag {}", type_tag));
    assert!(size >= std::mem::size_of::<ObjHeader>());
    unsafe {
        let s = gc_state();
        if !s.collecting {
            #[cfg(gc_stress_test)]
            collect_impl(s);
            #[cfg(not(gc_stress_test))]
            if s.total_allocated > s.threshold {
                collect_impl(s);
            }
        }
        let p = if crate::slab::is_slab_size(size) {
            crate::slab::slab().alloc(size) as *mut Obj
        } else {
            let layout = Layout::from_size_align(size, std::mem::align_of::<Obj>())
                .expect("gc_alloc layout");
            let p = alloc(layout) as *mut Obj;
            if p.is_null() {
                panic!("Out of memory");
            }
            s.objects.push(p);
            p
        };
        (*p).header = ObjHeader {
            type_tag: tag,
            marked: false,
            size,
        };
        ptr::write_bytes(
            (p as *mut u8).add(std::mem::size_of::<ObjHeader>()),
            0,
            size - std::mem::size_of::<ObjHeader>(),
        );
        s.total_allocated = s.total_allocated.saturating_add(size);
        p
    }
}

fn collect_impl(s: &mut GcState) {
    s.collecting = true;
    mark_roots(s);
    sweep(s);
    s.threshold = s.total_allocated.saturating_mul(2).max(1024 * 1024);
    s.collecting = false;
}

fn mark_roots(s: &mut GcState) {
    unsafe {
        let mut frame = s.stack_top;
        while !frame.is_null() {
            for i in 0..(*frame).nroots {
                mark_object(Value::from_ptr(*(*frame).roots.add(i)));
            }
            frame = (*frame).prev;
        }
    }
    for p in get_global_pointers() {
        mark_object(Value::from_ptr(p));
    }
    for p in get_class_attr_pointers() {
        mark_object(Value::from_ptr(p));
    }
    for p in crate::exceptions::get_exception_pointers() {
        mark_object(Value::from_ptr(p));
    }
    for p in crate::sys::get_sys_module_roots() {
        mark_object(Value::from_ptr(p));
    }
}

/// Mark a `Value` and the transitive closure of its heap children.
///
/// The traversal uses an explicit worklist (`Vec<*mut Obj>`) rather than native
/// recursion: deeply-nested structures (e.g. a 50k-deep list-of-lists) would
/// otherwise overflow the native stack. Children are *marked on enqueue*, so
/// the worklist never holds duplicates and popped objects are already marked.
///
/// `Value::is_ptr()` filters tagged primitives. After S3.3b.2 closed the
/// decorator-wrapper return-type leak (§P.2.3), the only pointer-shaped
/// non-object path is via wrapper return locals that fall back to runtime
/// trampoline (e.g. `*args` wrappers whose user-visible param is a
/// packed tuple — devirt skipped by design). Those still flow `Type::Any`,
/// but their values are always heap objects (results from runtime calls),
/// not raw fn-pointers — so the alignment / TypeTag sanity guards are
/// redundant. Keeping only the null check; everything else is on the
/// caller to type correctly.
fn mark_object(root: Value) {
    unsafe {
        let mut worklist: Vec<*mut Obj> = Vec::new();

        // Enqueue a tagged `Value` child if it is a non-null, not-yet-marked
        // heap pointer (marking it eagerly to dedup the worklist).
        macro_rules! enqueue_val {
            ($v:expr) => {{
                let vv: Value = $v;
                if vv.is_ptr() {
                    let p = vv.unwrap_ptr::<Obj>();
                    // `is_ptr()` only tests bit 0; a real heap object is always
                    // 8-aligned. This debug guard catches a future regression
                    // that lets a raw, non-8-aligned word (e.g. an unboxed float
                    // bit pattern) reach a GC-traced slot before it corrupts the
                    // heap walk by dereferencing a non-pointer (see PITFALLS B12).
                    debug_assert!(
                        p.is_null() || (p as usize).is_multiple_of(8),
                        "GC traced a misaligned (non-object) value as a pointer"
                    );
                    if !p.is_null() && !(*p).is_marked() {
                        (*p).set_marked(true);
                        worklist.push(p);
                    }
                }
            }};
        }
        // Enqueue a raw `*mut Obj` child (struct field; already a real pointer).
        macro_rules! enqueue_ptr {
            ($p:expr) => {{
                let p: *mut Obj = $p;
                if !p.is_null() && !(*p).is_marked() {
                    (*p).set_marked(true);
                    worklist.push(p);
                }
            }};
        }
        macro_rules! fields {
            ($o:expr, $t:ty, $($f:ident),+ $(,)?) => {{
                let p = $o as *mut $t;
                $(enqueue_ptr!((*p).$f);)+
            }};
        }

        enqueue_val!(root);
        while let Some(obj) = worklist.pop() {
            // `obj` is a non-null heap pointer already marked at enqueue time.
            match (*obj).type_tag() {
                TypeTagKind::List => {
                    let p = obj as *mut ListObj;
                    for i in 0..(*p).len {
                        enqueue_val!(*(*p).data.add(i));
                    }
                }
                // A closure is a `TupleObj`-layout object with a distinct tag:
                // trace it identically (slot 0's int-tagged code address is not a
                // pointer, so `enqueue_val!`'s `is_ptr` check skips it; slots 1..=N
                // are the captured cells).
                TypeTagKind::Tuple | TypeTagKind::Closure => {
                    let p = obj as *mut TupleObj;
                    for i in 0..(*p).len {
                        enqueue_val!(*(*p).data.as_ptr().add(i));
                    }
                }
                TypeTagKind::Instance => {
                    let p = obj as *mut InstanceObj;
                    // Storage is uniform tagged Value (Phase 2). `Value::is_ptr()`
                    // filters out non-pointer tags exhaustively; no per-class raw
                    // mask consultation needed.
                    for k in 0..(*p).field_count {
                        enqueue_val!(*(*p).fields.as_ptr().add(k));
                    }
                }
                TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                    let d = obj as *mut DictObj;
                    for i in 0..(*d).entries_len {
                        let e = (*d).entries.add(i);
                        if (*e).key.0 != 0 {
                            enqueue_val!((*e).key);
                            enqueue_val!((*e).value);
                        }
                    }
                }
                // FrozenSet shares `SetObj` layout — trace its element pointers
                // identically to `Set`.
                TypeTagKind::Set | TypeTagKind::FrozenSet => {
                    let st = obj as *mut SetObj;
                    for i in 0..(*st).capacity {
                        let e = (*st).entries.add(i);
                        if (*e).elem.0 != 0 && (*e).elem != TOMBSTONE {
                            enqueue_val!((*e).elem);
                        }
                    }
                }
                TypeTagKind::Generator => {
                    let g = obj as *mut GeneratorObj;
                    for k in 0..(*g).num_locals as usize {
                        enqueue_val!(*(*g).locals.as_ptr().add(k));
                    }
                    enqueue_val!((*g).sent_value);
                }
                TypeTagKind::Iterator => {
                    match IteratorKind::try_from((*(obj as *mut IteratorObj)).kind) {
                        Ok(IteratorKind::Map) | Ok(IteratorKind::MapTagged) => {
                            fields!(obj, MapIterObj, inner_iter, captures)
                        }
                        Ok(IteratorKind::Filter) | Ok(IteratorKind::FilterTagged) => {
                            fields!(obj, FilterIterObj, inner_iter, captures)
                        }
                        Ok(IteratorKind::Zip) => fields!(obj, ZipIterObj, iter1, iter2),
                        Ok(IteratorKind::Zip3) => fields!(obj, Zip3IterObj, iter1, iter2, iter3),
                        Ok(IteratorKind::Chain) => {
                            fields!(obj, ChainIterObj, iters, current_iter)
                        }
                        Ok(IteratorKind::ISlice) => fields!(obj, ISliceIterObj, inner_iter),
                        Ok(IteratorKind::ZipN) => fields!(obj, ZipNIterObj, iters),
                        _ => {
                            let it = obj as *mut IteratorObj;
                            enqueue_ptr!((*it).source);
                            // The dict/set mutation guard keeps the live container
                            // alive; null (skipped) for every other iterator kind.
                            enqueue_ptr!((*it).size_guard);
                        }
                    }
                }
                TypeTagKind::Cell => {
                    if let Some(p) = cell_get_ptr_for_gc(obj as *mut CellObj) {
                        enqueue_ptr!(p);
                    }
                }
                TypeTagKind::File => fields!(obj, FileObj, name),
                TypeTagKind::Match => fields!(obj, MatchObj, groups, original),
                TypeTagKind::CompletedProcess => {
                    fields!(obj, CompletedProcessObj, args, stdout, stderr)
                }
                TypeTagKind::ParseResult => fields!(
                    obj,
                    ParseResultObj,
                    scheme,
                    netloc,
                    path,
                    params,
                    query,
                    fragment
                ),
                TypeTagKind::HttpResponse => fields!(obj, HttpResponseObj, url, headers, body),
                TypeTagKind::Request => fields!(obj, RequestObj, url, data, headers, method),
                TypeTagKind::Deque => {
                    let d = obj as *mut DequeObj;
                    let (data, head, len, cap) = ((*d).data, (*d).head, (*d).len, (*d).capacity);
                    if !data.is_null() {
                        for i in 0..len {
                            enqueue_val!(*data.add((head + i) % cap));
                        }
                    }
                }
                // Atoms (Str/Float/Bytes/None/Int/Bool/Range/Hash/StringIO/BytesIO/...):
                // no heap children.
                _ => {}
            }
        }
    }
}

fn sweep(s: &mut GcState) {
    unsafe {
        crate::string::prune_string_pool();
    }
    unsafe {
        s.total_allocated = s
            .total_allocated
            .saturating_sub(crate::slab::slab().sweep());
    }
    s.objects.retain(|o| unsafe {
        let obj = &mut **o;
        if !obj.is_marked() {
            crate::slab::finalize_object_pub(*o);
            let layout = Layout::from_size_align(obj.header.size, std::mem::align_of::<Obj>())
                .expect("sweep layout");
            s.total_allocated = s.total_allocated.saturating_sub(obj.header.size);
            dealloc(*o as *mut u8, layout);
            false
        } else {
            obj.set_marked(false);
            true
        }
    });
}
