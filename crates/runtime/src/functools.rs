use crate::iterator::rt_iter_next_internal;
use crate::object::{IteratorObj, Obj};
use crate::tuple::rt_tuple_get;
use pyaot_core_defs::Value;

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
/// Reduce function with 5 captures
type ReduceFn5 =
    extern "C" fn(*mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj, *mut Obj) -> *mut Obj;
/// Reduce function with 6 captures
type ReduceFn6 = extern "C" fn(
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
    *mut Obj,
) -> *mut Obj;
/// Reduce function with 7 captures
type ReduceFn7 = extern "C" fn(
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
/// Reduce function with 8 captures
type ReduceFn8 = extern "C" fn(
    *mut Obj,
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
        5 => {
            let func: ReduceFn5 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            func(c0, c1, c2, c3, c4, acc, elem)
        }
        6 => {
            let func: ReduceFn6 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            func(c0, c1, c2, c3, c4, c5, acc, elem)
        }
        7 => {
            let func: ReduceFn7 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let c6 = rt_tuple_get(captures, 6);
            func(c0, c1, c2, c3, c4, c5, c6, acc, elem)
        }
        8 => {
            let func: ReduceFn8 = std::mem::transmute(func_ptr);
            let c0 = rt_tuple_get(captures, 0);
            let c1 = rt_tuple_get(captures, 1);
            let c2 = rt_tuple_get(captures, 2);
            let c3 = rt_tuple_get(captures, 3);
            let c4 = rt_tuple_get(captures, 4);
            let c5 = rt_tuple_get(captures, 5);
            let c6 = rt_tuple_get(captures, 6);
            let c7 = rt_tuple_get(captures, 7);
            func(c0, c1, c2, c3, c4, c5, c6, c7, acc, elem)
        }
        _ => {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "reduce: unsupported capture count (max 8)"
            );
        }
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
pub unsafe fn rt_reduce(
    func_ptr: i64,
    iter: *mut Obj,
    initial: *mut Obj,
    has_initial: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    // Encoding (after §F.7c BigBang):
    //   bits 0-7 : capture count (legacy bit 7 is needs_boxing — unused)
    //   bits 8-15: elem_unbox_kind (0=passthrough, 1=int, 2=bool)
    let cc_byte = capture_count as u8;
    let elem_unbox_kind = (capture_count >> 8) as u8;
    let unbox = |v: *mut Obj| -> *mut Obj {
        match elem_unbox_kind {
            1 => pyaot_core_defs::Value(v as u64).unwrap_int() as *mut Obj,
            2 => i64::from(pyaot_core_defs::Value(v as u64).unwrap_bool()) as *mut Obj,
            _ => v,
        }
    };

    // Initialize accumulator
    let mut acc = if has_initial == 0 {
        // No initial value provided, get first element from iterator
        let first_elem = rt_iter_next_internal(iter, false);
        let inner_iter = iter as *mut IteratorObj;
        if (*inner_iter).exhausted {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "reduce() of empty iterable with no initial value"
            );
        }
        unbox(first_elem)
    } else {
        // Initial value comes raw from the caller — no unbox.
        initial
    };

    // Iterate through remaining elements
    loop {
        let elem = rt_iter_next_internal(iter, false);
        let inner_iter = iter as *mut IteratorObj;
        if (*inner_iter).exhausted {
            return acc;
        }
        // Call reduction function: acc = func(acc, elem)
        acc = call_reduce_with_captures(func_ptr, captures, cc_byte, acc, unbox(elem));
    }
}
#[export_name = "rt_reduce"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_reduce_abi(
    func_ptr: i64,
    iter: Value,
    initial: Value,
    has_initial: i64,
    captures: Value,
    capture_count: i64,
) -> Value {
    Value::from_ptr(unsafe {
        rt_reduce(
            func_ptr,
            iter.unwrap_ptr(),
            initial.unwrap_ptr(),
            has_initial,
            captures.unwrap_ptr(),
            capture_count,
        )
    })
}

