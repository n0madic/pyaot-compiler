//! Core set operations: creation, find_slot, resize, finalization, min/max

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::hash_table_utils::find_slot_generic;
use crate::object::{Obj, SetObj, TypeTagKind, TOMBSTONE};

/// Round up to the next power of 2 (required for mask-based probing).
/// Returns the smallest power of 2 that is >= n.
#[inline]
pub(super) fn next_power_of_2(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    if n.is_power_of_two() {
        return n;
    }
    n.next_power_of_two()
}

pub(super) fn find_set_slot(set: *mut SetObj, elem: *mut Obj, hash: u64, for_insert: bool) -> i64 {
    unsafe {
        find_slot_generic(
            (*set).capacity,
            hash,
            for_insert,
            |i| (*(*set).entries.add(i)).elem,
            |i| (*(*set).entries.add(i)).hash,
            TOMBSTONE,
            elem,
        )
    }
}

pub(super) fn set_resize(set: *mut SetObj, new_capacity: usize) {
    use crate::object::SetEntry;
    use std::alloc::{alloc_zeroed, dealloc, Layout};

    unsafe {
        let old_entries = (*set).entries;
        let old_capacity = (*set).capacity;

        // Allocate new entries array
        let new_layout = Layout::array::<SetEntry>(new_capacity)
            .expect("Allocation size overflow - capacity too large");
        let new_entries = alloc_zeroed(new_layout) as *mut SetEntry;

        // Initialize new entries
        for i in 0..new_capacity {
            let entry = new_entries.add(i);
            (*entry).hash = 0;
            (*entry).elem = std::ptr::null_mut();
        }

        // Update set with new entries
        (*set).entries = new_entries;
        (*set).capacity = new_capacity;
        (*set).len = 0;

        // Rehash existing entries using triangular probing (power-of-2 capacity)
        let mask = new_capacity - 1;
        for i in 0..old_capacity {
            let old_entry = old_entries.add(i);
            let elem = (*old_entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = (*old_entry).hash;
                let base = hash as usize;

                // Find empty slot in new table using triangular probing.
                // offset = i*(i+1)/2: 0, 1, 3, 6, 10, 15, ...
                // With a power-of-2 capacity and a load factor <= 2/3, triangular
                // probing visits every slot exactly once before cycling, so an empty
                // slot is always found within `new_capacity` steps.  The bound check
                // below is a defensive safety guard; it should never trigger in
                // practice because set_resize is only called when there is spare
                // capacity, and the new_capacity is always a power of 2.
                let mut probe_i = 0usize;
                loop {
                    if probe_i >= new_capacity {
                        // This can only happen if new_capacity is too small for the
                        // existing elements, which indicates a bug in the caller.
                        // Panic here rather than looping forever.
                        panic!(
                            "set_resize: failed to find empty slot after {} probes \
                             (new_capacity={}, len={}); resize target too small",
                            probe_i,
                            new_capacity,
                            (*set).len,
                        );
                    }
                    let offset = (probe_i * (probe_i + 1)) >> 1;
                    let index = (base + offset) & mask;
                    let entry = new_entries.add(index);
                    if (*entry).elem.is_null() {
                        (*entry).hash = hash;
                        (*entry).elem = elem;
                        (*set).len += 1;
                        break;
                    }
                    probe_i += 1;
                }
            }
        }

        // Free old entries array
        if old_capacity > 0 && !old_entries.is_null() {
            let old_layout = Layout::array::<SetEntry>(old_capacity)
                .expect("Allocation size overflow - capacity too large");
            dealloc(old_entries as *mut u8, old_layout);
        }
    }
}

