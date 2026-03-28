//! Counter runtime support
//!
//! Counter is a dict subclass for counting hashable objects.
//! Elements are stored as dict keys and their counts as values.
//! Uses the same DictObj layout with TypeTagKind::Counter.

use crate::dict::rt_dict_set;
use crate::gc::{self, gc_pop, gc_push, ShadowFrame};
use crate::hash_table_utils::{eq_hashable_obj, hash_hashable_obj};
use crate::object::{DictEntry, DictObj, IntObj, Obj, TypeTagKind};

/// Create an empty Counter
#[no_mangle]
pub extern "C" fn rt_make_counter_empty() -> *mut Obj {
    let dict_size = std::mem::size_of::<DictObj>();
    let obj = gc::gc_alloc(dict_size, TypeTagKind::Counter as u8);

    unsafe {
        init_empty_counter(obj as *mut DictObj);
    }

    obj
}

/// Create a Counter from an iterator — counts occurrences of each element.
/// The iterator elements are used as dict keys, so raw integers from
/// ELEM_RAW_INT lists need to be boxed.
#[no_mangle]
pub extern "C" fn rt_make_counter_from_iter(iter: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorObj, ListObj, ELEM_RAW_INT};

    let obj = rt_make_counter_empty();

    if iter.is_null() {
        return obj;
    }

    unsafe {
        // Check if the underlying container uses raw ints
        // (list iterator with ELEM_RAW_INT needs boxing)
        let needs_boxing = if (*iter).type_tag() == TypeTagKind::Iterator {
            let iter_obj = iter as *mut IteratorObj;
            let container = (*iter_obj).source;
            if !container.is_null() && (*container).type_tag() == TypeTagKind::List {
                let list = container as *mut ListObj;
                (*list).elem_tag == ELEM_RAW_INT
            } else {
                false
            }
        } else {
            false
        };

        // Iterate and count
        loop {
            let raw_elem = crate::iterator::rt_iter_next_no_exc(iter);
            if raw_elem.is_null() {
                break;
            }

            // Box raw integers if needed
            let elem = if needs_boxing {
                crate::boxing::rt_box_int(raw_elem as i64)
            } else {
                raw_elem
            };

            // Root obj (counter) and elem during allocating operations.
            // rt_box_int and rt_dict_set may trigger GC; without rooting,
            // the counter and the current element could be collected.
            let mut roots: [*mut Obj; 2] = [obj, elem];
            let mut frame = ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 2,
                roots: roots.as_mut_ptr(),
            };
            gc_push(&mut frame);

            // Get current count (or 0 if not found)
            let dict = obj as *mut DictObj;
            let current = get_count_or_zero(dict, elem);
            let new_count = current + 1;

            // Store new count as boxed int
            let boxed_count = crate::boxing::rt_box_int(new_count);
            rt_dict_set(obj, elem, boxed_count);

            gc_pop();
        }
    }

    obj
}

/// Counter.most_common(n) — return list of (element, count) tuples, sorted by count descending.
/// If n <= 0, return all elements sorted by count.
#[no_mangle]
pub extern "C" fn rt_counter_most_common(counter: *mut Obj, n: i64) -> *mut Obj {
    use crate::list::{rt_list_push, rt_make_list};
    use crate::object::ELEM_HEAP_OBJ;
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if counter.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let dict = counter as *mut DictObj;
        let entries_len = (*dict).entries_len;

        // Collect all (element, count) pairs
        let mut pairs: Vec<(*mut Obj, i64)> = Vec::new();
        for i in 0..entries_len {
            let entry = (*dict).entries.add(i);
            if !(*entry).key.is_null() {
                let count = unbox_int_value((*entry).value);
                pairs.push(((*entry).key, count));
            }
        }

        // Sort by count descending
        pairs.sort_by(|a, b| b.1.cmp(&a.1));

        // Limit to n items
        let limit = if n <= 0 {
            pairs.len()
        } else {
            (n as usize).min(pairs.len())
        };

        // Create result list of tuples
        let result = rt_make_list(limit as i64, ELEM_HEAP_OBJ);
        for (elem, count) in pairs.iter().take(limit) {
            let tuple = rt_make_tuple(2, ELEM_HEAP_OBJ);
            rt_tuple_set(tuple, 0, *elem);
            let boxed_count = crate::boxing::rt_box_int(*count);
            rt_tuple_set(tuple, 1, boxed_count);
            rt_list_push(result, tuple);
        }

        result
    }
}

