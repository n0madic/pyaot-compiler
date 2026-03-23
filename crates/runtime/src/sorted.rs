//! Sorting operations for Python runtime

use crate::dict::rt_dict_keys;
use crate::list::rt_make_list;
use crate::object::{Obj, ELEM_HEAP_OBJ, ELEM_RAW_INT};
use crate::string::rt_str_getchar;

use crate::object::ListObj;

/// Convert a sorted ELEM_HEAP_OBJ list of boxed ints to a ELEM_RAW_INT list.
/// Used when sorted(set[int]) or sorted(dict[int,...]) needs to produce list[int].
fn convert_heap_list_to_raw_int(heap_list: *mut Obj) -> *mut Obj {
    unsafe {
        let src = heap_list as *mut ListObj;
        let len = (*src).len;
        let result = rt_make_list(len as i64, ELEM_RAW_INT);
        let dst = result as *mut ListObj;
        for i in 0..len {
            let boxed = *(*src).data.add(i);
            let raw_val = crate::boxing::rt_unbox_int(boxed);
            *(*dst).data.add(i) = raw_val as *mut Obj;
        }
        (*dst).len = len;
        result
    }
}

// Helper functions for sorting

pub(crate) unsafe fn compare_list_elements(
    a: *mut Obj,
    b: *mut Obj,
    elem_tag: u8,
) -> std::cmp::Ordering {
    use crate::object::{
        BoolObj, FloatObj, IntObj, StrObj, TypeTagKind, ELEM_RAW_BOOL, ELEM_RAW_INT,
    };
    use std::cmp::Ordering;

    // Use elem_tag to determine how to interpret the values
    match elem_tag {
        ELEM_RAW_INT => {
            // Raw integers stored as pointer values - compare as i64
            let val_a = a as i64;
            let val_b = b as i64;
            return val_a.cmp(&val_b);
        }
        ELEM_RAW_BOOL => {
            // Raw bools stored as pointer values - compare as i8
            let val_a = a as i8;
            let val_b = b as i8;
            return val_a.cmp(&val_b);
        }
        _ => {
            // ELEM_HEAP_OBJ or other - treat as heap objects
        }
    }

    // Both are heap objects - safe to dereference
    // Handle null cases
    if a.is_null() && b.is_null() {
        return Ordering::Equal;
    }
    if a.is_null() {
        return Ordering::Less;
    }
    if b.is_null() {
        return Ordering::Greater;
    }

    let tag_a = (*a).header.type_tag;
    let tag_b = (*b).header.type_tag;

    // If types differ, compare by type tag
    if tag_a != tag_b {
        return (tag_a as u8).cmp(&(tag_b as u8));
    }

    match tag_a {
        TypeTagKind::Int => {
            let int_a = (*(a as *mut IntObj)).value;
            let int_b = (*(b as *mut IntObj)).value;
            int_a.cmp(&int_b)
        }
        TypeTagKind::Str => {
            let str_a = a as *mut StrObj;
            let str_b = b as *mut StrObj;
            let len_a = (*str_a).len;
            let len_b = (*str_b).len;
            let data_a = std::slice::from_raw_parts((*str_a).data.as_ptr(), len_a);
            let data_b = std::slice::from_raw_parts((*str_b).data.as_ptr(), len_b);
            data_a.cmp(data_b)
        }
        TypeTagKind::Bool => {
            let bool_a = (*(a as *mut BoolObj)).value;
            let bool_b = (*(b as *mut BoolObj)).value;
            bool_a.cmp(&bool_b)
        }
        TypeTagKind::Float => {
            let float_a = (*(a as *mut FloatObj)).value;
            let float_b = (*(b as *mut FloatObj)).value;
            float_a.partial_cmp(&float_b).unwrap_or(Ordering::Equal)
        }
        _ => {
            // For other types, compare by pointer address
            (a as usize).cmp(&(b as usize))
        }
    }
}

