//! Sorting operations for Python runtime

use crate::dict::rt_dict_keys;
use crate::gc::{gc_pop, gc_push, ShadowFrame};
use crate::list::rt_make_list;
use crate::object::Obj;
use crate::string::rt_str_getchar;

// Helper functions for sorting

/// Compare two list/tuple elements; both `a` and `b` are tagged `Value` bit-patterns
/// cast to `*mut Obj`. Dispatches on `Value::tag()` for Int/Bool/None, then on the
/// object header's `TypeTagKind` for heap objects (Str, Float, …).
pub(crate) unsafe fn compare_list_elements(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    use crate::object::{FloatObj, StrObj, TypeTagKind};
    use pyaot_core_defs::Value;
    use std::cmp::Ordering;

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

    // Check Value-tagged primitives before heap pointer dereference.
    let va = Value(a as u64);
    let vb = Value(b as u64);
    if va.is_int() && vb.is_int() {
        return va.unwrap_int().cmp(&vb.unwrap_int());
    }
    if va.is_bool() && vb.is_bool() {
        return va.unwrap_bool().cmp(&vb.unwrap_bool());
    }
    // Cross-type Int/Bool comparison (Python: bool is int subtype)
    if (va.is_int() || va.is_bool()) && (vb.is_int() || vb.is_bool()) {
        let ia: i64 = if va.is_int() {
            va.unwrap_int()
        } else {
            va.unwrap_bool() as i64
        };
        let ib: i64 = if vb.is_int() {
            vb.unwrap_int()
        } else {
            vb.unwrap_bool() as i64
        };
        return ia.cmp(&ib);
    }

    let tag_a = (*a).header.type_tag;
    let tag_b = (*b).header.type_tag;

    // If types differ, compare by type tag
    if tag_a != tag_b {
        return (tag_a as u8).cmp(&(tag_b as u8));
    }

    match tag_a {
        TypeTagKind::Str => {
            let str_a = a as *mut StrObj;
            let str_b = b as *mut StrObj;
            let len_a = (*str_a).len;
            let len_b = (*str_b).len;
            let data_a = std::slice::from_raw_parts((*str_a).data.as_ptr(), len_a);
            let data_b = std::slice::from_raw_parts((*str_b).data.as_ptr(), len_b);
            data_a.cmp(data_b)
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

/// Stable sort for an array of `Value` slots.
/// After §F.7c: slots are tagged Values uniformly; the comparator dispatches
/// on `Value::tag()` (via `compare_list_elements`), so we pass the raw Value
/// bits directly without any `elem_tag` conversion.
unsafe fn stable_sort(data: *mut pyaot_core_defs::Value, len: usize, reverse: bool) {
    if len <= 1 {
        return;
    }

    let mut vec: Vec<pyaot_core_defs::Value> = (0..len).map(|i| *data.add(i)).collect();
    vec.sort_by(|&a, &b| {
        let ord = compare_list_elements(a.0 as *mut Obj, b.0 as *mut Obj);
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

/// Wrap a raw key-function return value into a tagged `Value` for comparison.
/// `tag`: 0 = heap pointer (low bits 0), 1 = raw i64 Int, 2 = raw 0/1 Bool,
/// 3 = first-class builtin (return value is already a tagged `Value`,
/// pass-through identical to tag 0).
pub(crate) fn wrap_key_result(key: *mut Obj, tag: u8) -> pyaot_core_defs::Value {
    use pyaot_core_defs::Value;
    match tag {
        1 => Value::from_int(key as i64),
        2 => Value::from_bool(key as i64 != 0),
        _ => Value(key as u64),
    }
}

/// Prepare a slot for the key-function call.
/// `tag`: 0/1/2 = user function (compiled body expects raw scalars); the
/// tagged `Value` slot is unwrapped to the underlying primitive bits.
/// `tag` = 3 = first-class builtin dispatcher (`rt_builtin_*`); the
/// dispatcher inspects `Value::tag()` itself, so the slot is passed through
/// unchanged.
pub(crate) fn unwrap_slot_for_key_fn(slot: pyaot_core_defs::Value, tag: u8) -> *mut Obj {
    if tag == 3 {
        return slot.0 as *mut Obj;
    }
    if slot.is_int() {
        slot.unwrap_int() as *mut Obj
    } else if slot.is_bool() {
        i64::from(slot.unwrap_bool()) as *mut Obj
    } else {
        slot.0 as *mut Obj
    }
}

/// Container tag constants for `rt_sorted` / `rt_sorted_with_key` dispatch.
const CONTAINER_LIST: u8 = 0;
const CONTAINER_TUPLE: u8 = 1;
const CONTAINER_DICT: u8 = 2;
const CONTAINER_SET: u8 = 3;
const CONTAINER_STR: u8 = 4;

/// Generic sorted: dispatches by `container_tag` to the appropriate implementation.
/// After §F.7c: containers store uniform tagged Values; no elem_tag needed.
#[no_mangle]
pub extern "C" fn rt_sorted(obj: *mut Obj, reverse: i64, container_tag: u8) -> *mut Obj {
    match container_tag {
        CONTAINER_LIST => sorted_list_impl(obj, reverse),
        CONTAINER_TUPLE => sorted_tuple_impl(obj, reverse),
        CONTAINER_DICT => sorted_dict_impl(obj, reverse),
        CONTAINER_SET => sorted_set_impl(obj, reverse),
        CONTAINER_STR => sorted_str_impl(obj, reverse),
        _ => rt_make_list(0),
    }
}

/// Generic sorted with key function: dispatches by `container_tag`.
/// `key_return_tag`: 0=heap, 1=Int(raw i64), 2=Bool(raw 0/1) — describes the key fn's return type.
/// After §F.7c: containers store uniform tagged Values; no elem_tag needed.
#[no_mangle]
pub extern "C" fn rt_sorted_with_key(
    obj: *mut Obj,
    reverse: i64,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    container_tag: u8,
    key_return_tag: u8,
) -> *mut Obj {
    match container_tag {
        CONTAINER_LIST => sorted_list_with_key_impl(
            obj,
            reverse,
            key_fn,
            captures,
            capture_count,
            key_return_tag,
        ),
        CONTAINER_TUPLE => sorted_tuple_with_key_impl(
            obj,
            reverse,
            key_fn,
            captures,
            capture_count,
            key_return_tag,
        ),
        CONTAINER_DICT => sorted_dict_with_key_impl(
            obj,
            reverse,
            key_fn,
            captures,
            capture_count,
            key_return_tag,
        ),
        CONTAINER_SET => sorted_set_with_key_impl(
            obj,
            reverse,
            key_fn,
            captures,
            capture_count,
            key_return_tag,
        ),
        CONTAINER_STR => sorted_str_with_key_impl(
            obj,
            reverse,
            key_fn,
            captures,
            capture_count,
            key_return_tag,
        ),
        _ => rt_make_list(0),
    }
}

// ==================== No-key implementations ====================

fn sorted_list_impl(list: *mut Obj, reverse: i64) -> *mut Obj {
    use crate::object::ListObj;

    if list.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;

        // Create new list as a copy
        let new_list = rt_make_list(len as i64);
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
            stable_sort(data, len, reverse != 0);
        }

        new_list
    }
}

fn sorted_tuple_impl(tuple: *mut Obj, reverse: i64) -> *mut Obj {
    use crate::object::{ListObj, TupleObj};

    if tuple.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let src = tuple as *mut TupleObj;
        let len = (*src).len;

        // Create new list
        let new_list = rt_make_list(len as i64);
        let new_list_obj = new_list as *mut ListObj;

        if len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_list_obj).data;

            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
            (*new_list_obj).len = len;

            // Sort using stable sort (required for CPython compatibility)
            let data = (*new_list_obj).data;
            stable_sort(data, len, reverse != 0);
        }

        new_list
    }
}

fn sorted_dict_impl(dict: *mut Obj, reverse: i64) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0);
    }

    // Get keys list (uniformly tagged Values)
    let keys_list = rt_dict_keys(dict);

    // Root keys_list before sorted_list_impl which allocates a new list
    let mut roots: [*mut Obj; 1] = [keys_list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };
    let result = sorted_list_impl(roots[0], reverse);
    gc_pop();
    result
}

