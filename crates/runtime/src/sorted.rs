//! Sorting operations for Python runtime

use crate::dict::rt_dict_keys;
use crate::gc::{gc_pop, gc_push, ShadowFrame};
use crate::list::rt_make_list;
use crate::object::{Obj, ELEM_HEAP_OBJ, ELEM_RAW_INT};
use crate::string::rt_str_getchar;

use crate::object::ListObj;

/// Convert a sorted ELEM_HEAP_OBJ list of boxed ints to a ELEM_RAW_INT list.
/// Used when sorted(set[int]) or sorted(dict[int,...]) needs to produce list[int].
fn convert_heap_list_to_raw_int(heap_list: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    unsafe {
        let src = heap_list as *mut ListObj;
        let len = (*src).len;

        // Root heap_list across rt_make_list which calls gc_alloc and may trigger GC.
        let mut roots: [*mut Obj; 1] = [heap_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let result = rt_make_list(len as i64, ELEM_RAW_INT);

        gc_pop();

        // Re-derive src through the rooted pointer after the allocation.
        let src = roots[0] as *mut ListObj;
        let dst = result as *mut ListObj;
        for i in 0..len {
            let boxed = crate::list::list_slot_raw(src, i);
            let raw_val = crate::boxing::rt_unbox_int(boxed);
            *(*dst).data.add(i) = pyaot_core_defs::Value::from_int(raw_val);
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

/// Stable sort for an array of `Value` slots (post-S2.3 list storage).
/// Uses Vec::sort_by which is guaranteed stable (merge sort based).
/// reverse: false for ascending, true for descending
/// elem_tag: element storage type (ELEM_HEAP_OBJ, ELEM_RAW_INT, ELEM_RAW_BOOL)
unsafe fn stable_sort(data: *mut pyaot_core_defs::Value, len: usize, reverse: bool, elem_tag: u8) {
    if len <= 1 {
        return;
    }

    // Collect into a Vec, sort stably, write back. Comparisons happen on the
    // raw ABI form, so convert each slot for the comparator.
    let mut vec: Vec<pyaot_core_defs::Value> = (0..len).map(|i| *data.add(i)).collect();
    vec.sort_by(|&a, &b| {
        let a_raw = crate::list::load_value_as_raw(a, elem_tag);
        let b_raw = crate::list::load_value_as_raw(b, elem_tag);
        let ord = compare_list_elements(a_raw, b_raw, elem_tag);
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

/// Container tag constants for `rt_sorted` / `rt_sorted_with_key` dispatch.
const CONTAINER_LIST: u8 = 0;
const CONTAINER_TUPLE: u8 = 1;
const CONTAINER_DICT: u8 = 2;
const CONTAINER_SET: u8 = 3;
const CONTAINER_STR: u8 = 4;

/// Generic sorted: dispatches by `container_tag` to the appropriate implementation.
///
/// - `container_tag`: 0=list, 1=tuple, 2=dict, 3=set, 4=str
/// - `elem_tag`: element storage type (used by dict/set to produce correctly-typed results)
/// - `reverse`: 0 for ascending, 1 for descending
///
/// Returns pointer to new ListObj.
#[no_mangle]
pub extern "C" fn rt_sorted(
    obj: *mut Obj,
    reverse: i64,
    elem_tag: u8,
    container_tag: u8,
) -> *mut Obj {
    match container_tag {
        CONTAINER_LIST => sorted_list_impl(obj, reverse),
        CONTAINER_TUPLE => sorted_tuple_impl(obj, reverse),
        CONTAINER_DICT => sorted_dict_impl(obj, reverse, elem_tag),
        CONTAINER_SET => sorted_set_impl(obj, reverse, elem_tag),
        CONTAINER_STR => sorted_str_impl(obj, reverse),
        _ => rt_make_list(0, ELEM_HEAP_OBJ),
    }
}

/// Generic sorted with key function: dispatches by `container_tag`.
///
/// - `container_tag`: 0=list, 1=tuple, 2=dict, 3=set, 4=str
/// - `elem_tag`: element storage type for key function boxing
/// - `reverse`: 0 for ascending, 1 for descending
/// - `key_fn`: function pointer for key extraction
/// - `captures`: tuple of captured variables (null if no captures)
/// - `capture_count`: number of captured variables
///
/// Returns pointer to new ListObj.
#[no_mangle]
pub extern "C" fn rt_sorted_with_key(
    obj: *mut Obj,
    reverse: i64,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
    container_tag: u8,
) -> *mut Obj {
    match container_tag {
        CONTAINER_LIST => {
            sorted_list_with_key_impl(obj, reverse, key_fn, elem_tag, captures, capture_count)
        }
        CONTAINER_TUPLE => {
            sorted_tuple_with_key_impl(obj, reverse, key_fn, elem_tag, captures, capture_count)
        }
        CONTAINER_DICT => sorted_dict_with_key_impl(obj, reverse, key_fn, captures, capture_count),
        CONTAINER_SET => {
            sorted_set_with_key_impl(obj, reverse, key_fn, elem_tag, captures, capture_count)
        }
        CONTAINER_STR => sorted_str_with_key_impl(obj, reverse, key_fn, captures, capture_count),
        _ => rt_make_list(0, ELEM_HEAP_OBJ),
    }
}

// ==================== No-key implementations ====================

fn sorted_list_impl(list: *mut Obj, reverse: i64) -> *mut Obj {
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

fn sorted_tuple_impl(tuple: *mut Obj, reverse: i64) -> *mut Obj {
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

            // Tuple stores `*mut Obj`; list stores `Value`. Convert per-slot.
            let elem_tag = (*src).elem_tag;
            for i in 0..len {
                *dst_data.add(i) = crate::list::store_raw_as_value(*src_data.add(i), elem_tag);
            }
            (*new_list_obj).len = len;

            // Sort using stable sort (required for CPython compatibility)
            let data = (*new_list_obj).data;
            stable_sort(data, len, reverse != 0, elem_tag);
        }

        new_list
    }
}

fn sorted_dict_impl(dict: *mut Obj, reverse: i64, elem_tag: u8) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0, elem_tag);
    }

    // Get keys list with the target elem_tag (unboxes if ELEM_RAW_INT)
    let keys_list = rt_dict_keys(dict, elem_tag);

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

fn sorted_set_impl(set: *mut Obj, reverse: i64, elem_tag: u8) -> *mut Obj {
    if set.is_null() {
        return rt_make_list(0, elem_tag);
    }

    // Convert set to list (always ELEM_HEAP_OBJ since set stores boxed elements)
    let list = crate::set::rt_set_to_list(set);

    // Root list before sorted_list_impl which allocates a new list
    let mut roots: [*mut Obj; 1] = [list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };
    let sorted = sorted_list_impl(roots[0], reverse);
    gc_pop();

    // If caller wants ELEM_RAW_INT, unbox the sorted list
    if elem_tag == ELEM_RAW_INT {
        return convert_heap_list_to_raw_int(sorted);
    }

    sorted
}

fn sorted_str_impl(str_obj: *mut Obj, reverse: i64) -> *mut Obj {
    use crate::object::{ListObj, StrObj};
    use crate::string::utf8_char_width;

    if str_obj.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;

        if byte_len == 0 {
            return rt_make_list(0, ELEM_HEAP_OBJ);
        }

        // Collect one string per Unicode codepoint, walking byte-by-byte.
        // Pre-allocate with byte_len as an upper bound (ASCII strings need exactly
        // byte_len slots; multi-byte strings need fewer).
        let new_list = rt_make_list(byte_len as i64, ELEM_HEAP_OBJ);

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
            // always a `*mut StrObj`, i.e. ELEM_HEAP_OBJ.
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
        stable_sort(data_ptr, char_count, reverse != 0, ELEM_HEAP_OBJ);

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
        return rt_make_list(0, ELEM_HEAP_OBJ);
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

    let new_list = rt_make_list(len as i64, ELEM_RAW_INT);

    if len == 0 {
        return new_list;
    }

    unsafe {
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        // Fill with tagged-int Values. Post-S2.3, ELEM_RAW_INT list slots
        // are `Value::from_int(i)`.
        let mut current = start;
        for i in 0..len {
            *dst_data.add(i) = pyaot_core_defs::Value::from_int(current);
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

/// Determine if a pointer value is a valid heap object by validating its object header.
///
/// This is safer than the raw `is_heap_obj` address heuristic because it also checks
/// that the `type_tag` field at the target address contains a known `TypeTagKind` value.
/// A raw integer that happens to be address-aligned and in a plausible range but does
/// not point to a real object will almost certainly not have a valid type tag byte at
/// that address.
///
/// Steps:
/// 1. Coarse address check: non-null, minimum address, 8-byte aligned.
/// 2. Read the first byte of the putative object header (the `type_tag` field).
/// 3. Verify the byte is a known `TypeTagKind` discriminant.
///
/// This can still theoretically misidentify a raw integer whose value points to
/// memory that happens to contain a valid type tag byte, but that scenario is
/// vanishingly unlikely in practice given the combination of checks.
unsafe fn is_heap_obj_validated(ptr: *mut Obj) -> bool {
    use crate::object::TypeTagKind;
    let addr = ptr as usize;
    // Coarse address / alignment check first (cheap).
    if addr < 0x10000 || (addr & 0x7) != 0 {
        return false;
    }
    // Validate by reading the type_tag byte at the object header.
    let tag_byte = (ptr as *const u8).read();
    TypeTagKind::from_tag(tag_byte).is_some()
}

/// Compare two key values returned by key functions.
/// Key functions can return heap objects (strings, etc.) or raw integers (e.g. len()).
/// The storage type is detected by validating the object header's type_tag field rather
/// than relying solely on address-range heuristics, which could misidentify a large raw
/// integer that happens to be address-aligned as a heap object.
pub(crate) unsafe fn compare_key_values(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    let a_is_heap = is_heap_obj_validated(a);
    let b_is_heap = is_heap_obj_validated(b);
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
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
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

        // Allocate a GC-visible list to hold the key objects so they survive
        // subsequent rt_box_int / key_fn calls that may trigger collections.
        let keys_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

        // Root both the source list and the keys list across all key-function calls
        let mut roots: [*mut Obj; 2] = [list, keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Apply key function to each element; store keys in the GC-visible list
        let cc = capture_count as u8;
        let elem_tag_u8 = elem_tag as u8;
        for i in 0..len {
            let src_live = roots[0] as *mut ListObj;
            let elem = crate::list::list_slot_raw(src_live, i);
            let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
                crate::boxing::rt_box_int(elem as i64)
            } else {
                elem
            };
            let key_value = call_key_fn(key_fn, captures, cc, boxed_elem);
            let keys_list_live = roots[1] as *mut ListObj;
            // keys_list is ELEM_HEAP_OBJ (stores boxed keys).
            *(*keys_list_live).data.add(i) = pyaot_core_defs::Value::from_ptr(key_value);
            (*keys_list_live).len = i + 1;
        }

        // Build (key, orig_index) pairs from the now-stable keys list
        let keys_list_live = roots[1] as *mut ListObj;
        let src_live = roots[0] as *mut ListObj;
        let mut key_index_pairs: Vec<(*mut Obj, usize)> = Vec::with_capacity(len);
        for i in 0..len {
            let key_value = crate::list::list_slot_raw(keys_list_live, i);
            key_index_pairs.push((key_value, i));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted indices
        let new_list = rt_make_list(len as i64, (*src_live).elem_tag);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        let src_data_live = (*src_live).data;
        for (i, (_, orig_idx)) in key_index_pairs.iter().enumerate() {
            // Both src and dst lists share `elem_tag`; the `Value` slot can
            // be copied verbatim (no re-conversion needed).
            let _ = elem_tag_u8; // silence unused if this branch isn't hit
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
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
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

        // Allocate a GC-visible list to hold the key objects so they survive
        // subsequent rt_box_int / key_fn calls that may trigger collections.
        let keys_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

        // Root both the source tuple and the keys list
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
            // Tuple is still `*mut Obj`-backed (S2.4).
            let elem = *(*src_live).data.as_ptr().add(i);
            let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
                crate::boxing::rt_box_int(elem as i64)
            } else {
                elem
            };
            let key_value = call_key_fn(key_fn, captures, cc, boxed_elem);
            let keys_list_live = roots[1] as *mut ListObj;
            // keys_list is ELEM_HEAP_OBJ.
            *(*keys_list_live).data.add(i) = pyaot_core_defs::Value::from_ptr(key_value);
            (*keys_list_live).len = i + 1;
        }

        let keys_list_live = roots[1] as *mut ListObj;
        let src_live = roots[0] as *mut TupleObj;
        let mut key_index_pairs: Vec<(*mut Obj, usize)> = Vec::with_capacity(len);
        for i in 0..len {
            let key_value = crate::list::list_slot_raw(keys_list_live, i);
            key_index_pairs.push((key_value, i));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted indices
        let src_elem_tag = (*src_live).elem_tag;
        let new_list = rt_make_list(len as i64, src_elem_tag);
        let new_list_obj = new_list as *mut ListObj;
        let dst_data = (*new_list_obj).data;

        let src_data_live = (*src_live).data.as_ptr();
        for (i, (_, orig_idx)) in key_index_pairs.iter().enumerate() {
            // Tuple slots are raw `*mut Obj`; wrap them as `Value` using the
            // tuple's elem_tag when writing into the list.
            let raw = *src_data_live.add(*orig_idx);
            *dst_data.add(i) = crate::list::store_raw_as_value(raw, src_elem_tag);
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
) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    // Get keys list first, then sort it with key
    let keys_list = rt_dict_keys(dict, ELEM_HEAP_OBJ);

    // Root keys_list before sorted_list_with_key_impl which allocates
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
        ELEM_HEAP_OBJ as i64,
        captures,
        capture_count,
    );
    gc_pop();
    result
}

fn sorted_set_with_key_impl(
    set: *mut Obj,
    reverse: i64,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    if set.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    // Convert set to list, then sort with key
    let list = crate::set::rt_set_to_list(set);

    // Root list before sorted_list_with_key_impl which allocates
    let mut roots: [*mut Obj; 1] = [list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };
    let result =
        sorted_list_with_key_impl(roots[0], reverse, key_fn, elem_tag, captures, capture_count);
    gc_pop();
    result
}

fn sorted_str_with_key_impl(
    str_obj: *mut Obj,
    reverse: i64,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    use crate::object::{ListObj, StrObj};
    use crate::string::utf8_char_width;

    if str_obj.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;

        if byte_len == 0 {
            return rt_make_list(0, ELEM_HEAP_OBJ);
        }

        // Allocate two GC-visible lists: one for the char strings, one for the keys.
        // Both lists are rooted so neither is collected during rt_str_getchar / key_fn
        // calls that may trigger GC.
        let chars_list = rt_make_list(byte_len as i64, ELEM_HEAP_OBJ);
        let keys_list = rt_make_list(byte_len as i64, ELEM_HEAP_OBJ);

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
            // Both chars_list and keys_list are ELEM_HEAP_OBJ.
            *(*chars_live).data.add(char_count) = pyaot_core_defs::Value::from_ptr(char_str);
            (*chars_live).len = char_count + 1;

            let key_value = call_key_fn(key_fn, captures, cc, char_str);
            let keys_live = roots[1] as *mut ListObj;
            *(*keys_live).data.add(char_count) = pyaot_core_defs::Value::from_ptr(key_value);
            (*keys_live).len = char_count + 1;

            char_count += 1;
            byte_idx += char_width.min(byte_len - byte_idx);
        }

        let chars_live = roots[0] as *mut ListObj;
        let keys_live = roots[1] as *mut ListObj;

        // Build (key, char_str) pairs from the stable lists
        let mut key_index_pairs: Vec<(*mut Obj, *mut Obj)> = Vec::with_capacity(char_count);
        for i in 0..char_count {
            let key_value = crate::list::list_slot_raw(keys_live, i);
            let char_str = crate::list::list_slot_raw(chars_live, i);
            key_index_pairs.push((key_value, char_str));
        }

        // Sort by key values using stable sort (required for CPython compatibility)
        stable_sort_key_obj_pairs(&mut key_index_pairs, reverse != 0);

        // Build result list from sorted pairs
        let new_list = rt_make_list(char_count as i64, ELEM_HEAP_OBJ);
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
