//! Counter runtime support
//!
//! Counter is a dict subclass for counting hashable objects.
//! Elements are stored as dict keys and their counts as values.
//! Uses the same DictObj layout with TypeTagKind::Counter.

use crate::dict::rt_dict_set;
use crate::gc::{self, gc_pop, gc_push, ShadowFrame};
use crate::hash_table_utils::{eq_hashable_obj, hash_hashable_obj};
use crate::object::{DictEntry, DictObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

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
/// The iterator elements are used as dict keys.
#[no_mangle]
pub extern "C" fn rt_make_counter_from_iter(iter: *mut Obj) -> *mut Obj {
    let obj = rt_make_counter_empty();

    if iter.is_null() {
        return obj;
    }

    unsafe {
        // Root both obj (counter) and iter for the entire loop.
        // rt_iter_next_no_exc may allocate (e.g., string iterators call
        // rt_str_getchar → rt_make_str → gc_alloc), sweeping both obj and iter
        // if they are not on the shadow stack.  elem is stored at roots[2] so
        // it stays alive across allocating calls.
        let mut roots: [*mut Obj; 3] = [obj, iter, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate and count
        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(roots[1]);
            if elem.is_null() {
                break;
            }

            // Store elem in roots[2] so it stays alive across allocating calls.
            roots[2] = elem;

            // Get current count (or 0 if not found) then update.
            let dict = roots[0] as *mut DictObj;
            let current = get_count_or_zero(dict, roots[2]);
            let new_count = current + 1;

            // Store new count as boxed int; re-read roots after every allocating call.
            let boxed_count =
                pyaot_core_defs::Value::from_int(new_count).0 as *mut crate::object::Obj;
            rt_dict_set(roots[0], roots[2], boxed_count);
        }

        gc_pop();
    }

    // Return the rooted counter pointer (same address; non-moving GC).
    obj
}

/// Counter.most_common(n) — return list of (element, count) tuples, sorted by count descending.
/// If n <= 0, return all elements sorted by count.
#[no_mangle]
pub extern "C" fn rt_counter_most_common(counter: *mut Obj, n: i64) -> *mut Obj {
    use crate::list::{rt_list_push, rt_make_list};
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if counter.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let dict = counter as *mut DictObj;
        let entries_len = (*dict).entries_len;

        // Collect all (element, count) pairs into a Vec.
        // Counts (i64) are unboxed so they don't need GC protection.
        // Key pointers are raw; we must keep `counter` alive so the keys
        // remain reachable while we build the result list below.
        let mut pairs: Vec<(*mut Obj, i64)> = Vec::new();
        for i in 0..entries_len {
            let entry = (*dict).entries.add(i);
            if (*entry).key.0 != 0 {
                let count = unbox_int_value((*entry).value.0 as *mut Obj);
                pairs.push(((*entry).key.0 as *mut Obj, count));
            }
        }

        // Sort by count descending (no allocation — safe)
        pairs.sort_by(|a, b| b.1.cmp(&a.1));

        // Limit to n items
        let limit = if n <= 0 {
            pairs.len()
        } else {
            (n as usize).min(pairs.len())
        };

        // Root counter and result list across all allocating calls.
        // counter must stay alive to keep the key pointers in `pairs` valid.
        // result must stay alive across rt_make_tuple / rt_list_push calls.
        // tuple and elem are stored at roots[2]/roots[3] to survive allocating calls.
        let result = rt_make_list(limit as i64);
        let mut roots: [*mut Obj; 4] =
            [counter, result, std::ptr::null_mut(), std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 4,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        #[allow(clippy::needless_range_loop)]
        for i in 0..limit {
            let (elem, count) = pairs[i];

            roots[3] = elem; // keep the key alive across tuple allocation
            let tuple = rt_make_tuple(2);
            roots[2] = tuple; // keep tuple alive across allocating calls

            rt_tuple_set(roots[2], 0, roots[3]);
            let boxed_count = pyaot_core_defs::Value::from_int(count).0 as *mut crate::object::Obj;
            rt_tuple_set(roots[2], 1, boxed_count);
            rt_list_push(roots[1], roots[2]);
        }

        gc_pop();

        roots[1] // result
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
            if (*entry).key.0 != 0 {
                total += unbox_int_value((*entry).value.0 as *mut Obj);
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
        // Root counter and the iterator (other) for the entire loop so neither
        // is swept by a GC triggered inside rt_iter_next_no_exc.  elem is kept
        // alive at roots[2] across the per-iter Value-tagging and rt_dict_set
        // calls.
        let mut roots: [*mut Obj; 3] = [counter, other, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(roots[1]);
            if elem.is_null() {
                break;
            }
            roots[2] = elem;

            let dict = roots[0] as *mut DictObj;
            let current = get_count_or_zero(dict, roots[2]);
            let new_count = current + 1;
            let boxed = pyaot_core_defs::Value::from_int(new_count).0 as *mut crate::object::Obj;
            rt_dict_set(roots[0], roots[2], boxed);
        }

        gc_pop();
    }
}

/// Counter.subtract(iterable) — subtract counts from iterable
#[no_mangle]
pub extern "C" fn rt_counter_subtract(counter: *mut Obj, other: *mut Obj) {
    if counter.is_null() || other.is_null() {
        return;
    }

    unsafe {
        // Root counter and the iterator (other) for the entire loop so neither
        // is swept by a GC triggered inside rt_iter_next_no_exc.
        let mut roots: [*mut Obj; 3] = [counter, other, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(roots[1]);
            if elem.is_null() {
                break;
            }
            roots[2] = elem;

            let dict = roots[0] as *mut DictObj;
            let current = get_count_or_zero(dict, roots[2]);
            let new_count = current - 1;
            let boxed = pyaot_core_defs::Value::from_int(new_count).0 as *mut crate::object::Obj;
            rt_dict_set(roots[0], roots[2], boxed);
        }

        gc_pop();
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
        if (*entry).hash == hash && eq_hashable_obj((*entry).key.0 as *mut Obj, key) {
            return unbox_int_value((*entry).value.0 as *mut Obj);
        }
    }
    0
}

/// Extract integer value from a tagged Value or boxed IntObj
unsafe fn unbox_int_value(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return 0;
    }
    Value(obj as u64).unwrap_int()
}