fn sorted_set_impl(set: *mut Obj, reverse: i64) -> *mut Obj {
    if set.is_null() {
        return rt_make_list(0);
    }

    let list = crate::set::rt_set_to_list(set);

    let mut roots: [*mut Obj; 1] = [list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };
    let sorted = sorted_list_impl(roots[0], reverse);
    gc_pop();

    sorted
}

fn sorted_str_impl(str_obj: *mut Obj, reverse: i64) -> *mut Obj {
    use crate::object::{ListObj, StrObj};
    use crate::string::utf8_char_width;

    if str_obj.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;

        if byte_len == 0 {
            return rt_make_list(0);
        }

        // Collect one string per Unicode codepoint, walking byte-by-byte.
        // Pre-allocate with byte_len as an upper bound (ASCII strings need exactly
        // byte_len slots; multi-byte strings need fewer).
        let new_list = rt_make_list(byte_len as i64);

        // Root both str_obj and new_list.
        // rt_str_getchar -> rt_make_str -> gc_alloc may trigger GC on every call;
        // str_obj must survive so we can read the next character, and new_list
        // must survive to receive the new char string.
        let mut roots: [*mut Obj; 2] = [str_obj, new_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let mut byte_idx: usize = 0;
        let mut char_count: usize = 0;
        loop {
            // Re-derive str_obj and new_list through rooted slots after each alloc.
            let live_str = roots[0] as *mut StrObj;
            let live_byte_len = (*live_str).len;
            if byte_idx >= live_byte_len {
                break;
            }
            let first_byte = *(*live_str).data.as_ptr().add(byte_idx);
            let char_width = utf8_char_width(first_byte);
            let char_str = rt_str_getchar(roots[0], byte_idx as i64);

            // Write char_str into the list and update len immediately so GC
            // sees this slot as live on the next collection. char_str is
            // always a `*mut StrObj` stored as a tagged pointer Value.
            let live_list = roots[1] as *mut ListObj;
            (*live_list)
                .data
                .add(char_count)
                .write(pyaot_core_defs::Value::from_ptr(char_str));
            char_count += 1;
            (*live_list).len = char_count;

            byte_idx += char_width.min(live_byte_len - byte_idx);
        }

        // Sort using stable sort (required for CPython compatibility)
        let live_list = roots[1] as *mut ListObj;
        let data_ptr = (*live_list).data;
        stable_sort(data_ptr, char_count, reverse != 0);

        gc_pop();

        roots[1]
    }
}

