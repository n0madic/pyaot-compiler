//! List mutation operations: append, pop, insert, remove, clear, reverse, extend, sort

use super::core::rt_list_push;
use super::timsort;

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::exceptions::ExceptionType;
use crate::object::{ListObj, Obj, ObjHeader, StrObj, TypeTagKind};
use pyaot_core_defs::Value;
use std::alloc::{alloc_zeroed, realloc, Layout};

/// Append element to list (mutates list)
/// This is the same as rt_list_push, but named to match Python's .append()
pub fn rt_list_append(list: *mut Obj, value: *mut Obj) {
    rt_list_push(list, value);
}
#[export_name = "rt_list_append"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_append_abi(list: Value, value: Value) {
    // `value` is an element that may be a tagged immediate (int/bool/None);
    // pass raw bits so the tag survives instead of tripping `unwrap_ptr`'s
    // debug `is_ptr` assertion. `list` is always a heap pointer.
    rt_list_append(list.unwrap_ptr(), value.0 as *mut Obj)
}

/// Pop element from list at given index
/// Negative indices are supported
/// Returns: the removed element, or null if out of bounds
pub fn rt_list_pop(list: *mut Obj, index: i64) -> *mut Obj {
    if list.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_pop");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len as i64;

        if len == 0 {
            return std::ptr::null_mut();
        }

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return std::ptr::null_mut();
        }

        let idx = idx as usize;
        let data = (*list_obj).data;

        // Get the element to return (convert stored Value back to ABI *mut Obj).
        let stored = *data.add(idx);
        let result = stored.0 as *mut Obj;

        // Shift remaining elements left
        let new_len = len as usize - 1;
        for i in idx..new_len {
            *data.add(i) = *data.add(i + 1);
        }
        *data.add(new_len) = Value(0);
        (*list_obj).len = new_len;

        result
    }
}
#[export_name = "rt_list_pop"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_pop_abi(list: Value, index: i64) -> Value {
    Value::from_ptr(rt_list_pop(list.unwrap_ptr(), index))
}

/// `del li[index]` — remove the element at `index`, shifting the tail left.
/// Negative indices count from the right (like `rt_list_pop`). Unlike pop this
/// returns nothing and raises `IndexError` on an out-of-range index, matching
/// CPython's `del list[i]`.
pub fn rt_list_delete(list: *mut Obj, index: i64) {
    if list.is_null() {
        return;
    }
    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_delete");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len as i64;

        // Handle negative index, then bounds-check (raise like CPython).
        let idx = if index < 0 { len + index } else { index };
        if idx < 0 || idx >= len {
            raise_exc!(
                ExceptionType::IndexError,
                "list assignment index out of range"
            );
        }

        let idx = idx as usize;
        let data = (*list_obj).data;
        // Shift remaining elements left over the gap, clear the vacated slot.
        let new_len = len as usize - 1;
        for i in idx..new_len {
            *data.add(i) = *data.add(i + 1);
        }
        *data.add(new_len) = Value(0);
        (*list_obj).len = new_len;
    }
}
#[export_name = "rt_list_delete"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_delete_abi(list: Value, index: i64) {
    rt_list_delete(list.unwrap_ptr(), index)
}

