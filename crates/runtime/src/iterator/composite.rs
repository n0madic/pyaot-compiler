//! Composite iterators: zip, map, filter, chain, islice
//!
//! These iterators wrap other iterators to transform or combine them.

use super::next::rt_iter_next_no_exc;
use crate::gc;
use crate::object::{Obj, TypeTagKind};

// ==================== Function Type Aliases ====================

/// Function type for map: takes element, returns transformed element
type MapFn = extern "C" fn(*mut Obj) -> *mut Obj;
/// Map function with 1 capture
type MapFn1 = extern "C" fn(*mut Obj, *mut Obj) -> *mut Obj;
/// Map function with 2 captures
type MapFn2 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Map function with 3 captures
type MapFn3 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Map function with 4 captures
type MapFn4 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Map function with 5 captures
type MapFn5 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Map function with 6 captures
type MapFn6 =
    extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Map function with 7 captures
type MapFn7 = extern "C" fn(
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
) -> *mut Obj;
/// Map function with 8 captures
type MapFn8 = extern "C" fn(
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
) -> *mut Obj;

/// Function type for filter: takes element, returns bool (as i8)
type FilterFn = extern "C" fn(*mut Obj) -> i8;
/// Filter function with 1 capture
type FilterFn1 = extern "C" fn(*mut Obj, *mut Obj) -> i8;
/// Filter function with 2 captures
type FilterFn2 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj) -> i8;
/// Filter function with 3 captures
type FilterFn3 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj) -> i8;
/// Filter function with 4 captures
type FilterFn4 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> i8;
/// Filter function with 5 captures
type FilterFn5 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> i8;
/// Filter function with 6 captures
type FilterFn6 =
    extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> i8;
/// Filter function with 7 captures
type FilterFn7 = extern "C" fn(
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
) -> i8;
/// Filter function with 8 captures
type FilterFn8 = extern "C" fn(
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
) -> i8;

// ==================== Helper Functions ====================

/// Call map function with captures extracted from tuple
/// Dispatch based on capture_count (0-4)
pub(crate) unsafe fn call_map_with_captures(
    func_ptr: i64,
    captures: *mut Obj,
    capture_count: u8,
    elem: *mut Obj,
) -> *mut Obj {
    use crate::tuple::rt_tuple_get;

    match capture_count {
        0 => {
            let func: MapFn = std::mem::transmute(func_ptr);
            func(elem)
        }
        1 => {
            let c0 = rt_tuple_get(captures, 0);
            let func: MapFn1 = std::mem::transmute(func_ptr);
            func(c0, elem)
        }
        2 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let func: MapFn2 = std::mem::transmute(func_ptr);
            func(c0, c1, elem)
        }
        3 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let func: MapFn3 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, elem)
        }
        4 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let func: MapFn4 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, elem)
        }
        5 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let func: MapFn5 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, elem)
        }
        6 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let func: MapFn6 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, c5, elem)
        }
        7 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let c6 = rt_tuple_get(captures, 6);
            let func: MapFn7 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, c5, c6, elem)
        }
        8 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let c6 = rt_tuple_get(captures, 6);
            let c7 = rt_tuple_get(captures, 7);
            let func: MapFn8 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, c5, c6, c7, elem)
        }
        _ => {
            eprintln!(
                "FATAL: map: too many captures (max 8 supported, got {})",
                capture_count
            );
            std::process::abort();
        }
    }
}

