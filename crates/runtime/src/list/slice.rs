//! List slicing operations

use super::core::rt_make_list;
use crate::object::{ListObj, Obj, ELEM_HEAP_OBJ};
use crate::slice_utils::{collect_step_indices, normalize_slice_indices, slice_length};

/// Slice a list: list[start:end]
/// Negative indices are supported (counted from end)
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated ListObj (shallow copy)
#[no_mangle]
pub extern "C" fn rt_list_slice(list: *mut Obj, start: i64, end: i64) -> *mut Obj {
    if list.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility (step=1 for simple slice)
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);

        // Create new list with same elem_tag
        let new_list = rt_make_list(slice_len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        if slice_len > 0 {
            let src_data = (*src).data;
            let dst_data = (*new_list_obj).data;

            // Copy element pointers (shallow copy)
            for i in 0..slice_len {
                *dst_data.add(i) = *src_data.add(start as usize + i);
            }
            (*new_list_obj).len = slice_len;
        }

        new_list
    }
}

/// Slice a list with step: list[start:end:step]
/// Uses i64::MIN as sentinel for "default start" and i64::MAX for "default end"
/// Defaults depend on step direction:
///   - Positive step: start=0, end=len
///   - Negative step: start=len-1, end=-1 (before index 0)
///
/// Returns: pointer to new allocated ListObj (shallow copy)
#[no_mangle]
pub extern "C" fn rt_list_slice_step(list: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj {
    if list.is_null() || step == 0 {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility
        let (start, end) = normalize_slice_indices(start, end, len, step);

        // Collect indices using shared utility
        let indices = collect_step_indices(start, end, step);
        let result_len = indices.len();
        let new_list = rt_make_list(result_len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        if result_len > 0 {
            let src_data = (*src).data;
            let dst_data = (*new_list_obj).data;

            for (dst_i, src_i) in indices.iter().enumerate() {
                *dst_data.add(dst_i) = *src_data.add(*src_i);
            }
            (*new_list_obj).len = result_len;
        }

        new_list
    }
}
