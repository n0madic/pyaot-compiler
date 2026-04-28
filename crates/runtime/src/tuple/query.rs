//! Tuple query operations: index, count, min, max

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::object::{Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Find index of value in tuple
/// Raises ValueError if not found
/// Returns: index (0-based)
pub fn rt_tuple_index(tuple: *mut Obj, value: *mut Obj) -> i64 {
    if tuple.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "tuple.index(x): x not in tuple"
            );
        }
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_index");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len;
        let data = (*tuple_obj).data.as_ptr();

        // Search for value using value equality
        for i in 0..len {
            let elem = *data.add(i);
            if crate::ops::rt_obj_eq(elem.0 as *mut Obj, value) == 1 {
                return i as i64;
            }
        }

        // Not found - raise ValueError
        raise_exc!(
            crate::exceptions::ExceptionType::ValueError,
            "tuple.index(x): x not in tuple"
        );
    }
}
#[export_name = "rt_tuple_index"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_index_abi(tuple: Value, value: Value) -> i64 {
    rt_tuple_index(tuple.unwrap_ptr(), value.unwrap_ptr())
}

/// Count occurrences of value in tuple
/// Returns: count
pub fn rt_tuple_count(tuple: *mut Obj, value: *mut Obj) -> i64 {
    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_count");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len;
        let data = (*tuple_obj).data.as_ptr();

        let mut count = 0i64;
        // Count occurrences using value equality
        for i in 0..len {
            let elem = *data.add(i);
            if crate::ops::rt_obj_eq(elem.0 as *mut Obj, value) == 1 {
                count += 1;
            }
        }

        count
    }
}
#[export_name = "rt_tuple_count"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_count_abi(tuple: Value, value: Value) -> i64 {
    rt_tuple_count(tuple.unwrap_ptr(), value.unwrap_ptr())
}

/// Generic tuple min/max for int and float elements.
/// is_min: 0=min, 1=max; elem_kind: 0=int, 1=float.
/// Returns i64 (for float, result is f64::to_bits()).
pub fn rt_tuple_minmax(tuple: *mut Obj, is_min: u8, elem_kind: u8) -> i64 {
    use crate::minmax_utils::{find_extremum_float, find_extremum_int};
    if tuple.is_null() {
        return 0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            if is_min == 0 {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "min() arg is an empty sequence"
                );
            } else {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "max() arg is an empty sequence"
                );
            }
        }
        let data = (*tuple_obj).data.as_ptr() as *const usize;
        let len = (*tuple_obj).len;
        let want_min = is_min == 0;
        if elem_kind == 1 {
            find_extremum_float(data, len, want_min).to_bits() as i64
        } else {
            // Slots store tagged Values; find_extremum_int preserves ordering
            // (sign-extending shift), but returns tagged bits — unwrap to raw int.
            pyaot_core_defs::Value(find_extremum_int(data, len, want_min) as u64).unwrap_int()
        }
    }
}
#[export_name = "rt_tuple_minmax"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_minmax_abi(tuple: Value, is_min: u8, elem_kind: u8) -> i64 {
    rt_tuple_minmax(tuple.unwrap_ptr(), is_min, elem_kind)
}

/// Generic tuple min/max with key function.
/// `key_return_tag`: 0=heap, 1=Int(raw i64), 2=Bool(raw 0/1).
pub fn rt_tuple_minmax_with_key(
    tuple: *mut Obj,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    is_min: u8,
    key_return_tag: u8,
) -> *mut Obj {
    unsafe {
        find_tuple_extremum_with_key(
            tuple,
            key_fn,
            captures,
            capture_count as u8,
            is_min == 0,
            key_return_tag,
        )
    }
}
#[export_name = "rt_tuple_minmax_with_key"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_minmax_with_key_abi(
    tuple: Value,
    key_fn: i64,
    captures: Value,
    capture_count: i64,
    is_min: u8,
    key_return_tag: u8,
) -> Value {
    Value::from_ptr(rt_tuple_minmax_with_key(
        tuple.unwrap_ptr(),
        key_fn,
        captures.unwrap_ptr(),
        capture_count,
        is_min,
        key_return_tag,
    ))
}

/// Find extremum (min or max) element in a tuple using a key function.
unsafe fn find_tuple_extremum_with_key(
    tuple: *mut Obj,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: u8,
    is_min: bool,
    key_return_tag: u8,
) -> *mut Obj {
    use crate::iterator::call_map_with_captures;
    use crate::sorted::{compare_key_values, unwrap_slot_for_key_fn, wrap_key_result};

    if tuple.is_null() {
        return std::ptr::null_mut();
    }

    let tuple_obj = tuple as *mut crate::object::TupleObj;
    let len = (*tuple_obj).len;

    if len == 0 {
        if is_min {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "min() arg is an empty sequence"
            );
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "max() arg is an empty sequence"
            );
        }
    }

    let data = (*tuple_obj).data.as_ptr();

    let mut extremum_slot = *data;
    let mut extremum_key = wrap_key_result(
        call_map_with_captures(
            key_fn,
            captures,
            capture_count,
            unwrap_slot_for_key_fn(extremum_slot, key_return_tag),
        ),
        key_return_tag,
    );

    for i in 1..len {
        let slot = *data.add(i);
        let key = wrap_key_result(
            call_map_with_captures(
                key_fn,
                captures,
                capture_count,
                unwrap_slot_for_key_fn(slot, key_return_tag),
            ),
            key_return_tag,
        );

        let cmp = compare_key_values(key.0 as *mut Obj, extremum_key.0 as *mut Obj);
        let is_better = if is_min {
            cmp == std::cmp::Ordering::Less
        } else {
            cmp == std::cmp::Ordering::Greater
        };

        if is_better {
            extremum_slot = slot;
            extremum_key = key;
        }
    }

    if extremum_slot.is_int() {
        extremum_slot.unwrap_int() as *mut Obj
    } else if extremum_slot.is_bool() {
        i64::from(extremum_slot.unwrap_bool()) as *mut Obj
    } else {
        extremum_slot.0 as *mut Obj
    }
}
