//! List query operations: index, count, copy

use super::core::rt_make_list;
use crate::object::{ListObj, Obj};

/// Find first occurrence of value in list
/// Uses pointer equality for comparison
/// Returns: index of first occurrence, or -1 if not found
#[no_mangle]
pub extern "C" fn rt_list_index(list: *mut Obj, value: *mut Obj) -> i64 {
    if list.is_null() {
        return -1;
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let data = (*list_obj).data;

        if data.is_null() {
            return -1;
        }

        for i in 0..len {
            if *data.add(i) == value {
                return i as i64;
            }
        }

        -1
    }
}

/// Count occurrences of value in list
/// Uses pointer equality for comparison
/// Returns: count of occurrences
#[no_mangle]
pub extern "C" fn rt_list_count(list: *mut Obj, value: *mut Obj) -> i64 {
    if list.is_null() {
        return 0;
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let data = (*list_obj).data;

        if data.is_null() {
            return 0;
        }

        let mut count = 0i64;
        for i in 0..len {
            if *data.add(i) == value {
                count += 1;
            }
        }

        count
    }
}

/// Create a shallow copy of list
/// Returns: pointer to new allocated ListObj
#[no_mangle]
pub extern "C" fn rt_list_copy(list: *mut Obj) -> *mut Obj {
    if list.is_null() {
        return rt_make_list(0, crate::object::ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;

        let new_list = rt_make_list(len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        if len > 0 {
            let src_data = (*src).data;
            let dst_data = (*new_list_obj).data;

            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
            (*new_list_obj).len = len;
        }

        new_list
    }
}

/// Concatenate two lists into a new list: list1 + list2
#[no_mangle]
pub extern "C" fn rt_list_concat(list1: *mut Obj, list2: *mut Obj) -> *mut Obj {
    if list1.is_null() && list2.is_null() {
        return rt_make_list(0, crate::object::ELEM_HEAP_OBJ);
    }
    if list1.is_null() {
        return rt_list_copy(list2);
    }
    if list2.is_null() {
        return rt_list_copy(list1);
    }

    unsafe {
        let src1 = list1 as *mut ListObj;
        let src2 = list2 as *mut ListObj;
        let len1 = (*src1).len;
        let len2 = (*src2).len;
        let total_len = len1 + len2;

        let elem_tag = if len1 > 0 {
            (*src1).elem_tag
        } else {
            (*src2).elem_tag
        };

        let new_list = rt_make_list(total_len as i64, elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        if len1 > 0 {
            let src_data = (*src1).data;
            let dst_data = (*new_list_obj).data;
            std::ptr::copy_nonoverlapping(src_data, dst_data, len1);
        }

        if len2 > 0 {
            let src_data = (*src2).data;
            let dst_data = (*new_list_obj).data;
            std::ptr::copy_nonoverlapping(src_data, dst_data.add(len1), len2);
        }

        (*new_list_obj).len = total_len;
        new_list
    }
}