/// Create a sorted list from a range
/// reverse: 0 for ascending, 1 for descending
/// Returns: pointer to new ListObj containing sorted integers (as raw i64 values)
#[no_mangle]
pub extern "C" fn rt_sorted_range(start: i64, stop: i64, step: i64, reverse: i64) -> *mut Obj {
    use crate::object::ListObj;

    if step == 0 {
        return rt_make_list(0);
    }

    // Calculate range length using i128 arithmetic to prevent overflow.
    // Plain i64 subtraction can wrap when operands span the full i64 range
    // (e.g. stop = i64::MAX, start = i64::MIN).
    let len = if step > 0 {
        let diff = (stop as i128).checked_sub(start as i128).unwrap_or(0);
        if diff <= 0 {
            0usize
        } else {
            let count = (diff + step as i128 - 1) / step as i128;
            count.min(i64::MAX as i128) as usize
        }
    } else {
        // step < 0 (step == 0 handled above)
        let diff = (start as i128).checked_sub(stop as i128).unwrap_or(0);
        if diff <= 0 {
            0usize
        } else {
            let count = (diff + (-step as i128) - 1) / (-step as i128);
            count.min(i64::MAX as i128) as usize
        }
    };

    let new_list = rt_make_list(len as i64);

    if len == 0 {
        return new_list;
    }

    unsafe {
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        // Fill with tagged-int Values.
        let mut current = start;
        for i in 0..len {
            *dst_data.add(i) = pyaot_core_defs::Value::from_int(current);
            current += step;
        }
        (*new_list_obj).len = len;

        // Sort using stable sort (required for CPython compatibility)
        // Range elements are raw integers
        let data = (*new_list_obj).data;
        stable_sort(data, len, reverse != 0);
    }

    new_list
}

// ==================== Sorted with key functions ====================

/// Compare two key values returned by key functions.
/// Both `a` and `b` are tagged `Value` bit-patterns (Int/Bool immediate or heap ptr).
pub(crate) unsafe fn compare_key_values(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    compare_list_elements(a, b)
}