/// Call filter function with captures extracted from tuple
/// Dispatch based on capture_count (0-4)
/// Returns: true (non-zero) if element passes, false (0) if not
pub(crate) unsafe fn call_filter_with_captures(
    func_ptr: i64,
    captures: *mut Obj,
    capture_count: u8,
    elem: *mut Obj,
) -> bool {
    use crate::tuple::rt_tuple_get;

    let result = match capture_count {
        0 => {
            let func: FilterFn = std::mem::transmute(func_ptr);
            func(elem)
        }
        1 => {
            let c0 = rt_tuple_get(captures, 0);
            let func: FilterFn1 = std::mem::transmute(func_ptr);
            func(c0, elem)
        }
        2 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let func: FilterFn2 = std::mem::transmute(func_ptr);
            func(c0, c1, elem)
        }
        3 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let func: FilterFn3 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, elem)
        }
        4 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let func: FilterFn4 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, elem)
        }
        5 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let func: FilterFn5 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, elem)
        }
        6 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let func: FilterFn6 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, c5, elem)
        }
        7 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let c6 = rt_tuple_get(captures, 6);
            let func: FilterFn7 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, c5, c6, elem)
        }
        8 => {
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let c6 = rt_tuple_get(captures, 6);
            let c7 = rt_tuple_get(captures, 7);
            let func: FilterFn8 = std::mem::transmute(func_ptr);
            func(c0, c1, c2, c3, c4, c5, c6, c7, elem)
        }
        _ => {
            eprintln!(
                "FATAL: filter: too many captures (max 8 supported, got {})",
                capture_count
            );
            std::process::abort();
        }
    };
    result != 0
}

// ==================== Zip Iterator ====================

/// Create a zip iterator from two iterators
/// Returns: new iterator that yields tuples
#[no_mangle]
pub extern "C" fn rt_zip_new(iter1: *mut Obj, iter2: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, ZipIterObj};

    // Allocate zip iterator object
    let size = std::mem::size_of::<ZipIterObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let zip_iter = obj as *mut ZipIterObj;
        (*zip_iter).kind = IteratorKind::Zip as u8;
        (*zip_iter).exhausted = false;
        (*zip_iter)._pad = [0; 6];
        (*zip_iter).iter1 = iter1;
        (*zip_iter).iter2 = iter2;
    }

    obj
}

/// Get next tuple from zip iterator
/// Returns: tuple or null (StopIteration) if either iterator is exhausted
#[no_mangle]
pub extern "C" fn rt_zip_next(zip_obj: *mut Obj) -> *mut Obj {
    use crate::object::ZipIterObj;
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if zip_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let zip_iter = zip_obj as *mut ZipIterObj;

        if (*zip_iter).exhausted {
            return std::ptr::null_mut();
        }

        // Use rt_iter_next_no_exc to avoid longjmp issues
        // rt_iter_next raises StopIteration via longjmp, making null checks unreachable
        let item1 = rt_iter_next_no_exc((*zip_iter).iter1);
        if item1.is_null() {
            (*zip_iter).exhausted = true;
            return std::ptr::null_mut();
        }

        let item2 = rt_iter_next_no_exc((*zip_iter).iter2);
        if item2.is_null() {
            (*zip_iter).exhausted = true;
            return std::ptr::null_mut();
        }

        // Create tuple with both items
        let tuple = rt_make_tuple(2);
        rt_tuple_set(tuple, 0, item1);
        rt_tuple_set(tuple, 1, item2);

        tuple
    }
}

// ==================== Zip3 Iterator ====================

/// Create a zip iterator from three iterators
/// Returns: new iterator that yields 3-tuples
#[no_mangle]
pub extern "C" fn rt_zip3_new(iter1: *mut Obj, iter2: *mut Obj, iter3: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, Zip3IterObj};

    let size = std::mem::size_of::<Zip3IterObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let zip_iter = obj as *mut Zip3IterObj;
        (*zip_iter).kind = IteratorKind::Zip3 as u8;
        (*zip_iter).exhausted = false;
        (*zip_iter)._pad = [0; 6];
        (*zip_iter).iter1 = iter1;
        (*zip_iter).iter2 = iter2;
        (*zip_iter).iter3 = iter3;
    }

    obj
}

// ==================== ZipN Iterator ====================

/// Create a zip iterator from N iterators (stored in a list)
/// iters: list of iterators
/// count: number of iterators
/// Returns: new iterator that yields N-tuples
#[no_mangle]
pub extern "C" fn rt_zipn_new(iters: *mut Obj, count: i64) -> *mut Obj {
    use crate::object::{IteratorKind, ZipNIterObj};

    let size = std::mem::size_of::<ZipNIterObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let zip_iter = obj as *mut ZipNIterObj;
        (*zip_iter).kind = IteratorKind::ZipN as u8;
        (*zip_iter).exhausted = false;
        (*zip_iter)._pad = [0; 6];
        (*zip_iter).iters = iters;
        (*zip_iter).count = count;
    }

    obj
}