/// Stable sort for an array of *mut Obj elements.
/// Uses Vec::sort_by which is guaranteed stable (merge sort based).
/// reverse: false for ascending, true for descending
/// elem_tag: element storage type (ELEM_HEAP_OBJ, ELEM_RAW_INT, ELEM_RAW_BOOL)
unsafe fn stable_sort(data: *mut *mut Obj, len: usize, reverse: bool, elem_tag: u8) {
    if len <= 1 {
        return;
    }

    // Collect into a Vec, sort stably, write back
    let mut vec: Vec<*mut Obj> = (0..len).map(|i| *data.add(i)).collect();
    vec.sort_by(|&a, &b| {
        let ord = compare_list_elements(a, b, elem_tag);
        if reverse {
            ord.reverse()
        } else {
            ord
        }
    });
    for (i, elem) in vec.into_iter().enumerate() {
        *data.add(i) = elem;
    }
}

/// Create a sorted list from a list
/// reverse: 0 for ascending, 1 for descending
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_sorted_list(list: *mut Obj, reverse: i64) -> *mut Obj {
    use crate::object::ListObj;

    if list.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;

        // Create new list as a copy
        let new_list = rt_make_list(len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        if len > 0 {
            let src_data = (*src).data;
            let dst_data = (*new_list_obj).data;

            // Copy elements
            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
            (*new_list_obj).len = len;

            // Sort using stable sort (required for CPython compatibility)
            let data = (*new_list_obj).data;
            stable_sort(data, len, reverse != 0, (*src).elem_tag);
        }

        new_list
    }
}

/// Create a sorted list from a tuple
/// reverse: 0 for ascending, 1 for descending
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_sorted_tuple(tuple: *mut Obj, reverse: i64) -> *mut Obj {
    use crate::object::{ListObj, TupleObj};

    if tuple.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = tuple as *mut TupleObj;
        let len = (*src).len;

        // Create new list
        let new_list = rt_make_list(len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut ListObj;

        if len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_list_obj).data;

            // Copy elements from tuple to list
            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
            (*new_list_obj).len = len;

            // Sort using stable sort (required for CPython compatibility)
            let data = (*new_list_obj).data;
            stable_sort(data, len, reverse != 0, (*src).elem_tag);
        }

        new_list
    }
}

/// Create a sorted list of keys from a dict
/// reverse: 0 for ascending, 1 for descending
/// Returns: pointer to new ListObj containing sorted keys
#[no_mangle]
pub extern "C" fn rt_sorted_dict(dict: *mut Obj, reverse: i64, elem_tag: u8) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0, elem_tag);
    }

    // Get keys list with the target elem_tag (unboxes if ELEM_RAW_INT)
    let keys_list = rt_dict_keys(dict, elem_tag);
    rt_sorted_list(keys_list, reverse)
}

/// Create a sorted list from a set
/// reverse: 0 for ascending, 1 for descending
/// Returns: pointer to new ListObj containing sorted elements
#[no_mangle]
pub extern "C" fn rt_sorted_set(set: *mut Obj, reverse: i64, elem_tag: u8) -> *mut Obj {
    if set.is_null() {
        return rt_make_list(0, elem_tag);
    }

    // Convert set to list (always ELEM_HEAP_OBJ since set stores boxed elements)
    let list = crate::set::rt_set_to_list(set);
    let sorted = rt_sorted_list(list, reverse);

    // If caller wants ELEM_RAW_INT, unbox the sorted list
    if elem_tag == ELEM_RAW_INT {
        return convert_heap_list_to_raw_int(sorted);
    }

    sorted
}

/// Create a sorted list of single-char strings from a string
/// reverse: 0 for ascending, 1 for descending
/// Returns: pointer to new ListObj containing sorted char strings
#[no_mangle]
pub extern "C" fn rt_sorted_str(str_obj: *mut Obj, reverse: i64) -> *mut Obj {
    use crate::object::{ListObj, StrObj};

    if str_obj.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Create list to hold character strings
        let new_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);
        let new_list_obj = new_list as *mut ListObj;

        if len > 0 {
            let dst_data = (*new_list_obj).data;

            // Create single-char strings for each byte
            for i in 0..len {
                let char_str = rt_str_getchar(str_obj, i as i64);
                *dst_data.add(i) = char_str;
            }
            (*new_list_obj).len = len;

            // Sort using stable sort (required for CPython compatibility)
            // Char strings are always heap objects
            let data = (*new_list_obj).data;
            stable_sort(data, len, reverse != 0, ELEM_HEAP_OBJ);
        }

        new_list
    }
}

