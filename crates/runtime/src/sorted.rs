//! Sorting operations for Python runtime

use crate::dict::rt_dict_keys;
use crate::gc::{gc_pop, gc_push, ShadowFrame};
use crate::list::rt_make_list;
use crate::object::Obj;
use crate::string::rt_str_getchar;
use pyaot_core_defs::Value;

// Helper functions for sorting

/// The one runtime ordering comparator for sort / `sorted` / `min` / `max`
/// and element-wise list/tuple ordering. Both `a` and `b` are tagged `Value`
/// bit-patterns cast to `*mut Obj`.
///
/// * numbers (int / bool / bignum / float, in any mix) compare by value
///   (`bigint::num_cmp`); a NaN compares `Equal` (CPython's `<` is False both
///   ways, so the sort leaves the pair in place);
/// * lists and tuples compare lexicographically (recursing here);
/// * class instances order via `__lt__`;
/// * everything else goes to `ops::obj_cmp_ordering`, which orders same-type
///   str/bytes and raises `TypeError` for unorderable mixes (`[1, "a"]`,
///   `None`), never dereferencing a tagged immediate as a heap header.
pub(crate) unsafe fn compare_list_elements(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    use crate::object::TypeTagKind;
    use std::cmp::Ordering;

    let va = Value(a as u64);
    let vb = Value(b as u64);
    // Hot path: two small ints.
    if va.is_int() && vb.is_int() {
        return va.unwrap_int().cmp(&vb.unwrap_int());
    }
    if a.is_null() || b.is_null() {
        return (a as usize).cmp(&(b as usize));
    }
    if let (Some(na), Some(nb)) = (
        crate::bigint::classify_num(va),
        crate::bigint::classify_num(vb),
    ) {
        return crate::bigint::num_cmp(&na, &nb).unwrap_or(Ordering::Equal);
    }
    if va.is_ptr() && vb.is_ptr() {
        let (tag_a, tag_b) = ((*a).type_tag(), (*b).type_tag());
        if tag_a == tag_b {
            match tag_a {
                TypeTagKind::Tuple => return crate::tuple::tuple_cmp_ordering(a, b),
                TypeTagKind::List => return crate::list::list_cmp_ordering(a, b),
                TypeTagKind::Instance => {
                    if let Some(ord) = crate::ops::try_instance_lt_ordering(a, b) {
                        return ord;
                    }
                }
                _ => {}
            }
        }
    }
    crate::ops::obj_cmp_ordering(a, b)
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

/// `stable_sort` over a freshly built (otherwise unreachable) result list,
/// keeping it rooted for the duration of the sort.
unsafe fn sort_rooted(list: *mut Obj, len: usize, reverse: bool) {
    let mut roots: [*mut Obj; 1] = [list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);
    stable_sort((*(list as *mut crate::object::ListObj)).data, len, reverse);
    gc_pop();
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

/// Container tag constants for `rt_sorted` dispatch.
const CONTAINER_LIST: u8 = 0;
const CONTAINER_TUPLE: u8 = 1;
const CONTAINER_DICT: u8 = 2;
const CONTAINER_SET: u8 = 3;
const CONTAINER_STR: u8 = 4;

/// Generic sorted: dispatches by `container_tag` to the appropriate implementation.
/// After §F.7c: containers store uniform tagged Values; no elem_tag needed.
/// Contract change (Phase 10): `reverse` is `i8` (the compiler's `Raw(I8)`
/// truthiness flag); `key=` is compiled as a frontend desugar feeding
/// `rt_list_sort_by_keys` — the callback-ABI `rt_sorted_with_key` is gone.
pub fn rt_sorted(obj: *mut Obj, reverse: i8, container_tag: u8) -> *mut Obj {
    let reverse = reverse as i64;
    match container_tag {
        CONTAINER_LIST => sorted_list_impl(obj, reverse),
        CONTAINER_TUPLE => sorted_tuple_impl(obj, reverse),
        CONTAINER_DICT => sorted_dict_impl(obj, reverse),
        CONTAINER_SET => sorted_set_impl(obj, reverse),
        CONTAINER_STR => sorted_str_impl(obj, reverse),
        _ => rt_make_list(0),
    }
}
#[export_name = "rt_sorted"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_sorted_abi(obj: Value, reverse: i8, container_tag: u8) -> Value {
    Value::from_ptr(rt_sorted(obj.unwrap_ptr(), reverse, container_tag))
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

            // Sort using stable sort (required for CPython compatibility).
            // Root the result across the sort: the comparator may run a user
            // `__lt__` that allocates and triggers a collection.
            sort_rooted(new_list, len, reverse != 0);
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

            // Sort using stable sort (required for CPython compatibility).
            // Root the result across the sort: the comparator may run a user
            // `__lt__` that allocates and triggers a collection.
            sort_rooted(new_list, len, reverse != 0);
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
pub fn rt_sorted_range(start: i64, stop: i64, step: i64, reverse: i64) -> *mut Obj {
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
#[export_name = "rt_sorted_range"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_sorted_range_abi(start: i64, stop: i64, step: i64, reverse: i64) -> Value {
    Value::from_ptr(rt_sorted_range(start, stop, step, reverse))
}

/// Compare two key values returned by key functions (the min/max with-key
/// paths in `list/minmax.rs`, `tuple/query.rs`, `set/core.rs`).
/// Both `a` and `b` are tagged `Value` bit-patterns (Int/Bool immediate or heap ptr).
pub(crate) unsafe fn compare_key_values(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    compare_list_elements(a, b)
}
