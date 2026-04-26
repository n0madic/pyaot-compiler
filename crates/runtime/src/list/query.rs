//! List query operations: index, count, copy

use super::core::rt_make_list;
use crate::exceptions::ExceptionType;
use crate::hash_table_utils::eq_hashable_obj;
use crate::object::{ListObj, Obj};

/// Find first occurrence of value in list. After §F.7c: slots are tagged
/// Values; pass raw Value bits to `eq_hashable_obj` which dispatches on
/// `Value::tag()`. Lowering boxes the search value to match.
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
            let slot = *data.add(i);
            if eq_hashable_obj(slot.0 as *mut Obj, value) {
                return i as i64;
            }
        }

        -1
    }
}

/// Count occurrences of value in list (post-§F.7c uniform Value semantics).
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
            let slot = *data.add(i);
            if eq_hashable_obj(slot.0 as *mut Obj, value) {
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
        return rt_make_list(0);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;

        // Root the source list across rt_make_list (which calls gc_alloc) so that
        // a GC collection triggered during that call cannot free the source list.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let new_list = rt_make_list(len as i64);
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
        return rt_make_list(0);
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

        // Root both source lists across rt_make_list (which calls gc_alloc) so
        // that a GC collection triggered during that call cannot free them.
        let mut roots: [*mut Obj; 2] = [list1, list2];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let new_list = rt_make_list(total_len as i64);
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