/// Call key function with captures support (delegates to map's capture dispatcher)
unsafe fn call_key_fn(
    key_fn: i64,
    captures: *mut Obj,
    capture_count: u8,
    elem: *mut Obj,
) -> *mut Obj {
    crate::iterator::call_map_with_captures(key_fn, captures, capture_count, elem)
}

/// Stable sort for (key, index) pairs.
/// Stability ensures equal keys preserve original order (CPython guarantee).
unsafe fn stable_sort_key_pairs(pairs: &mut [(*mut Obj, usize)], reverse: bool) {
    pairs.sort_by(|a, b| {
        let ord = compare_key_values(a.0, b.0);
        if reverse {
            ord.reverse()
        } else {
            ord
        }
    });
}

/// Stable sort for (key, obj) pairs (used for rt_sorted_str_with_key).
/// Stability ensures equal keys preserve original order (CPython guarantee).
unsafe fn stable_sort_key_obj_pairs(pairs: &mut [(*mut Obj, *mut Obj)], reverse: bool) {
    pairs.sort_by(|a, b| {
        let ord = compare_key_values(a.0, b.0);
        if reverse {
            ord.reverse()
        } else {
            ord
        }
    });
}

// ==================== With-key implementations ====================

fn sorted_list_with_key_impl(
    list: *mut Obj,
    reverse: i64,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    key_return_tag: u8,
) -> *mut Obj {
    use crate::object::ListObj;

    if list.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let src = list as *mut ListObj;
        let len = (*src).len;

        if len == 0 {
            return rt_make_list(0);
        }

        let keys_list = rt_make_list(len as i64);

        let mut roots: [*mut Obj; 2] = [list, keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let cc = capture_count as u8;
        for i in 0..len {
            let src_live = roots[0] as *mut ListObj;
            // Unwrap tagged Value before passing to key fn (key fn expects raw scalars).
            let elem = unwrap_slot_for_key_fn(*(*src_live).data.add(i), key_return_tag);
            let key_value = call_key_fn(key_fn, captures, cc, elem);
            let keys_list_live = roots[1] as *mut ListObj;
            *(*keys_list_live).data.add(i) = wrap_key_result(key_value, key_return_tag);
            (*keys_list_live).len = i + 1;
        }

        let keys_list_live = roots[1] as *mut ListObj;
        let src_live = roots[0] as *mut ListObj;
        let mut key_index_pairs: Vec<(*mut Obj, usize)> = Vec::with_capacity(len);
        for i in 0..len {
            // Read raw bits directly (skip `load_value_as_raw` to avoid
            // double-dispatch on already-raw values from user fns).
            let raw_bits = (*(*keys_list_live).data.add(i)).0 as *mut Obj;
            key_index_pairs.push((raw_bits, i));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted indices
        let new_list = rt_make_list(len as i64);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        let src_data_live = (*src_live).data;
        for (i, (_, orig_idx)) in key_index_pairs.iter().enumerate() {
            // Both src and dst lists share `elem_tag`; the `Value` slot can
            // be copied verbatim (no re-conversion needed).
            *dst_data.add(i) = *src_data_live.add(*orig_idx);
        }
        (*new_list_obj).len = len;

        gc_pop();

        new_list
    }
}

fn sorted_tuple_with_key_impl(
    tuple: *mut Obj,
    reverse: i64,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    key_return_tag: u8,
) -> *mut Obj {
    use crate::object::{ListObj, TupleObj};

    if tuple.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let src = tuple as *mut TupleObj;
        let len = (*src).len;

        if len == 0 {
            return rt_make_list(0);
        }

        let keys_list = rt_make_list(len as i64);

        let mut roots: [*mut Obj; 2] = [tuple, keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let cc = capture_count as u8;
        for i in 0..len {
            let src_live = roots[0] as *mut TupleObj;
            let elem = *(*src_live).data.as_ptr().add(i);
            let key_input = unwrap_slot_for_key_fn(elem, key_return_tag);
            let key_value = call_key_fn(key_fn, captures, cc, key_input);
            let keys_list_live = roots[1] as *mut ListObj;
            *(*keys_list_live).data.add(i) = wrap_key_result(key_value, key_return_tag);
            (*keys_list_live).len = i + 1;
        }

        let keys_list_live = roots[1] as *mut ListObj;
        let src_live = roots[0] as *mut TupleObj;
        let mut key_index_pairs: Vec<(*mut Obj, usize)> = Vec::with_capacity(len);
        for i in 0..len {
            let raw_bits = (*(*keys_list_live).data.add(i)).0 as *mut Obj;
            key_index_pairs.push((raw_bits, i));
        }

        stable_sort_key_pairs(&mut key_index_pairs, reverse != 0);

        let new_list = rt_make_list(len as i64);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        let src_data_live = (*src_live).data.as_ptr();
        for (i, (_, orig_idx)) in key_index_pairs.iter().enumerate() {
            // Both tuple and list store Value; direct slot copy.
            *dst_data.add(i) = *src_data_live.add(*orig_idx);
        }
        (*new_list_obj).len = len;

        gc_pop();

        new_list
    }
}

fn sorted_dict_with_key_impl(
    dict: *mut Obj,
    reverse: i64,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    key_return_tag: u8,
) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0);
    }

    let keys_list = rt_dict_keys(dict);

    let mut roots: [*mut Obj; 1] = [keys_list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };
    let result = sorted_list_with_key_impl(
        roots[0],
        reverse,
        key_fn,
        captures,
        capture_count,
        key_return_tag,
    );
    gc_pop();
    result
}

