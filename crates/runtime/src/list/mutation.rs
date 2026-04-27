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
#[no_mangle]
pub extern "C" fn rt_list_append(list: *mut Obj, value: *mut Obj) {
    rt_list_push(list, value);
}

/// Pop element from list at given index
/// Negative indices are supported
/// Returns: the removed element, or null if out of bounds
#[no_mangle]
pub extern "C" fn rt_list_pop(list: *mut Obj, index: i64) -> *mut Obj {
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

/// Insert element at given index (mutates list)
/// Negative indices are supported
#[no_mangle]
pub extern "C" fn rt_list_insert(list: *mut Obj, index: i64, value: *mut Obj) {
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

/// Remove first occurrence of value from list (mutates list)
/// Uses value equality for heap objects, raw equality for primitives.
/// Returns: 1 if found and removed, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_list_remove(list: *mut Obj, value: *mut Obj) -> i8 {
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

/// Clear all elements from list (mutates list)
#[no_mangle]
pub extern "C" fn rt_list_clear(list: *mut Obj) {
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

/// Reverse list in place (mutates list)
#[no_mangle]
pub extern "C" fn rt_list_reverse(list: *mut Obj) {
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

/// Extend list with elements from another list (mutates first list)
#[no_mangle]
pub extern "C" fn rt_list_extend(list: *mut Obj, other: *mut Obj) {
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
#[no_mangle]
pub extern "C" fn rt_list_sort(list: *mut Obj, reverse: i8) {
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

/// Compare two key objects for sorting
/// Returns -1 if a < b, 0 if a == b, 1 if a > b
/// Key values can be any type (heap objects or raw integers), so we detect
/// the storage type using a heuristic since key function return types vary.
unsafe fn compare_key_objects(a: *mut Obj, b: *mut Obj) -> i32 {
    use crate::sorted::compare_key_values;
    use std::cmp::Ordering;

    match compare_key_values(a, b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Sort list in place with a key function
/// key_fn: function pointer for key extraction
/// reverse: 0 = ascending, non-zero = descending
/// captures: tuple of captured variables (null if no captures)
/// capture_count: number of captured variables
/// key_return_tag: 0=heap, 1=Int(raw i64), 2=Bool(raw 0/1)
#[no_mangle]
pub extern "C" fn rt_list_sort_with_key(
    list: *mut Obj,
    reverse: i8,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    key_return_tag: u8,
) {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::sorted::{unwrap_slot_for_key_fn, wrap_key_result};

    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_sort_with_key");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;

        if len <= 1 {
            return;
        }

        let data = (*list_obj).data;
        if data.is_null() {
            return;
        }

        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let cc = capture_count as u8;
        let mut key_value_pairs: Vec<(*mut Obj, Value)> = Vec::with_capacity(len);
        for i in 0..len {
            let current_data = (*(list as *mut ListObj)).data;
            let stored = *current_data.add(i);
            let raw_elem = unwrap_slot_for_key_fn(stored, key_return_tag);
            let raw_key = crate::iterator::call_map_with_captures(key_fn, captures, cc, raw_elem);
            let key_value = wrap_key_result(raw_key, key_return_tag).0 as *mut Obj;
            key_value_pairs.push((key_value, stored));
        }

        gc_pop();

        // Sort by key values using Timsort (stable sort)
        timsort::timsort_with_cmp(&mut key_value_pairs, |(key_a, _), (key_b, _)| {
            let cmp = compare_key_objects(*key_a, *key_b);
            if reverse != 0 {
                // Reverse comparison for descending order
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

        // Write sorted values back to the list.
        // Re-read data from the list in case it was reallocated during key_fn
        // calls above (though rt_list_sort_with_key does not resize the list
        // itself, the GC is non-moving so the pointer is stable; re-deriving
        // makes the liveness explicit and is cheap).
        let final_data = (*(list as *mut ListObj)).data;
        for (i, (_, value)) in key_value_pairs.iter().enumerate() {
            *final_data.add(i) = *value;
        }
    }
}

/// Replace list[start:stop] with values from another list
#[no_mangle]
pub extern "C" fn rt_list_slice_assign(list: *mut Obj, start: i64, stop: i64, values: *mut Obj) {
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