// ==================== Map Iterator ====================

/// Create a map iterator from a function and an iterator
/// captures: tuple of captured values (null for no captures)
/// capture_count: number of captures (0-4)
/// Returns: new iterator that applies func to each element
#[no_mangle]
pub extern "C" fn rt_map_new(
    func_ptr: i64,
    iter: *mut Obj,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    use crate::object::{IteratorKind, MapIterObj};

    let size = std::mem::size_of::<MapIterObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let map_iter = obj as *mut MapIterObj;
        (*map_iter).kind = IteratorKind::Map as u8;
        (*map_iter).exhausted = false;
        // Encoding (after §F.7c BigBang):
        //   bits 0-7  : capture count (low byte) — bit 7 is legacy needs_boxing
        //   bits 8-15 : elem_unbox_kind   (0=passthrough, 1=int, 2=bool)
        //   bits 16-23: result_box_kind   (0=passthrough, 1=int, 2=bool)
        (*map_iter).capture_count = capture_count as u8;
        (*map_iter).elem_unbox_kind = (capture_count >> 8) as u8;
        (*map_iter).result_box_kind = (capture_count >> 16) as u8;
        (*map_iter)._pad = [0; 3];
        (*map_iter).func_ptr = func_ptr;
        (*map_iter).inner_iter = iter;
        (*map_iter).captures = captures;
    }

    obj
}

// ==================== Filter Iterator ====================

/// Create a filter iterator from a predicate and an iterator
/// func_ptr: 0 for truthiness filtering (filter(None, ...)), otherwise predicate function pointer
/// captures: tuple of captured values (null for no captures)
/// capture_count: number of captures (0-4)
/// Returns: new iterator that yields elements where predicate returns true
#[no_mangle]
pub extern "C" fn rt_filter_new(
    func_ptr: i64,
    iter: *mut Obj,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    use crate::object::{FilterIterObj, IteratorKind};

    let size = std::mem::size_of::<FilterIterObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let filter_iter = obj as *mut FilterIterObj;
        (*filter_iter).kind = IteratorKind::Filter as u8;
        (*filter_iter).exhausted = false;
        (*filter_iter).capture_count = capture_count as u8;
        (*filter_iter).elem_unbox_kind = (capture_count >> 8) as u8;
        (*filter_iter)._pad = [0; 4];
        (*filter_iter).func_ptr = func_ptr;
        (*filter_iter).inner_iter = iter;
        (*filter_iter).captures = captures;
    }

    obj
}

// ==================== Chain Iterator ====================

/// Create a chain iterator from a list of iterators
/// iters: ListObj containing iterators
/// num_iters: number of iterators
/// Returns: new iterator that chains all iterators sequentially
#[no_mangle]
pub extern "C" fn rt_chain_new(iters: *mut Obj, num_iters: i64) -> *mut Obj {
    use crate::object::{ChainIterObj, IteratorKind};

    let size = std::mem::size_of::<ChainIterObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let chain_iter = obj as *mut ChainIterObj;
        (*chain_iter).kind = IteratorKind::Chain as u8;
        (*chain_iter).exhausted = false;
        (*chain_iter)._pad = [0; 6];
        (*chain_iter).iters = iters;
        (*chain_iter).current_idx = 0;
        (*chain_iter).num_iters = num_iters;
    }

    obj
}

// ==================== ISlice Iterator ====================

/// Create an islice iterator
/// iter: inner iterator
/// start: start index
/// stop: stop index (-1 for no stop)
/// step: step value (1 or more)
/// Returns: new iterator that yields selected elements
#[no_mangle]
pub extern "C" fn rt_islice_new(iter: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj {
    use crate::object::{ISliceIterObj, IteratorKind};

    let size = std::mem::size_of::<ISliceIterObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let islice_iter = obj as *mut ISliceIterObj;
        (*islice_iter).kind = IteratorKind::ISlice as u8;
        (*islice_iter).exhausted = false;
        (*islice_iter)._pad = [0; 6];
        (*islice_iter).inner_iter = iter;
        (*islice_iter).next_yield = start;
        (*islice_iter).stop = stop;
        (*islice_iter).step = step;
        (*islice_iter).current = 0;
    }

    obj
}