/// Create a sorted list from a range
/// reverse: 0 for ascending, 1 for descending
/// Returns: pointer to new ListObj containing sorted integers (as raw i64 values)
#[no_mangle]
pub extern "C" fn rt_sorted_range(start: i64, stop: i64, step: i64, reverse: i64) -> *mut Obj {
    use crate::object::ListObj;

    if step == 0 {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    // Calculate range length
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

    let new_list = rt_make_list(len as i64, ELEM_RAW_INT);

    if len == 0 {
        return new_list;
    }

    unsafe {
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        // Fill with raw integer values (cast to *mut Obj)
        // This matches how list[int] stores elements
        let mut current = start;
        for i in 0..len {
            // Store raw integer as pointer (bit-cast)
            *dst_data.add(i) = current as *mut Obj;
            current += step;
        }
        (*new_list_obj).len = len;

        // Sort using stable sort (required for CPython compatibility)
        // Range elements are raw integers
        let data = (*new_list_obj).data;
        stable_sort(data, len, reverse != 0, ELEM_RAW_INT);
    }

    new_list
}

// ==================== Sorted with key functions ====================

/// Compare two key values returned by key functions.
/// Key functions can return heap objects (strings, etc.) or raw integers (e.g. len()),
/// so we detect the storage type using a heuristic.
pub(crate) unsafe fn compare_key_values(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    let a_is_heap = crate::utils::is_heap_obj(a);
    let b_is_heap = crate::utils::is_heap_obj(b);
    let elem_tag = if a_is_heap && b_is_heap {
        ELEM_HEAP_OBJ
    } else if !a_is_heap && !b_is_heap {
        ELEM_RAW_INT
    } else {
        // Mixed: one heap, one raw - compare as i64
        return (a as i64).cmp(&(b as i64));
    };
    compare_list_elements(a, b, elem_tag)
}

/// Type alias for key function pointer
/// The key function takes an element and returns a key value for comparison
type KeyFn = extern "C" fn(*mut Obj) -> *mut Obj;

/// Stable sort for (key, index) pairs.
/// Stability ensures equal keys preserve original order (CPython guarantee).
unsafe fn stable_sort_key_pairs(pairs: &mut [(*mut Obj, usize)], reverse: bool) {
    pairs.sort_by(|a, b| {
        let ord = compare_key_values(a.0, b.0);
        if reverse { ord.reverse() } else { ord }
    });
}

/// Stable sort for (key, obj) pairs (used for rt_sorted_str_with_key).
/// Stability ensures equal keys preserve original order (CPython guarantee).
unsafe fn stable_sort_key_obj_pairs(pairs: &mut [(*mut Obj, *mut Obj)], reverse: bool) {
    pairs.sort_by(|a, b| {
        let ord = compare_key_values(a.0, b.0);
        if reverse { ord.reverse() } else { ord }
    });
}

/// Create a sorted list from a list with a key function
/// key_fn: function pointer that takes an element and returns a key value
/// reverse: 0 for ascending, 1 for descending
/// elem_tag: element storage type (0=ELEM_HEAP_OBJ, 1=ELEM_RAW_INT, 2=ELEM_RAW_BOOL)
///           Used to box raw elements before passing to key function
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_sorted_list_with_key(
    list: *mut Obj,
    reverse: i64,
    key_fn: KeyFn,
    elem_tag: i64,
) -> *mut Obj {
    use crate::object::ListObj;

    if list.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;

        if len == 0 {
            return rt_make_list(0, ELEM_HEAP_OBJ);
        }

        let src_data = (*src).data;

        // Apply key function to each element and store (key_value, index) pairs
        let mut key_index_pairs: Vec<(*mut Obj, usize)> = Vec::with_capacity(len);
        for i in 0..len {
            let elem = *src_data.add(i);
            // Box raw elements before passing to key function
            let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
                crate::boxing::rt_box_int(elem as i64)
            } else {
                elem
            };
            let key_value = key_fn(boxed_elem);
            key_index_pairs.push((key_value, i));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted indices
        let new_list = rt_make_list(len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        for (i, (_, orig_idx)) in key_index_pairs.iter().enumerate() {
            *dst_data.add(i) = *src_data.add(*orig_idx);
        }
        (*new_list_obj).len = len;

        new_list
    }
}

/// Create a sorted list from a tuple with a key function
/// elem_tag: element storage type (0=ELEM_HEAP_OBJ, 1=ELEM_RAW_INT, 2=ELEM_RAW_BOOL)
///           Used to box raw elements before passing to key function
#[no_mangle]
pub extern "C" fn rt_sorted_tuple_with_key(
    tuple: *mut Obj,
    reverse: i64,
    key_fn: KeyFn,
    elem_tag: i64,
) -> *mut Obj {
    use crate::object::{ListObj, TupleObj};

    if tuple.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = tuple as *mut TupleObj;
        let len = (*src).len;

        if len == 0 {
            return rt_make_list(0, ELEM_HEAP_OBJ);
        }

        let src_data = (*src).data.as_ptr();

        // Apply key function to each element and store (key_value, index) pairs
        let mut key_index_pairs: Vec<(*mut Obj, usize)> = Vec::with_capacity(len);
        for i in 0..len {
            let elem = *src_data.add(i);
            // Box raw elements before passing to key function
            let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
                crate::boxing::rt_box_int(elem as i64)
            } else {
                elem
            };
            let key_value = key_fn(boxed_elem);
            key_index_pairs.push((key_value, i));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted indices
        let new_list = rt_make_list(len as i64, (*src).elem_tag);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        for (i, (_, orig_idx)) in key_index_pairs.iter().enumerate() {
            *dst_data.add(i) = *src_data.add(*orig_idx);
        }
        (*new_list_obj).len = len;

        new_list
    }
}

