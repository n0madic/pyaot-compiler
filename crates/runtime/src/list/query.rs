//! List query operations: index, count, copy

use super::core::rt_make_list;
use crate::exceptions::ExceptionType;
use crate::hash_table_utils::eq_hashable_obj;
use crate::object::{ListObj, Obj};
use pyaot_core_defs::Value;

/// Find first occurrence of value in list. After §F.7c: slots are tagged
/// Values; pass raw Value bits to `eq_hashable_obj` which dispatches on
/// `Value::tag()`. Lowering boxes the search value to match.
pub fn rt_list_index(list: *mut Obj, value: *mut Obj) -> i64 {
    if list.is_null() {
        return -1;
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let data = (*list_obj).data;

        if !data.is_null() {
            for i in 0..len {
                let slot = *data.add(i);
                if eq_hashable_obj(slot.0 as *mut Obj, value) {
                    return i as i64;
                }
            }
        }

        // `list.index(x)` raises `ValueError` when `x` is absent — CPython
        // semantics. Returning a `-1` sentinel (the old behaviour) silently
        // produced a wrong index downstream. This is `rt_list_index`'s only
        // caller (`lst.index`), so raising is unambiguously correct here.
        crate::raise_exc!(ExceptionType::ValueError, "list.index(x): x not in list");
    }
}
#[export_name = "rt_list_index"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_index_abi(list: Value, value: Value) -> i64 {
    // `value` is the search element, possibly a tagged immediate (int/bool/None);
    // pass raw bits so the tag survives instead of tripping `unwrap_ptr`'s debug
    // `is_ptr` assertion. The internal element comparison handles tagged values.
    rt_list_index(list.unwrap_ptr(), value.0 as *mut Obj)
}

/// Count occurrences of value in list (post-§F.7c uniform Value semantics).
pub fn rt_list_count(list: *mut Obj, value: *mut Obj) -> i64 {
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
#[export_name = "rt_list_count"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_count_abi(list: Value, value: Value) -> i64 {
    // `value` may be a tagged immediate; pass raw bits (see `rt_list_index_abi`).
    rt_list_count(list.unwrap_ptr(), value.0 as *mut Obj)
}

/// Create a shallow copy of list
/// Returns: pointer to new allocated ListObj
pub fn rt_list_copy(list: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_list_copy"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_copy_abi(list: Value) -> Value {
    Value::from_ptr(rt_list_copy(list.unwrap_ptr()))
}

/// Concatenate two lists into a new list: list1 + list2
pub fn rt_list_concat(list1: *mut Obj, list2: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_list_concat"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_concat_abi(list1: Value, list2: Value) -> Value {
    Value::from_ptr(rt_list_concat(list1.unwrap_ptr(), list2.unwrap_ptr()))
}

/// Repeat a list `count` times (Python's `list * int` / `int * list`).
/// Negative or zero counts produce an empty list (CPython behaviour).
/// Each repeated slot copies the same `Value` bits — for heap-pointer
/// elements this matches CPython's "shallow repetition" semantics where
/// every slot references the same underlying object.
pub fn rt_list_repeat(list: *mut Obj, count: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if list.is_null() || count <= 0 {
        return rt_make_list(0);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;
        if len == 0 {
            return rt_make_list(0);
        }

        let count_usize = count as usize;
        let total_len = match len.checked_mul(count_usize) {
            Some(l) => l,
            None => {
                raise_exc!(ExceptionType::OverflowError, "list repetition too long");
            }
        };

        // Root the source list across rt_make_list (which calls gc_alloc)
        // so a GC collection during that call cannot free it.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let new_list = rt_make_list(total_len as i64);
        let new_list_obj = new_list as *mut ListObj;

        gc_pop();

        let src_data = (*src).data;
        let dst_data = (*new_list_obj).data;
        for i in 0..count_usize {
            std::ptr::copy_nonoverlapping(src_data, dst_data.add(i * len), len);
        }

        (*new_list_obj).len = total_len;
        new_list
    }
}
#[export_name = "rt_list_repeat"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_repeat_abi(list: Value, count: i64) -> Value {
    Value::from_ptr(rt_list_repeat(list.unwrap_ptr(), count))
}
