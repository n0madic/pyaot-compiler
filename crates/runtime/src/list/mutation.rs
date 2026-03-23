//! List mutation operations: append, pop, insert, remove, clear, reverse, extend, sort

use super::core::rt_list_push;
use super::timsort;
#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::exceptions::{rt_exc_raise, ExceptionType};
use crate::object::{ListObj, Obj, ObjHeader, StrObj, TypeTagKind, ELEM_HEAP_OBJ, ELEM_RAW_INT};
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

        // Get the element to return
        let result = *data.add(idx);

        // Shift remaining elements left
        let new_len = len as usize - 1;
        for i in idx..new_len {
            *data.add(i) = *data.add(i + 1);
        }
        *data.add(new_len) = std::ptr::null_mut();
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
                let new_layout = Layout::array::<*mut Obj>(new_capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_data = alloc_zeroed(new_layout) as *mut *mut Obj;
                (*list_obj).data = new_data;
            } else {
                let old_layout = Layout::array::<*mut Obj>(capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_layout = Layout::array::<*mut Obj>(new_capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_data =
                    realloc(data as *mut u8, old_layout, new_layout.size()) as *mut *mut Obj;
                if new_data.is_null() {
                    let msg = b"MemoryError: cannot allocate memory for list";
                    rt_exc_raise(ExceptionType::MemoryError as u8, msg.as_ptr(), msg.len());
                }
                for i in capacity..new_capacity {
                    *new_data.add(i) = std::ptr::null_mut();
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
            // Insert the new element
            *data.add(idx) = value;
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

        let elem_tag = (*list_obj).elem_tag;

        // Find the element using value equality for heap objects, raw equality for primitives
        for i in 0..len {
            let elem = *data.add(i);
            let found = if elem_tag == ELEM_HEAP_OBJ {
                crate::hash_table_utils::eq_hashable_obj(elem, value)
            } else {
                elem == value // Raw value comparison (ELEM_RAW_INT, ELEM_RAW_BOOL)
            };
            if found {
                // Shift remaining elements left
                for j in i..(len - 1) {
                    *data.add(j) = *data.add(j + 1);
                }
                *data.add(len - 1) = std::ptr::null_mut();
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
                *data.add(i) = std::ptr::null_mut();
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
            let tmp = *data.add(left);
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
        let other_data = (*other_obj).data;

        if other_data.is_null() || other_len == 0 {
            return;
        }

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

            let old_layout = Layout::array::<*mut Obj>(capacity)
                .expect("Allocation size overflow - capacity too large");
            let new_layout = Layout::array::<*mut Obj>(new_capacity)
                .expect("Allocation size overflow - new_capacity too large");

            let old_data = (*list_obj).data as *mut u8;
            let new_data = if old_data.is_null() {
                alloc_zeroed(new_layout)
            } else {
                realloc(old_data, old_layout, new_layout.size())
            };

            if new_data.is_null() {
                let msg = b"Failed to reallocate list";
                rt_exc_raise(ExceptionType::MemoryError as u8, msg.as_ptr(), msg.len());
            }

            (*list_obj).data = new_data as *mut *mut Obj;
            (*list_obj).capacity = new_capacity;
        }

        // Copy all elements in one operation
        let list_data = (*list_obj).data;
        std::ptr::copy_nonoverlapping(other_data, list_data.add(list_len), other_len);
        (*list_obj).len = new_len;
    }
}

/// Compare two objects for sorting
/// Returns -1 if a < b, 0 if a == b, 1 if a > b
unsafe fn compare_objects(a: *mut Obj, b: *mut Obj, elem_tag: u8) -> i32 {
    if elem_tag == ELEM_RAW_INT {
        // For raw integers, compare as signed i64
        let a_val = a as i64;
        let b_val = b as i64;
        if a_val < b_val {
            -1
        } else if a_val > b_val {
            1
        } else {
            0
        }
    } else if elem_tag == ELEM_HEAP_OBJ {
        // For heap objects, check the type
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
            // String comparison
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
            // If all compared bytes are equal, shorter string comes first
            if a_len < b_len {
                -1
            } else if a_len > b_len {
                1
            } else {
                0
            }
        } else {
            // For other objects, fall back to pointer comparison
            if a < b {
                -1
            } else if a > b {
                1
            } else {
                0
            }
        }
    } else {
        // Unknown element type, use pointer comparison
        if a < b {
            -1
        } else if a > b {
            1
        } else {
            0
        }
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

        let elem_tag = (*list_obj).elem_tag;

        // Use Timsort for O(n log n) performance
        if elem_tag == ELEM_RAW_INT {
            // For raw integers, convert to i64 slice and sort
            let slice = std::slice::from_raw_parts_mut(data as *mut i64, len);
            timsort::timsort_int(slice);

            if reverse != 0 {
                // Reverse the sorted array
                slice.reverse();
            }
        } else {
            // For heap objects, use custom comparison
            let slice = std::slice::from_raw_parts_mut(data, len);
            timsort::timsort_with_cmp(slice, |a, b| {
                let cmp = compare_objects(*a, *b, elem_tag);
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
        }
    }
}

/// Type alias for key function pointer
type KeyFn = extern "C" fn(*mut Obj) -> *mut Obj;

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
/// key_fn: function that takes an element and returns a key value for comparison
/// reverse: 0 = ascending, non-zero = descending
/// elem_tag: element storage type (0=ELEM_HEAP_OBJ, 1=ELEM_RAW_INT, 2=ELEM_RAW_BOOL)
///           Used to box raw elements before passing to key function
#[no_mangle]
pub extern "C" fn rt_list_sort_with_key(list: *mut Obj, reverse: i8, key_fn: KeyFn, elem_tag: i64) {
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

        // Apply key function to each element and store (key_value, original_value) pairs
        let mut key_value_pairs: Vec<(*mut Obj, *mut Obj)> = Vec::with_capacity(len);
        for i in 0..len {
            let elem = *data.add(i);
            // Box raw elements before passing to key function
            let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
                crate::boxing::rt_box_int(elem as i64)
            } else {
                elem
            };
            let key_value = key_fn(boxed_elem);
            key_value_pairs.push((key_value, elem));
        }

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

        // Write sorted values back to the list
        for (i, (_, value)) in key_value_pairs.iter().enumerate() {
            *data.add(i) = *value;
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

        // Get values to insert
        let (values_data, values_len) = if values.is_null() {
            (std::ptr::null_mut(), 0)
        } else {
            let values_obj = values as *mut ListObj;
            ((*values_obj).data, (*values_obj).len)
        };

        let old_len = (*list_obj).len;
        let new_len = old_len - slice_len + values_len;

        // Ensure capacity
        let capacity = (*list_obj).capacity;
        if new_len > capacity {
            // Calculate new capacity and resize
            use super::list_grow_capacity;
            let mut new_capacity = capacity;
            while new_capacity < new_len {
                new_capacity = list_grow_capacity(new_capacity);
            }

            let new_layout = Layout::array::<*mut Obj>(new_capacity)
                .expect("Allocation size overflow - capacity too large");
            let new_data = alloc_zeroed(new_layout) as *mut *mut Obj;

            // Copy existing elements
            if old_len > 0 {
                std::ptr::copy_nonoverlapping((*list_obj).data, new_data, old_len);
            }

            // Free old data (note: list data is allocated with std::alloc, not GC)
            // We don't free here as the old data might be referenced elsewhere

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
    }
}
