//! List query operations: index, count, copy

use super::core::rt_make_list;
use crate::exceptions::ExceptionType;
use crate::hash_table_utils::eq_hashable_obj;
use crate::object::{ListObj, Obj, ELEM_HEAP_OBJ};

/// Compare two list elements for equality.
/// For heap objects, uses value equality (eq_hashable_obj).
/// For raw values (int, bool), uses bitwise equality.
#[inline]
unsafe fn elem_eq(a: *mut Obj, b: *mut Obj, elem_tag: u8) -> bool {
    if elem_tag == ELEM_HEAP_OBJ {
        eq_hashable_obj(a, b)
    } else {
        // Raw int/bool: compare as raw bits
        a == b
    }
}

/// Find first occurrence of value in list
/// Uses value equality for heap objects, raw equality for primitives
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
        let elem_tag = (*list_obj).elem_tag;

        if data.is_null() {
            return -1;
        }

        for i in 0..len {
            if elem_eq(*data.add(i), value, elem_tag) {
                return i as i64;
            }
        }

        -1
    }
}

/// Count occurrences of value in list
/// Uses value equality for heap objects, raw equality for primitives
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
        let elem_tag = (*list_obj).elem_tag;

        if data.is_null() {
            return 0;
        }

        let mut count = 0i64;
        for i in 0..len {
            if elem_eq(*data.add(i), value, elem_tag) {
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
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if list.is_null() {
        return rt_make_list(0, crate::object::ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;
        let elem_tag = (*src).elem_tag;

        // Root the source list across rt_make_list (which calls gc_alloc) so that
        // a GC collection triggered during that call cannot free the source list.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let new_list = rt_make_list(len as i64, elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        gc_pop();

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
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

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
        let total_len = match len1.checked_add(len2) {
            Some(l) => l,
            None => {
                raise_exc!(ExceptionType::OverflowError, "list concatenation too long");
            }
        };

        let elem_tag = if len1 > 0 {
            (*src1).elem_tag
        } else {
            (*src2).elem_tag
        };

        // Root both source lists across rt_make_list (which calls gc_alloc) so
        // that a GC collection triggered during that call cannot free them.
        let mut roots: [*mut Obj; 2] = [list1, list2];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let new_list = rt_make_list(total_len as i64, elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        gc_pop();

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
