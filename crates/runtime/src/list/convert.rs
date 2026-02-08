//! List conversion operations: create lists from other types

use super::core::{rt_list_push, rt_make_list};
use crate::object::{ListObj, Obj, StrObj, TupleObj, ELEM_HEAP_OBJ, ELEM_RAW_INT};

/// Create a list from a tuple
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_list_from_tuple(tuple: *mut Obj) -> *mut Obj {
    if tuple.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let tuple_obj = tuple as *mut TupleObj;
        let len = (*tuple_obj).len;
        let elem_tag = (*tuple_obj).elem_tag;

        let list = rt_make_list(len as i64, elem_tag);
        let list_obj = list as *mut ListObj;

        if len > 0 {
            let src_data = (*tuple_obj).data.as_ptr();
            let dst_data = (*list_obj).data;

            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
            (*list_obj).len = len;
        }

        list
    }
}

/// Create a list from a string (each character becomes an element)
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_list_from_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::string::rt_make_str;

    if str_obj.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let str = str_obj as *mut StrObj;
        let len = (*str).len;
        let data = (*str).data.as_ptr();

        let list = rt_make_list(len as i64, ELEM_HEAP_OBJ);
        let list_obj = list as *mut ListObj;

        for i in 0..len {
            let ch = *data.add(i);
            // Create single-character string
            let char_str = rt_make_str(&ch, 1);
            *(*list_obj).data.add(i) = char_str;
        }
        (*list_obj).len = len;

        list
    }
}

/// Create a list from a range
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_list_from_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    if step == 0 {
        return rt_make_list(0, ELEM_RAW_INT);
    }

    let len = if step > 0 {
        if stop > start {
            ((stop - start + step - 1) / step) as usize
        } else {
            0
        }
    } else if start > stop {
        ((start - stop - step - 1) / (-step)) as usize
    } else {
        0
    };

    let list = rt_make_list(len as i64, ELEM_RAW_INT);

    unsafe {
        let list_obj = list as *mut ListObj;

        let mut current = start;
        for i in 0..len {
            *(*list_obj).data.add(i) = current as *mut Obj;
            current += step;
        }
        (*list_obj).len = len;
    }

    list
}

/// Create a list by consuming an iterator
/// elem_tag: 0 = ELEM_HEAP_OBJ, 1 = ELEM_RAW_INT (passed from compiler based on element type)
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_list_from_iter(iter: *mut Obj, elem_tag: i64) -> *mut Obj {
    use crate::iterator::rt_iter_next_no_exc;

    if iter.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    // Create list with initial capacity using the elem_tag from the compiler
    let list = rt_make_list(8, elem_tag as u8);

    // Keep getting elements until exhausted
    loop {
        let elem = rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        rt_list_push(list, elem);
    }

    list
}

/// Create a list from a set
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_list_from_set(set: *mut Obj) -> *mut Obj {
    use crate::set::rt_set_to_list;

    // rt_set_to_list already does what we need
    rt_set_to_list(set)
}

/// Create a list from a dict (keys only)
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_list_from_dict(dict: *mut Obj) -> *mut Obj {
    use crate::dict::rt_dict_keys;

    // rt_dict_keys already returns a list of keys
    rt_dict_keys(dict)
}

/// Extract list tail as tuple (list[start:] → tuple)
/// Used for varargs collection: def f(a, *rest): f(*my_list)
/// Returns: pointer to new TupleObj containing elements from start to end
/// NOTE: This copies elements verbatim. For int lists (ELEM_RAW_INT), this works directly.
/// For float/bool lists (ELEM_HEAP_OBJ with boxed values), use the specialized _unbox variants.
#[no_mangle]
pub extern "C" fn rt_list_tail_to_tuple(list: *mut Obj, start: i64) -> *mut Obj {
    use crate::object::ListObj;
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if list.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let list_len = (*list_obj).len as i64;
        let elem_tag = (*list_obj).elem_tag;

        // Calculate tail length
        let start_idx = start.max(0);
        let tail_len = (list_len - start_idx).max(0);

        // Create tuple with appropriate element tag
        let tuple = rt_make_tuple(tail_len, elem_tag);

        // Copy elements from list[start..] to tuple
        if tail_len > 0 {
            let src_data = (*list_obj).data;
            for i in 0..tail_len {
                let elem = *src_data.add((start_idx + i) as usize);
                rt_tuple_set(tuple, i, elem);
            }
        }

        tuple
    }
}

/// Extract list tail as tuple, unboxing float elements
/// Float lists store boxed FloatObj pointers, but varargs tuples need raw f64 bits
/// Returns: pointer to new TupleObj with unboxed float values
#[no_mangle]
pub extern "C" fn rt_list_tail_to_tuple_float(list: *mut Obj, start: i64) -> *mut Obj {
    use crate::boxing::rt_unbox_float;
    use crate::object::{ListObj, ELEM_RAW_INT};
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if list.is_null() {
        return rt_make_tuple(0, ELEM_RAW_INT);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let list_len = (*list_obj).len as i64;

        // Calculate tail length
        let start_idx = start.max(0);
        let tail_len = (list_len - start_idx).max(0);

        // Create tuple with ELEM_RAW_INT (floats stored as raw f64 bits)
        // Note: Varargs tuples store raw floats as i64-sized values
        let tuple = rt_make_tuple(tail_len, ELEM_RAW_INT);

        // Copy and unbox each float element
        if tail_len > 0 {
            let src_data = (*list_obj).data;
            for i in 0..tail_len {
                let boxed_float = *src_data.add((start_idx + i) as usize);
                // Unbox the float to get raw f64
                let raw_float = rt_unbox_float(boxed_float);
                // Store raw f64 bits as pointer-sized value
                let raw_bits = raw_float.to_bits() as i64;
                rt_tuple_set(tuple, i, raw_bits as *mut Obj);
            }
        }

        tuple
    }
}

/// Extract list tail as tuple, unboxing bool elements
/// Bool lists store boxed BoolObj pointers, but varargs tuples need raw i8 values
/// Returns: pointer to new TupleObj with unboxed bool values
#[no_mangle]
pub extern "C" fn rt_list_tail_to_tuple_bool(list: *mut Obj, start: i64) -> *mut Obj {
    use crate::boxing::rt_unbox_bool;
    use crate::object::{ListObj, ELEM_RAW_BOOL};
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if list.is_null() {
        return rt_make_tuple(0, ELEM_RAW_BOOL);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let list_len = (*list_obj).len as i64;

        // Calculate tail length
        let start_idx = start.max(0);
        let tail_len = (list_len - start_idx).max(0);

        // Create tuple with ELEM_RAW_BOOL (bools stored as raw i8)
        let tuple = rt_make_tuple(tail_len, ELEM_RAW_BOOL);

        // Copy and unbox each bool element
        if tail_len > 0 {
            let src_data = (*list_obj).data;
            for i in 0..tail_len {
                let boxed_bool = *src_data.add((start_idx + i) as usize);
                // Unbox the bool to get raw i8
                let raw_bool = rt_unbox_bool(boxed_bool);
                // Store raw i8 as pointer-sized value
                rt_tuple_set(tuple, i, raw_bool as *mut Obj);
            }
        }

        tuple
    }
}
