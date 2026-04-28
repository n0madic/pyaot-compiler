//! List conversion operations: create lists from other types

use super::core::{rt_list_push, rt_make_list};
use crate::gc::{gc_pop, gc_push, ShadowFrame};
use crate::object::{ListObj, Obj, StrObj, TupleObj};
use pyaot_core_defs::Value;

/// Create a list from a tuple
/// Returns: pointer to new ListObj
pub fn rt_list_from_tuple(tuple: *mut Obj) -> *mut Obj {
    if tuple.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let tuple_obj = tuple as *mut TupleObj;
        let len = (*tuple_obj).len;

        // Root the source tuple across rt_make_list which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [tuple];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let list = rt_make_list(len as i64);

        gc_pop();

        let tuple_obj = tuple as *mut TupleObj;
        let list_obj = list as *mut ListObj;

        if len > 0 {
            // Both tuple and list store `[Value]`; direct slot copy.
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
#[export_name = "rt_list_from_tuple"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_from_tuple_abi(tuple: Value) -> Value {
    Value::from_ptr(rt_list_from_tuple(tuple.unwrap_ptr()))
}


/// Create a list from a string (each character becomes an element)
/// Returns: pointer to new ListObj
pub fn rt_list_from_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::string::rt_make_str;

    if str_obj.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let str = str_obj as *mut StrObj;
        let len = (*str).len;

        // Root both str_obj and the new list across every rt_make_str call.
        // rt_make_str calls gc_alloc which may trigger a collection, freeing
        // str_obj (invalidating `data`) or freeing the partially-built list.
        //
        // We first allocate the list, then push both into the shadow frame.
        let list = rt_make_list(len as i64);

        // Root both str_obj and list.
        let mut roots: [*mut Obj; 2] = [str_obj, list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        for i in 0..len {
            // Re-derive str pointer through the rooted str_obj after each alloc.
            let src = roots[0] as *mut StrObj;
            let ch = (*src).data.as_ptr().add(i);
            // Create single-character string (may trigger GC)
            let char_str = rt_make_str(ch, 1);
            // Re-derive list_obj through the rooted list pointer (non-moving GC)
            let list_obj = roots[1] as *mut ListObj;
            // Each slot is a tagged pointer Value::from_ptr(*mut StrObj).
            (*list_obj).data.add(i).write(Value::from_ptr(char_str));
            // Update len immediately so GC sees this element as live
            (*list_obj).len = i + 1;
        }

        gc_pop();
        list
    }
}
#[export_name = "rt_list_from_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_from_str_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_list_from_str(str_obj.unwrap_ptr()))
}


/// Create a list from a range
/// Returns: pointer to new ListObj
pub fn rt_list_from_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    if step == 0 {
        return rt_make_list(0);
    }

    // Use i128 arithmetic to avoid overflow when computing the element count.
    // i64::MAX - i64::MIN can exceed i64::MAX, so plain i64 subtraction wraps.
    let len = if step > 0 {
        let diff = (stop as i128).checked_sub(start as i128).unwrap_or(0);
        if diff <= 0 {
            0usize
        } else {
            let count = (diff + step as i128 - 1) / step as i128;
            count.min(i64::MAX as i128) as usize
        }
    } else {
        // step < 0 (step == 0 is handled above)
        let diff = (start as i128).checked_sub(stop as i128).unwrap_or(0);
        if diff <= 0 {
            0usize
        } else {
            let count = (diff + (-step as i128) - 1) / (-step as i128);
            count.min(i64::MAX as i128) as usize
        }
    };

    let list = rt_make_list(len as i64);

    unsafe {
        let list_obj = list as *mut ListObj;

        let mut current = start;
        for i in 0..len {
            *(*list_obj).data.add(i) = Value::from_int(current);
            current = match current.checked_add(step) {
                Some(v) => v,
                // Overflow means we've exceeded i64 range; stop iteration
                None => break,
            };
        }
        (*list_obj).len = len;
    }

    list
}
#[export_name = "rt_list_from_range"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_from_range_abi(start: i64, stop: i64, step: i64) -> Value {
    Value::from_ptr(rt_list_from_range(start, stop, step))
}


