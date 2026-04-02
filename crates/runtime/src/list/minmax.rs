//! List min/max operations

use crate::minmax_utils::{find_extremum_float, find_extremum_int};
use crate::object::{ListObj, Obj};
use crate::sorted::compare_key_values;

/// Generic list min/max for int and float elements.
/// is_min: 0=min, 1=max; elem_kind: 0=int, 1=float.
/// Returns i64 (for float, result is f64::to_bits()).
#[no_mangle]
pub extern "C" fn rt_list_minmax(list: *mut Obj, is_min: u8, elem_kind: u8) -> i64 {
    if list.is_null() {
        return 0;
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        if (*list_obj).len == 0 {
            let msg = if is_min == 0 {
                b"min() arg is an empty sequence" as &[u8]
            } else {
                b"max() arg is an empty sequence"
            };
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        let data = (*list_obj).data as *const usize;
        let len = (*list_obj).len;
        let want_min = is_min == 0;
        if elem_kind == 1 {
            find_extremum_float(data, len, want_min).to_bits() as i64
        } else {
            find_extremum_int(data, len, want_min)
        }
    }
}

/// Generic list min/max with key function.
/// is_min: 0=min, 1=max
#[no_mangle]
pub extern "C" fn rt_list_minmax_with_key(
    list: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
    is_min: u8,
) -> *mut Obj {
    unsafe {
        find_extremum_with_key(
            list,
            key_fn,
            elem_tag,
            captures,
            capture_count as u8,
            is_min == 0,
        )
    }
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
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: u8,
    is_min: bool,
) -> *mut Obj {
    use crate::object::ELEM_RAW_INT;

    if list.is_null() {
        return std::ptr::null_mut();
    }

    let list_obj = list as *mut ListObj;
    let len = (*list_obj).len;

    if len == 0 {
        let msg = if is_min {
            b"min() arg is an empty sequence"
        } else {
            b"max() arg is an empty sequence"
        };
        crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
    }

    let data = (*list_obj).data;

    // Apply key function to first element
    let mut extremum_elem = *data;
    let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
        crate::boxing::rt_box_int(extremum_elem as i64)
    } else {
        extremum_elem
    };
    let mut extremum_key = call_key_fn(key_fn, captures, capture_count, boxed_elem);

    // Compare remaining elements
    for i in 1..len {
        let elem = *data.add(i);
        let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
            crate::boxing::rt_box_int(elem as i64)
        } else {
            elem
        };
        let key = call_key_fn(key_fn, captures, capture_count, boxed_elem);

        let cmp = compare_key_values(key, extremum_key);
        let is_better = if is_min {
            cmp == std::cmp::Ordering::Less
        } else {
            cmp == std::cmp::Ordering::Greater
        };

        if is_better {
            extremum_elem = elem;
            extremum_key = key;
        }
    }

    extremum_elem // Return original element, not key!
}
