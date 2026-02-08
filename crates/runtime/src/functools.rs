use crate::iterator::rt_iter_next_internal;
use crate::object::{IteratorObj, Obj};
use crate::tuple::rt_tuple_get;

/// Function type for reduce: takes (accumulator, element), returns new accumulator
type ReduceFn = extern "C" fn(*mut Obj, *mut Obj) -> *mut Obj;
/// Reduce function with 1 capture
type ReduceFn1 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Reduce function with 2 captures
type ReduceFn2 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Reduce function with 3 captures
type ReduceFn3 = extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Reduce function with 4 captures
type ReduceFn4 =
    extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;

/// Helper to call reduce function with captures.
/// Captures are prepended to the argument list: func(c0, c1, ..., acc, elem)
unsafe fn call_reduce_with_captures(
    func_ptr: i64,
    captures: *mut Obj,
    capture_count: u8,
    acc: *mut Obj,
    elem: *mut Obj,
) -> *mut Obj {
    match capture_count {
        0 => {
            let func: ReduceFn = std::mem::transmute(func_ptr);
            func(acc, elem)
        }
        1 => {
            let func: ReduceFn1 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            func(c0, acc, elem)
        }
        2 => {
            let func: ReduceFn2 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            func(c0, c1, acc, elem)
        }
        3 => {
            let func: ReduceFn3 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            func(c0, c1, c2, acc, elem)
        }
        4 => {
            let func: ReduceFn4 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            func(c0, c1, c2, c3, acc, elem)
        }
        _ => panic!("reduce: unsupported capture count {}", capture_count),
    }
}

/// Runtime function for functools.reduce
///
/// Implements: reduce(function, iterable[, initial])
///
/// Apply a function of two arguments cumulatively to the items of an iterable,
/// from left to right, so as to reduce the iterable to a single value.
///
/// # Arguments
/// * `func_ptr` - Function pointer (takes 2 args: accumulator, element)
/// * `iter` - Iterator object
/// * `initial` - Initial value (ignored if has_initial == 0)
/// * `has_initial` - 1 if initial value is provided, 0 otherwise
/// * `captures` - Tuple of captured variables (null if capture_count == 0)
/// * `capture_count` - Number of captured variables
///
/// # Returns
/// The final accumulated value
///
/// # Panics
/// Raises TypeError if the iterable is empty and no initial value is provided
#[no_mangle]
pub unsafe extern "C" fn rt_reduce(
    func_ptr: i64,
    iter: *mut Obj,
    initial: *mut Obj,
    has_initial: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    let capture_count = capture_count as u8;

    // Initialize accumulator
    let mut acc = if has_initial == 0 {
        // No initial value provided, get first element from iterator
        let first_elem = rt_iter_next_internal(iter, false);
        let inner_iter = iter as *mut IteratorObj;
        if (*inner_iter).exhausted {
            // Iterator is empty and no initial value
            let msg = "reduce() of empty iterable with no initial value";
            crate::exceptions::rt_exc_raise(
                pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                msg.as_ptr(),
                msg.len(),
            );
        }
        first_elem
    } else {
        // Use provided initial value
        initial
    };

    // Iterate through remaining elements
    loop {
        let elem = rt_iter_next_internal(iter, false);
        let inner_iter = iter as *mut IteratorObj;
        if (*inner_iter).exhausted {
            // Iterator exhausted, return accumulated value
            return acc;
        }

        // Call reduction function: acc = func(acc, elem)
        acc = call_reduce_with_captures(func_ptr, captures, capture_count, acc, elem);
    }
}