/// Create a list by consuming an iterator
/// Returns: pointer to new ListObj
pub fn rt_list_from_iter(iter: *mut Obj) -> *mut Obj {
    use crate::iterator::rt_iter_next_no_exc;

    if iter.is_null() {
        return rt_make_list(0);
    }

    // Root the list across every rt_iter_next_no_exc call: the iterator's next()
    // may allocate (e.g., string iterator calls rt_str_getchar), which triggers
    // a GC collection under gc_stress_test.  Without rooting, the list would be
    // seen as unreachable and freed.
    let list = rt_make_list(8);

    let mut roots: [*mut Obj; 1] = [list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    // Keep getting elements until exhausted
    loop {
        let elem = rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        rt_list_push(roots[0], elem);
    }

    gc_pop();
    roots[0]
}
#[export_name = "rt_list_from_iter"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_from_iter_abi(iter: Value) -> Value {
    Value::from_ptr(rt_list_from_iter(iter.unwrap_ptr()))
}


/// Create a list from a set
/// Returns: pointer to new ListObj
pub fn rt_list_from_set(set: *mut Obj) -> *mut Obj {
    use crate::set::rt_set_to_list;

    // rt_set_to_list already does what we need
    rt_set_to_list(set)
}
#[export_name = "rt_list_from_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_from_set_abi(set: Value) -> Value {
    Value::from_ptr(rt_list_from_set(set.unwrap_ptr()))
}


/// Create a list from a dict (keys only)
/// Returns: pointer to new ListObj
pub fn rt_list_from_dict(dict: *mut Obj) -> *mut Obj {
    use crate::dict::rt_dict_keys;

    rt_dict_keys(dict)
}
#[export_name = "rt_list_from_dict"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_from_dict_abi(dict: Value) -> Value {
    Value::from_ptr(rt_list_from_dict(dict.unwrap_ptr()))
}


/// Extract list tail as tuple (list[start:] → tuple)
/// Used for varargs collection: def f(a, *rest): f(*my_list)
/// Returns: pointer to new TupleObj containing elements from start to end
/// NOTE: This copies elements verbatim. Slots are uniformly tagged Values, so the copy is direct.
pub fn rt_list_tail_to_tuple(list: *mut Obj, start: i64) -> *mut Obj {
    use crate::object::ListObj;
    use crate::tuple::rt_make_tuple;

    if list.is_null() {
        return rt_make_tuple(0);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let list_len = (*list_obj).len as i64;

        // Calculate tail length
        let start_idx = start.max(0);
        let tail_len = (list_len - start_idx).max(0);

        // Root the source list across rt_make_tuple which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let tuple = rt_make_tuple(tail_len);

        gc_pop();

        let list_obj = list as *mut ListObj;

        if tail_len > 0 {
            let src_data = (*list_obj).data;
            // After §F.7c: copy Value slots verbatim — tuple stores tagged
            // Values uniformly, matching the source list's encoding.
            let tuple_data = (*(tuple as *mut crate::object::TupleObj)).data.as_mut_ptr();
            for i in 0..tail_len {
                *tuple_data.add(i as usize) = *src_data.add((start_idx + i) as usize);
            }
        }

        tuple
    }
}
#[export_name = "rt_list_tail_to_tuple"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_tail_to_tuple_abi(list: Value, start: i64) -> Value {
    Value::from_ptr(rt_list_tail_to_tuple(list.unwrap_ptr(), start))
}


/// Extract list tail as tuple, keeping float elements as heap-boxed FloatObj pointers.
/// The varargs iterator returns each slot as *mut FloatObj; codegen calls rt_unbox_float.
pub fn rt_list_tail_to_tuple_float(list: *mut Obj, start: i64) -> *mut Obj {
    rt_list_tail_to_tuple(list, start)
}
#[export_name = "rt_list_tail_to_tuple_float"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_tail_to_tuple_float_abi(list: Value, start: i64) -> Value {
    Value::from_ptr(rt_list_tail_to_tuple_float(list.unwrap_ptr(), start))
}


/// Extract list tail as tuple, unboxing bool elements
/// Bool lists store boxed BoolObj pointers, but varargs tuples need raw i8 values
/// Returns: pointer to new TupleObj with unboxed bool values
pub fn rt_list_tail_to_tuple_bool(list: *mut Obj, start: i64) -> *mut Obj {
    // After F.7c, bool slots are already tagged Value::from_bool. Delegate to
    // the generic tail_to_tuple which uses load_value_as_raw (dispatches on Value::tag).
    rt_list_tail_to_tuple(list, start)
}
#[export_name = "rt_list_tail_to_tuple_bool"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_tail_to_tuple_bool_abi(list: Value, start: i64) -> Value {
    Value::from_ptr(rt_list_tail_to_tuple_bool(list.unwrap_ptr(), start))
}