/// Insert element at given index (mutates list)
/// Negative indices are supported
pub fn rt_list_insert(list: *mut Obj, index: i64, value: *mut Obj) {
    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_insert");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let capacity = (*list_obj).capacity;

        // Handle negative index
        let idx = if index < 0 {
            (len as i64 + index).max(0) as usize
        } else {
            (index as usize).min(len)
        };

        // Grow if needed (CPython-style growth: ~12.5% for large lists)
        if len >= capacity {
            let new_capacity = super::list_grow_capacity(capacity);
            let data = (*list_obj).data;

            if data.is_null() {
                let new_layout = Layout::array::<Value>(new_capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_data = alloc_zeroed(new_layout) as *mut Value;
                (*list_obj).data = new_data;
            } else {
                let old_layout = Layout::array::<Value>(capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_layout = Layout::array::<Value>(new_capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_data =
                    realloc(data as *mut u8, old_layout, new_layout.size()) as *mut Value;
                if new_data.is_null() {
                    raise_exc!(
                        ExceptionType::MemoryError,
                        "cannot allocate memory for list"
                    );
                }
                for i in capacity..new_capacity {
                    *new_data.add(i) = Value(0);
                }
                (*list_obj).data = new_data;
            }
            (*list_obj).capacity = new_capacity;
        }

        let data = (*list_obj).data;
        if !data.is_null() {
            // Shift elements right to make room
            for i in (idx..len).rev() {
                *data.add(i + 1) = *data.add(i);
            }
            // Insert the new element (value is already a tagged Value bit-pattern).
            *data.add(idx) = Value(value as u64);
            (*list_obj).len = len + 1;
        }
    }
}
#[export_name = "rt_list_insert"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_insert_abi(list: Value, index: i64, value: Value) {
    // `value` may be a tagged immediate; pass raw bits (see `rt_list_append_abi`).
    rt_list_insert(list.unwrap_ptr(), index, value.0 as *mut Obj)
}

/// Remove first occurrence of value from list (mutates list)
/// Uses value equality for heap objects, raw equality for primitives.
/// Returns: 1 if found and removed, 0 otherwise
pub fn rt_list_remove(list: *mut Obj, value: *mut Obj) -> i8 {
    if list.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_remove");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let data = (*list_obj).data;

        if data.is_null() {
            return 0;
        }

        // After F.7c slots are tagged Values; pass bits directly to eq_hashable_obj.
        // The search `value` is also a tagged Value (boxed by emit_value_slot).
        for i in 0..len {
            let stored = *data.add(i);
            let elem = stored.0 as *mut Obj;
            let found = crate::hash_table_utils::eq_hashable_obj(elem, value);
            if found {
                // Shift remaining elements left
                for j in i..(len - 1) {
                    *data.add(j) = *data.add(j + 1);
                }
                *data.add(len - 1) = Value(0);
                (*list_obj).len = len - 1;
                return 1;
            }
        }

        0
    }
}
#[export_name = "rt_list_remove"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_remove_abi(list: Value, value: Value) -> i8 {
    // `value` is the search element, possibly a tagged immediate; pass raw bits
    // (see `rt_list_append_abi`). The internal element comparison already
    // handles tagged values.
    rt_list_remove(list.unwrap_ptr(), value.0 as *mut Obj)
}

/// Clear all elements from list (mutates list)
pub fn rt_list_clear(list: *mut Obj) {
    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_clear");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let data = (*list_obj).data;

        if !data.is_null() {
            for i in 0..len {
                *data.add(i) = Value(0);
            }
        }
        (*list_obj).len = 0;
    }
}
#[export_name = "rt_list_clear"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_clear_abi(list: Value) {
    rt_list_clear(list.unwrap_ptr())
}

/// Reverse list in place (mutates list)
pub fn rt_list_reverse(list: *mut Obj) {
    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_reverse");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let data = (*list_obj).data;

        if data.is_null() || len < 2 {
            return;
        }

        let mut left = 0;
        let mut right = len - 1;
        while left < right {
            let tmp: Value = *data.add(left);
            *data.add(left) = *data.add(right);
            *data.add(right) = tmp;
            left += 1;
            right -= 1;
        }
    }
}
#[export_name = "rt_list_reverse"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_reverse_abi(list: Value) {
    rt_list_reverse(list.unwrap_ptr())
}

