//! Dictionary iteration: keys, values, items

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::list::{rt_list_push, rt_make_list};
use crate::object::{DictObj, ListObj, Obj, TypeTagKind};
use crate::tuple::{rt_make_tuple, rt_tuple_set};
use pyaot_core_defs::Value;

/// Get list of all keys in dictionary (insertion order).
/// After §F.7c: result list stores uniform tagged Values; no elem_tag arg.
pub fn rt_dict_keys(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_keys");
        let dict_obj = dict as *mut DictObj;
        let keys_list = rt_make_list((*dict_obj).len as i64);
        let list_obj = keys_list as *mut ListObj;

        let mut roots: [*mut Obj; 1] = [keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let mut idx = 0usize;
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if key.0 != 0 {
                *(*list_obj).data.add(idx) = key;
                idx += 1;
            }
        }
        (*list_obj).len = idx;

        gc_pop();
        keys_list
    }
}
#[export_name = "rt_dict_keys"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_keys_abi(dict: Value) -> Value {
    Value::from_ptr(rt_dict_keys(dict.unwrap_ptr()))
}

/// Get list of all values in dictionary (insertion order).
/// After §F.7c: result list stores uniform tagged Values; no elem_tag arg.
pub fn rt_dict_values(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_values");
        let dict_obj = dict as *mut DictObj;
        let values_list = rt_make_list((*dict_obj).len as i64);
        let list_obj = values_list as *mut ListObj;

        let mut roots: [*mut Obj; 1] = [values_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let mut idx = 0usize;
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if key.0 != 0 {
                *(*list_obj).data.add(idx) = (*entry).value;
                idx += 1;
            }
        }
        (*list_obj).len = idx;

        gc_pop();
        values_list
    }
}
#[export_name = "rt_dict_values"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_values_abi(dict: Value) -> Value {
    Value::from_ptr(rt_dict_values(dict.unwrap_ptr()))
}

/// Get list of (key, value) tuples for all entries (insertion order).
pub fn rt_dict_items(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_items");
        let dict_obj = dict as *mut DictObj;

        let items_list = rt_make_list((*dict_obj).len as i64);

        // CRITICAL: Protect items_list from GC. rt_make_tuple and rt_list_push
        // both trigger GC allocations inside the loop, so items_list must be
        // rooted or it may be collected between iterations.
        let mut roots: [*mut Obj; 2] = [items_list, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if key.0 != 0 {
                let tuple = rt_make_tuple(2);
                roots[1] = tuple;
                rt_tuple_set(tuple, 0, key.0 as *mut Obj);
                rt_tuple_set(tuple, 1, (*entry).value.0 as *mut Obj);
                rt_list_push(items_list, tuple);
                roots[1] = std::ptr::null_mut();
            }
        }

        gc_pop();
        items_list
    }
}
#[export_name = "rt_dict_items"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_items_abi(dict: Value) -> Value {
    Value::from_ptr(rt_dict_items(dict.unwrap_ptr()))
}
