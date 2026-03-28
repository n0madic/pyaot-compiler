//! List min/max operations for integer and float lists

use crate::minmax_utils::{find_extremum_float, find_extremum_int};
use crate::object::{ListObj, Obj};
use crate::sorted::compare_key_values;

/// Find minimum element in an integer list
#[no_mangle]
pub extern "C" fn rt_list_min_int(list: *mut Obj) -> i64 {
    if list.is_null() {
        return 0;
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        if (*list_obj).len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_int((*list_obj).data as *const usize, (*list_obj).len, true)
    }
}

/// Find maximum element in an integer list
#[no_mangle]
pub extern "C" fn rt_list_max_int(list: *mut Obj) -> i64 {
    if list.is_null() {
        return 0;
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        if (*list_obj).len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_int((*list_obj).data as *const usize, (*list_obj).len, false)
    }
}

/// Find minimum element in a float list
#[no_mangle]
pub extern "C" fn rt_list_min_float(list: *mut Obj) -> f64 {
    if list.is_null() {
        return 0.0;
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        if (*list_obj).len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_float((*list_obj).data as *const usize, (*list_obj).len, true)
    }
}

/// Find maximum element in a float list
#[no_mangle]
pub extern "C" fn rt_list_max_float(list: *mut Obj) -> f64 {
    if list.is_null() {
        return 0.0;
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        if (*list_obj).len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_float((*list_obj).data as *const usize, (*list_obj).len, false)
    }
}

/// Find minimum element in a list with key function
/// key_fn: function pointer for key extraction
/// elem_tag: element storage type (0=ELEM_HEAP_OBJ, 1=ELEM_RAW_INT, 2=ELEM_RAW_BOOL)
/// captures: tuple of captured variables (null if no captures)
/// capture_count: number of captured variables
#[no_mangle]
pub extern "C" fn rt_list_min_with_key(
    list: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    unsafe { find_extremum_with_key(list, key_fn, elem_tag, captures, capture_count as u8, true) }
}

/// Find maximum element in a list with key function
#[no_mangle]
pub extern "C" fn rt_list_max_with_key(
    list: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    unsafe { find_extremum_with_key(list, key_fn, elem_tag, captures, capture_count as u8, false) }
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