/// Create a sorted list of keys from a dict with a key function
#[no_mangle]
pub extern "C" fn rt_sorted_dict_with_key(dict: *mut Obj, reverse: i64, key_fn: KeyFn) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    // Get keys list first, then sort it with key
    // Dict keys are always boxed (ELEM_HEAP_OBJ = 0)
    let keys_list = rt_dict_keys(dict, ELEM_HEAP_OBJ);
    rt_sorted_list_with_key(keys_list, reverse, key_fn, ELEM_HEAP_OBJ as i64)
}

/// Create a sorted list from a set with a key function
#[no_mangle]
pub extern "C" fn rt_sorted_set_with_key(
    set: *mut Obj,
    reverse: i64,
    key_fn: KeyFn,
    elem_tag: i64,
) -> *mut Obj {
    if set.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    // Convert set to list, then sort with key
    let list = crate::set::rt_set_to_list(set);
    rt_sorted_list_with_key(list, reverse, key_fn, elem_tag)
}

/// Create a sorted list of single-char strings from a string with a key function
#[no_mangle]
pub extern "C" fn rt_sorted_str_with_key(
    str_obj: *mut Obj,
    reverse: i64,
    key_fn: KeyFn,
) -> *mut Obj {
    use crate::object::{ListObj, StrObj};

    if str_obj.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        if len == 0 {
            return rt_make_list(0, ELEM_HEAP_OBJ);
        }

        // First create single-char strings and compute keys
        let mut key_index_pairs: Vec<(*mut Obj, *mut Obj)> = Vec::with_capacity(len);
        for i in 0..len {
            let char_str = rt_str_getchar(str_obj, i as i64);
            let key_value = key_fn(char_str);
            key_index_pairs.push((key_value, char_str));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_obj_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted pairs
        let new_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        for (i, (_, char_str)) in key_index_pairs.iter().enumerate() {
            *dst_data.add(i) = *char_str;
        }
        (*new_list_obj).len = len;

        new_list
    }
}