/// Extend list with elements from another list (mutates first list)
pub fn rt_list_extend(list: *mut Obj, other: *mut Obj) {
    if list.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_extend");
        debug_assert_type_tag!(other, TypeTagKind::List, "rt_list_extend");

        let list_obj = list as *mut ListObj;
        let other_obj = other as *mut ListObj;

        let list_len = (*list_obj).len;
        let other_len = (*other_obj).len;

        if other_len == 0 {
            return;
        }

        // Self-extend case (a.extend(a)): snapshot other's data before any realloc
        // that might move the underlying allocation and leave us with a dangling pointer.
        let snapshot: Option<Vec<Value>> = if list == other {
            let data = (*other_obj).data;
            if data.is_null() {
                return;
            }
            Some((0..other_len).map(|i| *data.add(i)).collect())
        } else {
            let data = (*other_obj).data;
            if data.is_null() {
                return;
            }
            None
        };

        // Pre-allocate capacity for all new elements (optimization)
        let new_len = list_len + other_len;
        let capacity = (*list_obj).capacity;

        if new_len > capacity {
            // Calculate new capacity and resize once
            use super::list_grow_capacity;
            let mut new_capacity = capacity;
            while new_capacity < new_len {
                new_capacity = list_grow_capacity(new_capacity);
            }

            let old_layout = Layout::array::<Value>(capacity)
                .expect("Allocation size overflow - capacity too large");
            let new_layout = Layout::array::<Value>(new_capacity)
                .expect("Allocation size overflow - new_capacity too large");

            let old_data = (*list_obj).data as *mut u8;
            let new_data = if old_data.is_null() {
                alloc_zeroed(new_layout)
            } else {
                realloc(old_data, old_layout, new_layout.size())
            };

            if new_data.is_null() {
                raise_exc!(ExceptionType::MemoryError, "Failed to reallocate list");
            }

            (*list_obj).data = new_data as *mut Value;
            (*list_obj).capacity = new_capacity;
        }

        // Copy elements: use snapshot for self-extend, direct copy otherwise
        let list_data = (*list_obj).data;
        if let Some(ref snap) = snapshot {
            for (i, &elem) in snap.iter().enumerate() {
                *list_data.add(list_len + i) = elem;
            }
        } else {
            // Safe: list != other, so their data buffers do not overlap
            let other_data = (*other_obj).data;
            std::ptr::copy_nonoverlapping(other_data, list_data.add(list_len), other_len);
        }
        (*list_obj).len = new_len;
    }
}
#[export_name = "rt_list_extend"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_extend_abi(list: Value, other: Value) {
    rt_list_extend(list.unwrap_ptr(), other.unwrap_ptr())
}

/// Compare two objects for sorting. Values are uniform tagged Values after F.7c.
/// Returns -1 if a < b, 0 if a == b, 1 if a > b
unsafe fn compare_objects(a: *mut Obj, b: *mut Obj) -> i32 {
    // Check Value tags before dereferencing as a heap pointer.
    let va = Value(a as u64);
    let vb = Value(b as u64);

    if va.is_int() && vb.is_int() {
        let av = va.unwrap_int();
        let bv = vb.unwrap_int();
        return if av < bv {
            -1
        } else if av > bv {
            1
        } else {
            0
        };
    }
    if va.is_bool() && vb.is_bool() {
        let av = va.unwrap_bool();
        let bv = vb.unwrap_bool();
        return if !av && bv {
            -1
        } else if av && !bv {
            1
        } else {
            0
        };
    }

    if a.is_null() && b.is_null() {
        return 0;
    }
    if a.is_null() {
        return -1;
    }
    if b.is_null() {
        return 1;
    }

    // None is encoded as a tagged Value (NONE_TAG = 0b101), not null, so it
    // would otherwise be dereferenced as a heap pointer below. Order None
    // before any non-None value (mirrors the null handling above).
    if va.is_none() && vb.is_none() {
        return 0;
    }
    if va.is_none() {
        return -1;
    }
    if vb.is_none() {
        return 1;
    }

    let a_header = a as *mut ObjHeader;
    let a_type = (*a_header).type_tag;

    if a_type == TypeTagKind::Str {
        let a_str = a as *mut StrObj;
        let b_str = b as *mut StrObj;
        let a_len = (*a_str).len;
        let b_len = (*b_str).len;
        let min_len = a_len.min(b_len);

        let a_data = (*a_str).data.as_ptr();
        let b_data = (*b_str).data.as_ptr();

        for i in 0..min_len {
            let a_byte = *a_data.add(i);
            let b_byte = *b_data.add(i);
            if a_byte < b_byte {
                return -1;
            }
            if a_byte > b_byte {
                return 1;
            }
        }
        if a_len < b_len {
            -1
        } else if a_len > b_len {
            1
        } else {
            0
        }
    } else if a_type == TypeTagKind::Float {
        let a_val = (*(a as *mut crate::object::FloatObj)).value;
        let b_val = (*(b as *mut crate::object::FloatObj)).value;
        match a_val.partial_cmp(&b_val) {
            Some(std::cmp::Ordering::Less) => -1,
            Some(std::cmp::Ordering::Greater) => 1,
            None => 1, // NaN sorts to the end
            _ => 0,
        }
    } else if a_type == TypeTagKind::Instance {
        // Class instances order via their `__lt__` dunder (CPython sorts with
        // `<`). Falls back to a stable address ordering when the class
        // defines no `__lt__`. Shared with `sorted()`'s comparator.
        match crate::ops::try_instance_lt_ordering(a, b) {
            Some(std::cmp::Ordering::Less) => -1,
            Some(std::cmp::Ordering::Greater) => 1,
            Some(std::cmp::Ordering::Equal) => 0,
            None if a < b => -1,
            None if a > b => 1,
            None => 0,
        }
    } else if a < b {
        -1
    } else if a > b {
        1
    } else {
        0
    }
}