/// Phase 4+ Extension E2b: tagged-delivery variant of `rt_reduce`.
/// Asymmetric semantics:
///   - INPUT accumulator and element flow through to the callback
///     verbatim — callback's prologue does its own `UnboxValue` for
///     primitive-typed params (Step E1).
///   - OUTPUT (the callback's return, which becomes the next iteration's
///     accumulator and the final result) is re-wrapped according to
///     `result_box_kind` encoded in `capture_count` bits 16-23. Lambdas
///     are not return-ABI flipped today, so primitive-typed returns
///     come back as raw bits; the re-wrap keeps `acc` uniformly tagged
///     across iterations and final result.
///
/// Encoding (same shape as `rt_map_new` tagged variant):
///   bits 0-7  : actual capture count (low 7 bits consumed)
///   bits 16-23: result_box_kind (0=passthrough, 1=int, 2=bool)
///
/// # Safety
/// Same constraints as `rt_reduce`: callers must pass valid iterator
/// and tuple pointers.
pub unsafe fn rt_reduce_tagged(
    func_ptr: i64,
    iter: *mut Obj,
    initial: *mut Obj,
    has_initial: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    let cc_byte = (capture_count as u8) & 0x7F;
    let result_box_kind = (capture_count >> 16) as u8;

    let rewrap = |v: *mut Obj| -> *mut Obj {
        match result_box_kind {
            1 => pyaot_core_defs::Value::from_int(v as i64).0 as *mut Obj,
            2 => pyaot_core_defs::Value::from_bool((v as i64) != 0).0 as *mut Obj,
            _ => v,
        }
    };

    let mut acc = if has_initial == 0 {
        let first_elem = rt_iter_next_internal(iter, false);
        let inner_iter = iter as *mut IteratorObj;
        if (*inner_iter).exhausted {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "reduce() of empty iterable with no initial value"
            );
        }
        // Initial accumulator comes directly from the inner iterator —
        // already a tagged Value, no rewrap needed.
        first_elem
    } else {
        // Caller-provided initial arrives as raw primitive bits in
        // `initial` (the ABI shim's `Value::unwrap_ptr` strips the tag).
        // Wrap it into a tagged Value so the callback's prologue
        // `UnboxValue` reads matching bit shape on the first iteration.
        rewrap(initial)
    };

    loop {
        let elem = rt_iter_next_internal(iter, false);
        let inner_iter = iter as *mut IteratorObj;
        if (*inner_iter).exhausted {
            // Final return: unwrap the tagged accumulator back to the
            // caller's primitive shape. The caller's dest local was
            // allocated with the callback's declared return type
            // (Int / Bool / heap), so we mirror the unwrap of the
            // legacy reduce that returns raw bits directly.
            return match result_box_kind {
                1 => pyaot_core_defs::Value(acc as u64).unwrap_int() as *mut Obj,
                2 => i64::from(pyaot_core_defs::Value(acc as u64).unwrap_bool()) as *mut Obj,
                _ => acc,
            };
        }
        // Callback returns raw primitive for primitive-typed lambdas;
        // re-wrap before the next iteration so the callback's tagged
        // ABI contract for `acc` is honoured.
        let raw_result = call_reduce_with_captures(func_ptr, captures, cc_byte, acc, elem);
        acc = rewrap(raw_result);
    }
}
#[export_name = "rt_reduce_tagged"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_reduce_tagged_abi(
    func_ptr: i64,
    iter: Value,
    initial: Value,
    has_initial: i64,
    captures: Value,
    capture_count: i64,
) -> Value {
    Value::from_ptr(unsafe {
        rt_reduce_tagged(
            func_ptr,
            iter.unwrap_ptr(),
            initial.unwrap_ptr(),
            has_initial,
            captures.unwrap_ptr(),
            capture_count,
        )
    })
}