fn sorted_set_with_key_impl(
    set: *mut Obj,
    reverse: i64,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    key_return_tag: u8,
) -> *mut Obj {
    if set.is_null() {
        return rt_make_list(0);
    }

    let list = crate::set::rt_set_to_list(set);

    let mut roots: [*mut Obj; 1] = [list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };
    let result = sorted_list_with_key_impl(
        roots[0],
        reverse,
        key_fn,
        captures,
        capture_count,
        key_return_tag,
    );
    gc_pop();
    result
}

fn sorted_str_with_key_impl(
    str_obj: *mut Obj,
    reverse: i64,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    key_return_tag: u8,
) -> *mut Obj {
    use crate::object::{ListObj, StrObj};
    use crate::string::utf8_char_width;

    if str_obj.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;

        if byte_len == 0 {
            return rt_make_list(0);
        }

        // Allocate two GC-visible lists: one for the char strings, one for the keys.
        // Both lists are rooted so neither is collected during rt_str_getchar / key_fn
        // calls that may trigger GC.
        let chars_list = rt_make_list(byte_len as i64);
        let keys_list = rt_make_list(byte_len as i64);

        let mut roots: [*mut Obj; 2] = [chars_list, keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Walk byte-by-byte, collecting one string per Unicode codepoint.
        let data = (*(str_obj as *mut StrObj)).data.as_ptr();
        let mut byte_idx: usize = 0;
        let mut char_count: usize = 0;
        let cc = capture_count as u8;
        while byte_idx < byte_len {
            let first_byte = *data.add(byte_idx);
            let char_width = utf8_char_width(first_byte);
            let char_str = rt_str_getchar(str_obj, byte_idx as i64);
            let chars_live = roots[0] as *mut ListObj;
            // Both chars_list and keys_list store tagged pointer Values.
            *(*chars_live).data.add(char_count) = pyaot_core_defs::Value::from_ptr(char_str);
            (*chars_live).len = char_count + 1;

            let key_value = call_key_fn(key_fn, captures, cc, char_str);
            let keys_live = roots[1] as *mut ListObj;
            *(*keys_live).data.add(char_count) = wrap_key_result(key_value, key_return_tag);
            (*keys_live).len = char_count + 1;

            char_count += 1;
            byte_idx += char_width.min(byte_len - byte_idx);
        }

        let chars_live = roots[0] as *mut ListObj;
        let keys_live = roots[1] as *mut ListObj;

        // Build (key, char_str) pairs from the stable lists
        let mut key_index_pairs: Vec<(*mut Obj, *mut Obj)> = Vec::with_capacity(char_count);
        for i in 0..char_count {
            let key_value = (*(*keys_live).data.add(i)).0 as *mut Obj;
            let char_str = (*(*chars_live).data.add(i)).0 as *mut Obj;
            key_index_pairs.push((key_value, char_str));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_obj_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted pairs
        let new_list = rt_make_list(char_count as i64);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        for (i, (_, char_str)) in key_index_pairs.iter().enumerate() {
            *dst_data.add(i) = pyaot_core_defs::Value::from_ptr(*char_str);
        }
        (*new_list_obj).len = char_count;

        gc_pop();

        new_list
    }
}