/// Sort list in place
/// reverse: 0 = ascending, non-zero = descending
pub fn rt_list_sort(list: *mut Obj, reverse: i8) {
    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_sort");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;

        if len <= 1 {
            return;
        }

        let data = (*list_obj).data;
        if data.is_null() {
            return;
        }

        // Use Timsort for O(n log n) performance.
        // After F.7c all slots are uniform tagged Values — dispatch on Value::tag() at compare time.
        let slice = std::slice::from_raw_parts_mut(data, len);
        timsort::timsort_with_cmp(slice, |a, b| {
            let cmp = compare_objects(a.0 as *mut Obj, b.0 as *mut Obj);
            if reverse != 0 {
                match cmp {
                    c if c < 0 => std::cmp::Ordering::Greater,
                    c if c > 0 => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                }
            } else {
                match cmp {
                    c if c < 0 => std::cmp::Ordering::Less,
                    c if c > 0 => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                }
            }
        });
    }
}
#[export_name = "rt_list_sort"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_sort_abi(list: Value, reverse: i8) {
    rt_list_sort(list.unwrap_ptr(), reverse)
}

/// Stable tandem sort (contract change, Phase 10): sort `list` in place by
/// the parallel `keys` list. The `key=` callback runs as COMPILED code in a
/// frontend-desugared loop that builds `keys` BEFORE this call — the runtime
/// performs no callbacks (the PITFALLS-A4 `key_return_tag` ABI is gone).
/// The comparator is `compare_objects`, the same one `rt_list_sort` uses;
/// `reverse` flips only Less/Greater — Equal stays Equal, so equal keys keep
/// their original order (CPython stability). No allocation happens between
/// reading the `Value` slots and writing them back, and the GC is non-moving,
/// so holding the raw bits in a Rust Vec across comparisons is safe exactly
/// as in `rt_list_sort`.
pub fn rt_list_sort_by_keys(list: *mut Obj, keys: *mut Obj, reverse: i8) {
    if list.is_null() || keys.is_null() {
        return;
    }

    unsafe {
        // Loud type guard: the frontend dispatches `.sort(key=…)` by METHOD
        // NAME (the receiver's static type may be unknown), so a non-list
        // receiver must surface as a Python TypeError, not a wild deref.
        if (*list).header.type_tag != TypeTagKind::List
            || (*keys).header.type_tag != TypeTagKind::List
        {
            raise_exc!(
                ExceptionType::TypeError,
                "sort(key=) requires a list receiver"
            );
        }
        let list_obj = list as *mut ListObj;
        let keys_obj = keys as *mut ListObj;
        let len = (*list_obj).len;
        if (*keys_obj).len != len {
            raise_exc!(
                ExceptionType::TypeError,
                "sort(key=) keys/values length mismatch"
            );
        }
        if len <= 1 {
            return;
        }
        let data = (*list_obj).data;
        let kdata = (*keys_obj).data;
        if data.is_null() || kdata.is_null() {
            return;
        }

        let mut pairs: Vec<(Value, Value)> = (0..len)
            .map(|i| (*kdata.add(i), *data.add(i)))
            .collect();
        timsort::timsort_with_cmp(&mut pairs, |(key_a, _), (key_b, _)| {
            let cmp = compare_objects(key_a.0 as *mut Obj, key_b.0 as *mut Obj);
            if reverse != 0 {
                match cmp {
                    c if c < 0 => std::cmp::Ordering::Greater,
                    c if c > 0 => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Equal,
                }
            } else {
                match cmp {
                    c if c < 0 => std::cmp::Ordering::Less,
                    c if c > 0 => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                }
            }
        });

        for (i, (_, value)) in pairs.iter().enumerate() {
            *data.add(i) = *value;
        }
    }
}
#[export_name = "rt_list_sort_by_keys"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_sort_by_keys_abi(list: Value, keys: Value, reverse: i8) {
    unsafe {
        let l: *mut Obj = crate::utils::expect_ptr_or_type_error(list, "sort(key=)");
        let k: *mut Obj = crate::utils::expect_ptr_or_type_error(keys, "sort(key=)");
        rt_list_sort_by_keys(l, k, reverse)
    }
}

