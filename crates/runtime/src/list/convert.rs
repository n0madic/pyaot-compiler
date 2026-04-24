//! List conversion operations: create lists from other types

use super::core::{rt_list_push, rt_make_list};
use super::{load_value_as_raw, store_raw_as_value};
use crate::gc::{gc_pop, gc_push, ShadowFrame};
use crate::object::{ListObj, Obj, StrObj, TupleObj, ELEM_HEAP_OBJ, ELEM_RAW_INT};
use pyaot_core_defs::Value;

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

        // Root the source tuple across rt_make_list which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [tuple];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let list = rt_make_list(len as i64, elem_tag);

        gc_pop();

        let tuple_obj = tuple as *mut TupleObj;
        let list_obj = list as *mut ListObj;

        if len > 0 {
            // Tuple data is still `*mut *mut Obj` (tuple migration is S2.4).
            // Convert each tuple slot to a tagged `Value` before writing into
            // the list's `[Value]` storage.
            let src_data = (*tuple_obj).data.as_ptr();
            let dst_data = (*list_obj).data;

            for i in 0..len {
                let raw = *src_data.add(i);
                *dst_data.add(i) = store_raw_as_value(raw, elem_tag);
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

        // Root both str_obj and the new list across every rt_make_str call.
        // rt_make_str calls gc_alloc which may trigger a collection, freeing
        // str_obj (invalidating `data`) or freeing the partially-built list.
        //
        // We first allocate the list, then push both into the shadow frame.
        let list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

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
            // List was created with ELEM_HEAP_OBJ, so each slot is a
            // `Value::from_ptr(*mut StrObj)`.
            (*list_obj).data.add(i).write(Value::from_ptr(char_str));
            // Update len immediately so GC sees this element as live
            (*list_obj).len = i + 1;
        }

        gc_pop();
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

    let list = rt_make_list(len as i64, ELEM_RAW_INT);

    unsafe {
        let list_obj = list as *mut ListObj;

        let mut current = start;
        for i in 0..len {
            // ELEM_RAW_INT list slots are tagged immediates — `Value::from_int`
            // is the canonical constructor.
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

/// Create a list by consuming an iterator
/// elem_tag: 0 = ELEM_HEAP_OBJ, 1 = ELEM_RAW_INT (passed from compiler based on element type)
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_list_from_iter(iter: *mut Obj, elem_tag: i64) -> *mut Obj {
    use crate::iterator::rt_iter_next_no_exc;

    if iter.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    // Create list with initial capacity using the elem_tag from the compiler.
    // Root the list across every rt_iter_next_no_exc call: the iterator's next()
    // may allocate (e.g., string iterator calls rt_str_getchar), which triggers
    // a GC collection under gc_stress_test.  Without rooting, the list would be
    // seen as unreachable and freed.
    let list = rt_make_list(8, elem_tag as u8);

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
    rt_dict_keys(dict, crate::object::ELEM_HEAP_OBJ)
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

        // Root the source list across rt_make_tuple which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let tuple = rt_make_tuple(tail_len, elem_tag);

        gc_pop();

        let list_obj = list as *mut ListObj;

        // Copy elements from list[start..] to tuple. Convert each `Value`
        // back to the raw ABI form that `rt_tuple_set` still expects
        // (tuple migration is S2.4).
        if tail_len > 0 {
            let src_data = (*list_obj).data;
            for i in 0..tail_len {
                let elem = load_value_as_raw(*src_data.add((start_idx + i) as usize), elem_tag);
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

        // Root the source list across rt_make_tuple which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Create tuple with ELEM_RAW_INT (floats stored as raw f64 bits)
        // Note: Varargs tuples store raw floats as i64-sized values
        let tuple = rt_make_tuple(tail_len, ELEM_RAW_INT);

        gc_pop();

        let list_obj = list as *mut ListObj;

        // Copy and unbox each float element. Float list slots are
        // ELEM_HEAP_OBJ (tagged `*mut FloatObj`); extract the pointer via
        // `load_value_as_raw`, then unbox to raw f64 bits.
        let src_elem_tag = (*list_obj).elem_tag;
        if tail_len > 0 {
            let src_data = (*list_obj).data;
            for i in 0..tail_len {
                let boxed_float =
                    load_value_as_raw(*src_data.add((start_idx + i) as usize), src_elem_tag);
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

        // Root the source list across rt_make_tuple which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Create tuple with ELEM_RAW_BOOL (bools stored as raw i8)
        let tuple = rt_make_tuple(tail_len, ELEM_RAW_BOOL);

        gc_pop();

        let list_obj = list as *mut ListObj;

        // Copy and unbox each bool element. Same pattern as the float
        // variant: extract the heap pointer from each `Value` slot, then
        // unbox to raw i8.
        let src_elem_tag = (*list_obj).elem_tag;
        if tail_len > 0 {
            let src_data = (*list_obj).data;
            for i in 0..tail_len {
                let boxed_bool =
                    load_value_as_raw(*src_data.add((start_idx + i) as usize), src_elem_tag);
                let raw_bool = rt_unbox_bool(boxed_bool);
                // Store raw i8 as pointer-sized value
                rt_tuple_set(tuple, i, raw_bool as *mut Obj);
            }
        }

        tuple
    }
}
