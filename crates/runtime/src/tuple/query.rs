//! Tuple query operations: index, count, min, max

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::object::{Obj, TypeTagKind};

/// Find index of value in tuple
/// Raises ValueError if not found
/// Returns: index (0-based)
#[no_mangle]
pub extern "C" fn rt_tuple_index(tuple: *mut Obj, value: *mut Obj) -> i64 {
    if tuple.is_null() {
        unsafe {
            let msg = b"tuple.index(x): x not in tuple";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
            if crate::ops::rt_obj_eq(elem, value) == 1 {
                return i as i64;
            }
        }

        // Not found - raise ValueError
        let msg = b"tuple.index(x): x not in tuple";
        crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
    }
}

/// Count occurrences of value in tuple
/// Returns: count
#[no_mangle]
pub extern "C" fn rt_tuple_count(tuple: *mut Obj, value: *mut Obj) -> i64 {
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
            if crate::ops::rt_obj_eq(elem, value) == 1 {
                count += 1;
            }
        }

        count
    }
}

/// Find minimum element in an integer tuple
#[no_mangle]
pub extern "C" fn rt_tuple_min_int(tuple: *mut Obj) -> i64 {
    use crate::minmax_utils::find_extremum_int;
    if tuple.is_null() {
        return 0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_int(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            true,
        )
    }
}

/// Find maximum element in an integer tuple
#[no_mangle]
pub extern "C" fn rt_tuple_max_int(tuple: *mut Obj) -> i64 {
    use crate::minmax_utils::find_extremum_int;
    if tuple.is_null() {
        return 0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_int(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            false,
        )
    }
}

/// Find minimum element in a float tuple
#[no_mangle]
pub extern "C" fn rt_tuple_min_float(tuple: *mut Obj) -> f64 {
    use crate::minmax_utils::find_extremum_float;
    if tuple.is_null() {
        return 0.0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_float(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            true,
        )
    }
}

/// Find maximum element in a float tuple
#[no_mangle]
pub extern "C" fn rt_tuple_max_float(tuple: *mut Obj) -> f64 {
    use crate::minmax_utils::find_extremum_float;
    if tuple.is_null() {
        return 0.0;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        if (*tuple_obj).len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
        find_extremum_float(
            (*tuple_obj).data.as_ptr() as *const usize,
            (*tuple_obj).len,
            false,
        )
    }
}

/// Find minimum element in a tuple with key function
#[no_mangle]
pub extern "C" fn rt_tuple_min_with_key(
    tuple: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    unsafe {
        find_tuple_extremum_with_key(tuple, key_fn, elem_tag, captures, capture_count as u8, true)
    }
}

/// Find maximum element in a tuple with key function
#[no_mangle]
pub extern "C" fn rt_tuple_max_with_key(
    tuple: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    unsafe {
        find_tuple_extremum_with_key(
            tuple,
            key_fn,
            elem_tag,
            captures,
            capture_count as u8,
            false,
        )
    }
}

/// Find extremum (min or max) element in a tuple using a key function
unsafe fn find_tuple_extremum_with_key(
    tuple: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: u8,
    is_min: bool,
) -> *mut Obj {
    use crate::iterator::call_map_with_captures;
    use crate::object::ELEM_RAW_INT;
    use crate::sorted::compare_key_values;

    if tuple.is_null() {
        return std::ptr::null_mut();
    }

    let tuple_obj = tuple as *mut crate::object::TupleObj;
    let len = (*tuple_obj).len;

    if len == 0 {
        let msg = if is_min {
            b"min() arg is an empty sequence"
        } else {
            b"max() arg is an empty sequence"
        };
        crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
    }

    let data = (*tuple_obj).data.as_ptr();

    // Apply key function to first element
    let mut extremum_elem = *data;
    let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
        crate::boxing::rt_box_int(extremum_elem as i64)
    } else {
        extremum_elem
    };
    let mut extremum_key = call_map_with_captures(key_fn, captures, capture_count, boxed_elem);

    // Compare remaining elements
    for i in 1..len {
        let elem = *data.add(i);
        let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
            crate::boxing::rt_box_int(elem as i64)
        } else {
            elem
        };
        let key = call_map_with_captures(key_fn, captures, capture_count, boxed_elem);

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
