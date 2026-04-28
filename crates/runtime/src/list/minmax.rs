//! List min/max operations

use crate::minmax_utils::find_extremum_float;
use crate::object::{ListObj, Obj};
use crate::sorted::compare_key_values;
use pyaot_core_defs::Value;

/// Generic list min/max for int and float elements.
/// is_min: 0=min, 1=max; elem_kind: 0=int, 1=float.
/// Returns i64 (for float, result is f64::to_bits()).
pub fn rt_list_minmax(list: *mut Obj, is_min: u8, elem_kind: u8) -> i64 {
    if list.is_null() {
        return 0;
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        if (*list_obj).len == 0 {
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
        let data = (*list_obj).data;
        let len = (*list_obj).len;
        let want_min = is_min == 0;
        if elem_kind == 1 {
            // Float: heap-boxed FloatObj pointers; find_extremum_float works unchanged.
            find_extremum_float(data as *const usize, len, want_min).to_bits() as i64
        } else {
            // Int: dispatch on Value::tag() to extract the raw i64.
            let load_int = |slot: Value| -> i64 {
                if slot.is_int() {
                    slot.unwrap_int()
                } else if slot.is_bool() {
                    i64::from(slot.unwrap_bool())
                } else {
                    slot.0 as i64
                }
            };
            if len == 0 {
                return 0;
            }
            let mut extremum = load_int(*data);
            for i in 1..len {
                let val = load_int(*data.add(i));
                if want_min {
                    if val < extremum {
                        extremum = val;
                    }
                } else if val > extremum {
                    extremum = val;
                }
            }
            extremum
        }
    }
}
#[export_name = "rt_list_minmax"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_minmax_abi(list: Value, is_min: u8, elem_kind: u8) -> i64 {
    rt_list_minmax(list.unwrap_ptr(), is_min, elem_kind)
}


/// Generic list min/max with key function.
/// `key_return_tag`: 0=heap, 1=Int(raw i64), 2=Bool(raw 0/1).
pub fn rt_list_minmax_with_key(
    list: *mut Obj,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    is_min: u8,
    key_return_tag: u8,
) -> *mut Obj {
    unsafe {
        find_extremum_with_key(
            list,
            key_fn,
            captures,
            capture_count as u8,
            is_min == 0,
            key_return_tag,
        )
    }
}
#[export_name = "rt_list_minmax_with_key"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_minmax_with_key_abi(
    list: Value,
    key_fn: i64,
    captures: Value,
    capture_count: i64,
    is_min: u8,
    key_return_tag: u8,
) -> Value {
    Value::from_ptr(rt_list_minmax_with_key(list.unwrap_ptr(), key_fn, captures.unwrap_ptr(), capture_count, is_min, key_return_tag))
}


/// Call key function with captures support
unsafe fn call_key_fn(
    key_fn: i64,
    captures: *mut Obj,
    capture_count: u8,
    elem: *mut Obj,
) -> *mut Obj {
    crate::iterator::call_map_with_captures(key_fn, captures, capture_count, elem)
}

/// Find extremum (min or max) element in a list using a key function
unsafe fn find_extremum_with_key(
    list: *mut Obj,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: u8,
    is_min: bool,
    key_return_tag: u8,
) -> *mut Obj {
    use crate::sorted::{unwrap_slot_for_key_fn, wrap_key_result};

    if list.is_null() {
        return std::ptr::null_mut();
    }

    let list_obj = list as *mut ListObj;
    let len = (*list_obj).len;

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

    let data = (*list_obj).data;

    let mut extremum_slot = *data;
    let mut extremum_key = wrap_key_result(
        call_key_fn(
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
            call_key_fn(
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