/// Allocate a new empty set with given initial capacity
/// Returns: pointer to SetObj
#[no_mangle]
pub extern "C" fn rt_make_set(capacity: i64) -> *mut Obj {
    use crate::object::{SetEntry, SetObj, TypeTagKind};
    use std::alloc::{alloc_zeroed, Layout};

    // Ensure capacity is power of 2 for efficient mask-based probing
    let requested = if capacity <= 0 {
        8
    } else {
        capacity.max(8) as usize
    };
    let capacity = next_power_of_2(requested);

    // Allocate SetObj using GC
    let set_size = std::mem::size_of::<SetObj>();
    let obj = gc::gc_alloc(set_size, TypeTagKind::Set as u8);

    unsafe {
        let set = obj as *mut SetObj;
        (*set).len = 0;
        (*set).capacity = capacity;

        // Allocate entries array separately
        let entries_layout = Layout::array::<SetEntry>(capacity)
            .expect("Allocation size overflow - capacity too large");
        let entries_ptr = alloc_zeroed(entries_layout) as *mut SetEntry;
        (*set).entries = entries_ptr;

        // Initialize all entries to empty (null elements)
        for i in 0..capacity {
            let entry = entries_ptr.add(i);
            (*entry).hash = 0;
            (*entry).elem = std::ptr::null_mut();
        }
    }

    obj
}

/// Get length of set (number of elements)
#[no_mangle]
pub extern "C" fn rt_set_len(set: *mut Obj) -> i64 {
    if set.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_len");
        let set_obj = set as *mut SetObj;
        (*set_obj).len as i64
    }
}

/// Finalize a set by freeing its entries array
/// Called by GC during sweep phase before freeing the SetObj itself
///
/// # Safety
/// The caller must ensure that `set` is a valid pointer to a SetObj
/// that is about to be deallocated.
pub unsafe fn set_finalize(set: *mut Obj) {
    use crate::object::SetEntry;
    use std::alloc::{dealloc, Layout};

    if set.is_null() {
        return;
    }

    let set_obj = set as *mut SetObj;
    let entries = (*set_obj).entries;
    let capacity = (*set_obj).capacity;

    // Free the entries array if allocated
    if !entries.is_null() && capacity > 0 {
        let entries_layout = Layout::array::<SetEntry>(capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(entries as *mut u8, entries_layout);
    }
}

/// Find minimum element in an integer set
/// Returns the minimum i64 value, or 0 if set is empty
#[no_mangle]
pub extern "C" fn rt_set_min_int(set: *mut Obj) -> i64 {
    use crate::object::IntObj;

    if set.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_min_int");
        let set_obj = set as *mut SetObj;
        let len = (*set_obj).len;
        let capacity = (*set_obj).capacity;

        if len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }

        let entries = (*set_obj).entries;
        let mut min_val: Option<i64> = None;

        for i in 0..capacity {
            let entry = entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let int_obj = elem as *mut IntObj;
                let val = (*int_obj).value;
                match min_val {
                    None => min_val = Some(val),
                    Some(current_min) if val < current_min => min_val = Some(val),
                    _ => {}
                }
            }
        }

        min_val.unwrap_or(0)
    }
}

/// Find maximum element in an integer set
/// Returns the maximum i64 value, or 0 if set is empty
#[no_mangle]
pub extern "C" fn rt_set_max_int(set: *mut Obj) -> i64 {
    use crate::object::IntObj;

    if set.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_max_int");
        let set_obj = set as *mut SetObj;
        let len = (*set_obj).len;
        let capacity = (*set_obj).capacity;

        if len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }

        let entries = (*set_obj).entries;
        let mut max_val: Option<i64> = None;

        for i in 0..capacity {
            let entry = entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let int_obj = elem as *mut IntObj;
                let val = (*int_obj).value;
                match max_val {
                    None => max_val = Some(val),
                    Some(current_max) if val > current_max => max_val = Some(val),
                    _ => {}
                }
            }
        }

        max_val.unwrap_or(0)
    }
}

/// Find minimum element in a float set
/// Returns the minimum f64 value, or 0.0 if set is empty
#[no_mangle]
pub extern "C" fn rt_set_min_float(set: *mut Obj) -> f64 {
    use crate::object::FloatObj;

    if set.is_null() {
        return 0.0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_min_float");
        let set_obj = set as *mut SetObj;
        let len = (*set_obj).len;
        let capacity = (*set_obj).capacity;

        if len == 0 {
            let msg = b"min() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }

        let entries = (*set_obj).entries;
        let mut min_val: Option<f64> = None;

        for i in 0..capacity {
            let entry = entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let float_obj = elem as *mut FloatObj;
                let val = (*float_obj).value;
                match min_val {
                    None => min_val = Some(val),
                    Some(current_min) if val < current_min => min_val = Some(val),
                    _ => {}
                }
            }
        }

        min_val.unwrap_or(0.0)
    }
}