/// Replace list[start:stop] with values from another list
pub fn rt_list_slice_assign(list: *mut Obj, start: i64, stop: i64, values: *mut Obj) {
    use crate::slice_utils::normalize_slice_indices;
    use std::alloc::{alloc_zeroed, Layout};

    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_slice_assign");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len as i64;

        // Normalize indices (step=1 for simple slice)
        let (start, stop) = normalize_slice_indices(start, stop, len, 1);
        let slice_len = (stop - start).max(0) as usize;

        // Detect self-aliasing: if values and list are the same object,
        // snapshot the values data first to avoid use-after-free during realloc
        let (values_data, values_len, values_is_alias) = if values.is_null() {
            (std::ptr::null_mut::<Value>(), 0, false)
        } else if list == values {
            // Make a copy of the data before any potential realloc of list
            let vlist = values as *mut ListObj;
            let vlen = (*vlist).len;
            let copy = std::alloc::alloc(
                std::alloc::Layout::array::<Value>(vlen.max(1)).expect("alloc layout overflow"),
            ) as *mut Value;
            if vlen > 0 {
                std::ptr::copy_nonoverlapping((*vlist).data, copy, vlen);
            }
            (copy, vlen, true)
        } else {
            let values_obj = values as *mut ListObj;
            ((*values_obj).data, (*values_obj).len, false)
        };

        let old_len = (*list_obj).len;
        let new_len = old_len.saturating_sub(slice_len) + values_len;

        // Ensure capacity
        let capacity = (*list_obj).capacity;
        if new_len > capacity {
            // Calculate new capacity and resize
            use super::list_grow_capacity;
            let mut new_capacity = capacity;
            while new_capacity < new_len {
                new_capacity = list_grow_capacity(new_capacity);
            }

            let new_layout = Layout::array::<Value>(new_capacity)
                .expect("Allocation size overflow - capacity too large");
            let new_data = alloc_zeroed(new_layout) as *mut Value;

            // Copy existing elements into the new buffer
            let old_data = (*list_obj).data;
            if old_len > 0 {
                std::ptr::copy_nonoverlapping(old_data, new_data, old_len);
            }

            // Free the old data buffer (list data is std::alloc-managed, not GC-managed)
            if !old_data.is_null() && capacity > 0 {
                let old_layout =
                    Layout::array::<Value>(capacity).expect("list old layout overflow");
                std::alloc::dealloc(old_data as *mut u8, old_layout);
            }

            (*list_obj).data = new_data;
            (*list_obj).capacity = new_capacity;
        }

        let data = (*list_obj).data;

        // Move elements after slice
        if slice_len != values_len {
            let src_start = start as usize + slice_len;
            let dst_start = start as usize + values_len;
            let count = old_len - src_start;

            if count > 0 {
                std::ptr::copy(data.add(src_start), data.add(dst_start), count);
            }
        }

        // Copy new values into the slice
        if values_len > 0 && !values_data.is_null() {
            std::ptr::copy_nonoverlapping(values_data, data.add(start as usize), values_len);
        }

        (*list_obj).len = new_len;

        // Free the temporary snapshot buffer created for the self-aliasing case
        if values_is_alias && !values_data.is_null() {
            std::alloc::dealloc(
                values_data as *mut u8,
                std::alloc::Layout::array::<Value>(values_len.max(1))
                    .expect("alloc layout overflow"),
            );
        }
    }
}
#[export_name = "rt_list_slice_assign"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_slice_assign_abi(list: Value, start: i64, stop: i64, values: Value) {
    rt_list_slice_assign(list.unwrap_ptr(), start, stop, values.unwrap_ptr())
}
