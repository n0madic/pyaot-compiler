//! Dictionary iteration: keys, values, items

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::list::{rt_list_push, rt_make_list};
use crate::object::{DictObj, ListObj, Obj, TypeTagKind, ELEM_HEAP_OBJ, ELEM_RAW_INT};
use crate::tuple::{rt_make_tuple, rt_tuple_set};

/// Get list of all keys in dictionary (insertion order)
/// elem_tag controls the result list's storage format:
///   0 (ELEM_HEAP_OBJ) — store as-is (keys are already *mut Obj)
///   1 (ELEM_RAW_INT) — unbox IntObj to raw i64
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_dict_keys(dict: *mut Obj, elem_tag: u8) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0, elem_tag);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_keys");
        let dict_obj = dict as *mut DictObj;
        let len = (*dict_obj).len;

        let keys_list = rt_make_list(len as i64, elem_tag);
        let list_obj = keys_list as *mut ListObj;

        // Protect keys_list from GC during iteration. Although the loop body
        // does not currently perform GC allocations, rooting it here is a
        // safety net against future changes and matches the pattern used
        // throughout the runtime.
        let mut roots: [*mut Obj; 1] = [keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate entries in insertion order
        let mut idx = 0usize;
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
                if elem_tag == ELEM_RAW_INT {
                    let raw_val = crate::boxing::rt_unbox_int(key);
                    *(*list_obj).data.add(idx) = pyaot_core_defs::Value::from_int(raw_val);
                } else {
                    *(*list_obj).data.add(idx) = pyaot_core_defs::Value::from_ptr(key);
                }
                idx += 1;
            }
        }
        (*list_obj).len = idx;

        gc_pop();
        keys_list
    }
}

/// Get list of all values in dictionary (insertion order)
/// elem_tag controls the result list's storage format:
///   0 (ELEM_HEAP_OBJ) — store as-is (boxed values)
///   1 (ELEM_RAW_INT) — unbox IntObj to raw i64
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_dict_values(dict: *mut Obj, elem_tag: u8) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0, elem_tag);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_values");
        let dict_obj = dict as *mut DictObj;
        let len = (*dict_obj).len;

        let values_list = rt_make_list(len as i64, elem_tag);
        let list_obj = values_list as *mut ListObj;

        // Protect values_list from GC during iteration. Although the loop body
        // does not currently perform GC allocations, rooting it here is a
        // safety net against future changes and matches the pattern used
        // throughout the runtime.
        let mut roots: [*mut Obj; 1] = [values_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate entries in insertion order
        let mut idx = 0usize;
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
                let value = (*entry).value;
                if elem_tag == ELEM_RAW_INT {
                    let raw_val = crate::boxing::rt_unbox_int(value);
                    *(*list_obj).data.add(idx) = pyaot_core_defs::Value::from_int(raw_val);
                } else {
                    *(*list_obj).data.add(idx) = pyaot_core_defs::Value::from_ptr(value);
                }
                idx += 1;
            }
        }
        (*list_obj).len = idx;

        gc_pop();
        values_list
    }
}

/// Get list of (key, value) tuples for all entries (insertion order)
/// Returns: pointer to new ListObj containing TupleObj elements
#[no_mangle]
pub extern "C" fn rt_dict_items(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_items");
        let dict_obj = dict as *mut DictObj;
        let len = (*dict_obj).len;

        let items_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

        // CRITICAL: Protect items_list from GC. rt_make_tuple and rt_list_push
        // both trigger GC allocations inside the loop, so items_list must be
        // rooted or it may be collected between iterations.
        //
        // The roots slot at index 1 is reserved for the per-iteration tuple so
        // that it is also reachable during the rt_list_push call that follows.
        let mut roots: [*mut Obj; 2] = [items_list, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate entries in insertion order
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
                // Root the tuple before rt_list_push, which may allocate
                let tuple = rt_make_tuple(2, ELEM_HEAP_OBJ);
                roots[1] = tuple;
                rt_tuple_set(tuple, 0, key);
                rt_tuple_set(tuple, 1, (*entry).value);
                rt_list_push(items_list, tuple);
                roots[1] = std::ptr::null_mut();
            }
        }

        gc_pop();
        items_list
    }
}