/// Find maximum element in a float set
/// Returns the maximum f64 value, or 0.0 if set is empty
#[no_mangle]
pub extern "C" fn rt_set_max_float(set: *mut Obj) -> f64 {
    use crate::object::FloatObj;

    if set.is_null() {
        return 0.0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_max_float");
        let set_obj = set as *mut SetObj;
        let len = (*set_obj).len;
        let capacity = (*set_obj).capacity;

        if len == 0 {
            let msg = b"max() arg is an empty sequence";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }

        let entries = (*set_obj).entries;
        let mut max_val: Option<f64> = None;

        for i in 0..capacity {
            let entry = entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let float_obj = elem as *mut FloatObj;
                let val = (*float_obj).value;
                match max_val {
                    None => max_val = Some(val),
                    Some(current_max) if val > current_max => max_val = Some(val),
                    _ => {}
                }
            }
        }

        max_val.unwrap_or(0.0)
    }
}

/// Find minimum element in a set with key function
#[no_mangle]
pub extern "C" fn rt_set_min_with_key(
    set: *mut Obj,
    key_fn: i64,
    needs_unbox: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    unsafe {
        find_set_extremum_with_key(
            set,
            key_fn,
            needs_unbox,
            captures,
            capture_count as u8,
            true,
        )
    }
}

/// Find maximum element in a set with key function
#[no_mangle]
pub extern "C" fn rt_set_max_with_key(
    set: *mut Obj,
    key_fn: i64,
    needs_unbox: i64,
    captures: *mut Obj,
    capture_count: i64,
) -> *mut Obj {
    unsafe {
        find_set_extremum_with_key(
            set,
            key_fn,
            needs_unbox,
            captures,
            capture_count as u8,
            false,
        )
    }
}

/// Find extremum (min or max) element in a set using a key function
unsafe fn find_set_extremum_with_key(
    set: *mut Obj,
    key_fn: i64,
    needs_unbox: i64,
    captures: *mut Obj,
    capture_count: u8,
    is_min: bool,
) -> *mut Obj {
    use crate::iterator::call_map_with_captures;
    use crate::object::{IntObj, TypeTagKind};
    use crate::sorted::compare_key_values;

    if set.is_null() {
        return std::ptr::null_mut();
    }

    let set_obj = set as *mut SetObj;
    let len = (*set_obj).len;
    let capacity = (*set_obj).capacity;

    if len == 0 {
        let msg = if is_min {
            b"min() arg is an empty sequence"
        } else {
            b"max() arg is an empty sequence"
        };
        crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
    }

    let entries = (*set_obj).entries;
    let mut extremum_elem: *mut Obj = std::ptr::null_mut();
    let mut extremum_key: *mut Obj = std::ptr::null_mut();
    let mut found_first = false;

    // Helper to prepare element for key function based on needs_unbox
    let prepare_elem_for_key = |elem: *mut Obj| -> *mut Obj {
        if needs_unbox != 0 && !elem.is_null() {
            let header = &(*elem).header;
            if header.type_tag == TypeTagKind::Int {
                let int_obj = elem as *mut IntObj;
                let raw_value = (*int_obj).value;
                return raw_value as *mut Obj;
            }
        }
        elem
    };

    // Find extremum by iterating through all valid elements
    for i in 0..capacity {
        let entry = entries.add(i);
        let elem = (*entry).elem;
        if !elem.is_null() && elem != TOMBSTONE {
            if !found_first {
                extremum_elem = elem;
                let key_input = prepare_elem_for_key(elem);
                extremum_key = call_map_with_captures(key_fn, captures, capture_count, key_input);
                found_first = true;
            } else {
                let key_input = prepare_elem_for_key(elem);
                let key = call_map_with_captures(key_fn, captures, capture_count, key_input);

                let cmp = compare_key_values(key, extremum_key);
                let is_better = if is_min {
                    cmp == std::cmp::Ordering::Less
                } else {
                    cmp == std::cmp::Ordering::Greater
                };

                if is_better {
                    extremum_elem = elem;
                    extremum_key = key;
                }
            }
        }
    }

    if !found_first {
        return std::ptr::null_mut();
    }

    extremum_elem
}