/// Counter.total() — sum of all counts
#[no_mangle]
pub extern "C" fn rt_counter_total(counter: *mut Obj) -> i64 {
    if counter.is_null() {
        return 0;
    }

    unsafe {
        let dict = counter as *mut DictObj;
        let entries_len = (*dict).entries_len;
        let mut total: i64 = 0;
        for i in 0..entries_len {
            let entry = (*dict).entries.add(i);
            if !(*entry).key.is_null() {
                total += unbox_int_value((*entry).value);
            }
        }
        total
    }
}

/// Counter.update(iterable) — add counts from iterable
#[no_mangle]
pub extern "C" fn rt_counter_update(counter: *mut Obj, other: *mut Obj) {
    if counter.is_null() || other.is_null() {
        return;
    }

    unsafe {
        // Iterate and add counts
        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(other);
            if elem.is_null() {
                break;
            }

            // Root counter and elem during allocating operations.
            // rt_box_int and rt_dict_set may trigger GC; without rooting,
            // the counter and the current element could be collected.
            let mut roots: [*mut Obj; 2] = [counter, elem];
            let mut frame = ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 2,
                roots: roots.as_mut_ptr(),
            };
            gc_push(&mut frame);

            let dict = counter as *mut DictObj;
            let current = get_count_or_zero(dict, elem);
            let new_count = current + 1;
            let boxed = crate::boxing::rt_box_int(new_count);
            rt_dict_set(counter, elem, boxed);

            gc_pop();
        }
    }
}

/// Counter.subtract(iterable) — subtract counts from iterable
#[no_mangle]
pub extern "C" fn rt_counter_subtract(counter: *mut Obj, other: *mut Obj) {
    if counter.is_null() || other.is_null() {
        return;
    }

    unsafe {
        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(other);
            if elem.is_null() {
                break;
            }

            let dict = counter as *mut DictObj;
            let current = get_count_or_zero(dict, elem);
            let new_count = current - 1;
            let boxed = crate::boxing::rt_box_int(new_count);
            rt_dict_set(counter, elem, boxed);
        }
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

unsafe fn init_empty_counter(dict: *mut DictObj) {
    use std::alloc::{alloc_zeroed, Layout};

    let indices_capacity = 8usize;
    let entries_capacity = indices_capacity;

    (*dict).len = 0;
    (*dict).entries_len = 0;

    let indices_layout = Layout::array::<i64>(indices_capacity).expect("Allocation size overflow");
    let indices_ptr = alloc_zeroed(indices_layout) as *mut i64;
    for i in 0..indices_capacity {
        *indices_ptr.add(i) = -1; // EMPTY_INDEX
    }
    (*dict).indices = indices_ptr;
    (*dict).indices_capacity = indices_capacity;

    let entries_layout =
        Layout::array::<DictEntry>(entries_capacity).expect("Allocation size overflow");
    let entries_ptr = alloc_zeroed(entries_layout) as *mut DictEntry;
    (*dict).entries = entries_ptr;
    (*dict).entries_capacity = entries_capacity;
}

/// Get count for an element, or 0 if not found
unsafe fn get_count_or_zero(dict: *mut DictObj, key: *mut Obj) -> i64 {
    let cap = (*dict).indices_capacity;
    if cap == 0 || (*dict).len == 0 {
        return 0;
    }

    let hash = hash_hashable_obj(key);
    let mask = cap - 1;
    let base = hash as usize;

    for probe in 0..cap {
        let offset = (probe * (probe + 1)) >> 1;
        let slot = (base + offset) & mask;
        let entry_idx = *(*dict).indices.add(slot);

        if entry_idx == -1 {
            return 0;
        }
        if entry_idx == -2 {
            continue;
        }
        let entry = (*dict).entries.add(entry_idx as usize);
        if (*entry).hash == hash && eq_hashable_obj((*entry).key, key) {
            return unbox_int_value((*entry).value);
        }
    }
    0
}

/// Extract integer value from a boxed IntObj
unsafe fn unbox_int_value(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return 0;
    }
    let int_obj = obj as *mut IntObj;
    (*int_obj).value
}
