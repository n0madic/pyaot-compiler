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
    stack_depth: usize,
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
        stack_depth: 0,
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
    s.stack_depth += 1;
    (*frame).prev = s.stack_top;
    s.stack_top = frame;
}

#[no_mangle]
pub extern "C" fn gc_pop() {
    unsafe {
        let s = gc_state();
        if !s.stack_top.is_null() {
            s.stack_top = (*s.stack_top).prev;
            s.stack_depth = s.stack_depth.saturating_sub(1);
        }
    }
}

#[no_mangle]
pub extern "C" fn gc_get_stack_depth() -> usize {
    unsafe { gc_state().stack_depth }
}
pub fn get_stack_top() -> *mut ShadowFrame {
    unsafe { gc_state().stack_top }
}

pub fn unwind_to(target: *mut ShadowFrame) {
    unsafe {
        let s = gc_state();
        let mut popped = 0;
        let mut cur = s.stack_top;
        while !cur.is_null() && cur != target {
            popped += 1;
            cur = (*cur).prev;
        }
        s.stack_top = target;
        s.stack_depth = s.stack_depth.saturating_sub(popped);
        if cur.is_null() && !target.is_null() {
            eprintln!("FATAL: unwind_to: target frame not in shadow stack");
            std::process::abort();
        }
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

#[no_mangle]
pub extern "C" fn gc_collect() {
    unsafe {
        collect_impl(gc_state());
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

/// Mark a `Value`; if it's a heap pointer recurse into its children.
/// `Value::is_ptr()` filters tagged primitives. The alignment / low-page
/// guard and the `TypeTagKind::from_tag` validation reject pointer-shaped
/// non-objects: F.7c closed Instance.fields (the largest residual source),
/// but a smaller set of paths (closures/decorators/generators) still
/// produces values that pass `is_ptr()` without being heap objects, and the
/// guards keep the GC robust until those are tracked down individually.
fn mark_object(v: Value) {
    unsafe {
        if !v.is_ptr() {
            return;
        }
        let obj = v.unwrap_ptr::<Obj>();
        if obj.is_null()
            || (obj as usize) < 0x1000
            || !(obj as usize).is_multiple_of(std::mem::align_of::<Obj>())
        {
            return;
        }
        if TypeTagKind::from_tag((*obj).type_tag() as u8).is_none() {
            return;
        }
        if (*obj).is_marked() {
            return;
        }
        (*obj).set_marked(true);
        let mp = |p: *mut Obj| mark_object(Value::from_ptr(p));
        macro_rules! fields { ($t:ty, $($f:ident),+ $(,)?) => {{ let p = obj as *mut $t; $(mp((*p).$f);)+ }} }
        match (*obj).type_tag() {
            TypeTagKind::List => {
                let p = obj as *mut ListObj;
                for i in 0..(*p).len {
                    mark_object(*(*p).data.add(i));
                }
            }
            TypeTagKind::Tuple => {
                let p = obj as *mut TupleObj;
                for i in 0..(*p).len {
                    mark_object(*(*p).data.as_ptr().add(i));
                }
            }
            TypeTagKind::Instance => {
                let p = obj as *mut InstanceObj;
                for k in 0..(*p).field_count {
                    mark_object(*(*p).fields.as_ptr().add(k));
                }
            }
            TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => {
                let d = obj as *mut DictObj;
                for i in 0..(*d).entries_len {
                    let e = (*d).entries.add(i);
                    if (*e).key.0 != 0 {
                        mark_object((*e).key);
                        mark_object((*e).value);
                    }
                }
            }
            TypeTagKind::Set => {
                let st = obj as *mut SetObj;
                for i in 0..(*st).capacity {
                    let e = (*st).entries.add(i);
                    if (*e).elem.0 != 0 && (*e).elem != TOMBSTONE {
                        mark_object((*e).elem);
                    }
                }
            }
            TypeTagKind::Generator => {
                let g = obj as *mut GeneratorObj;
                for k in 0..(*g).num_locals as usize {
                    mark_object(*(*g).locals.as_ptr().add(k));
                }
                mark_object((*g).sent_value);
            }
            TypeTagKind::Iterator => {
                match IteratorKind::try_from((*(obj as *mut IteratorObj)).kind) {
                    Ok(IteratorKind::Map) => fields!(MapIterObj, inner_iter, captures),
                    Ok(IteratorKind::Filter) => fields!(FilterIterObj, inner_iter, captures),
                    Ok(IteratorKind::Zip) => fields!(ZipIterObj, iter1, iter2),
                    Ok(IteratorKind::Zip3) => fields!(Zip3IterObj, iter1, iter2, iter3),
                    Ok(IteratorKind::Chain) => fields!(ChainIterObj, iters),
                    Ok(IteratorKind::ISlice) => fields!(ISliceIterObj, inner_iter),
                    Ok(IteratorKind::ZipN) => fields!(ZipNIterObj, iters),
                    _ => mp((*(obj as *mut IteratorObj)).source),
                }
            }
            TypeTagKind::Cell => {
                if let Some(p) = cell_get_ptr_for_gc(obj as *mut CellObj) {
                    mp(p);
                }
            }
            TypeTagKind::File => fields!(FileObj, name),
            TypeTagKind::Match => fields!(MatchObj, groups, original),
            TypeTagKind::CompletedProcess => fields!(CompletedProcessObj, args, stdout, stderr),
            TypeTagKind::ParseResult => fields!(
                ParseResultObj,
                scheme,
                netloc,
                path,
                params,
                query,
                fragment
            ),
            TypeTagKind::HttpResponse => fields!(HttpResponseObj, url, headers, body),
            TypeTagKind::Request => fields!(RequestObj, url, data, headers, method),
            TypeTagKind::Deque => {
                let d = obj as *mut DequeObj;
                let (data, head, len, cap) = ((*d).data, (*d).head, (*d).len, (*d).capacity);
                if !data.is_null() {
                    for i in 0..len {
                        mark_object(*data.add((head + i) % cap));
                    }
                }
            }
            // Atoms (Str/Float/Bytes/None/Int/Bool/Range/Hash/StringIO/BytesIO/...): no heap children.
            _ => {}
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
