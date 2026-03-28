//! Set operations for Python runtime

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::hash_table_utils::hash_hashable_obj;
use crate::object::{ListObj, Obj, SetObj, TypeTagKind, ELEM_HEAP_OBJ, TOMBSTONE};
use std::alloc::{alloc_zeroed, Layout};

use crate::hash_table_utils::find_slot_generic;

/// Round up to the next power of 2 (required for mask-based probing).
/// Returns the smallest power of 2 that is >= n.
#[inline]
fn next_power_of_2(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    if n.is_power_of_two() {
        return n;
    }
    n.next_power_of_two()
}

fn find_set_slot(
    set: *mut crate::object::SetObj,
    elem: *mut Obj,
    hash: u64,
    for_insert: bool,
) -> i64 {
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

fn set_resize(set: *mut crate::object::SetObj, new_capacity: usize) {
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

/// Add an element to the set
/// If element exists, no change. If not, inserts new element.
#[no_mangle]
pub extern "C" fn rt_set_add(set: *mut Obj, elem: *mut Obj) {
    if set.is_null() || elem.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_add");
        let set_obj = set as *mut SetObj;

        // Check load factor and resize if needed.
        // Count tombstones to prevent table degradation from repeated add/remove cycles:
        // after many deletions the table fills with tombstones, but `len` stays low so
        // the standard len-only check never triggers a resize. We include tombstones in
        // the fill count so the table is compacted once (fill / capacity) >= 0.75.
        // The scan is O(capacity) in the worst case, but that is the same order as the
        // resize itself, so the amortised cost per element remains O(1).
        let mut tombstone_count = 0usize;
        let cap = (*set_obj).capacity;
        if (*set_obj).len * 4 >= cap * 2 {
            // Only scan when len is already above 50% — below that, tombstones alone
            // cannot push fill past 75%, so the cheap len-only check is sufficient.
            let entries = (*set_obj).entries;
            for i in 0..cap {
                if (*entries.add(i)).elem == TOMBSTONE {
                    tombstone_count += 1;
                }
            }
        }
        let fill = (*set_obj).len + tombstone_count;
        if fill * 4 >= cap * 3 {
            let new_capacity = cap * 2;
            set_resize(set_obj, new_capacity);
        }

        let hash = hash_hashable_obj(elem);
        let slot = find_set_slot(set_obj, elem, hash, true);

        if slot >= 0 {
            let entry = (*set_obj).entries.add(slot as usize);
            let is_new = (*entry).elem.is_null() || (*entry).elem == TOMBSTONE;
            if is_new {
                (*entry).hash = hash;
                (*entry).elem = elem;
                (*set_obj).len += 1;
            }
            // If element already exists, do nothing
        }
    }
}

/// Check if element exists in set
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_set_contains(set: *mut Obj, elem: *mut Obj) -> i8 {
    if set.is_null() || elem.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_contains");
        let set_obj = set as *mut SetObj;
        let hash = hash_hashable_obj(elem);
        let slot = find_set_slot(set_obj, elem, hash, false);
        if slot >= 0 {
            1
        } else {
            0
        }
    }
}

/// Remove element from set (raises KeyError if missing)
#[no_mangle]
pub extern "C" fn rt_set_remove(set: *mut Obj, elem: *mut Obj) {
    if set.is_null() || elem.is_null() {
        let msg = b"set.remove() called with null";
        unsafe { crate::exceptions::rt_exc_raise_key_error(msg.as_ptr(), msg.len()) }
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_remove");
        let set_obj = set as *mut SetObj;
        let hash = hash_hashable_obj(elem);
        let slot = find_set_slot(set_obj, elem, hash, false);

        if slot >= 0 {
            let entry = (*set_obj).entries.add(slot as usize);
            // Mark as tombstone
            (*entry).elem = TOMBSTONE;
            (*set_obj).len -= 1;
        } else {
            let msg = b"element not in set";
            crate::exceptions::rt_exc_raise_key_error(msg.as_ptr(), msg.len());
        }
    }
}

/// Remove element from set if present (no error if missing)
#[no_mangle]
pub extern "C" fn rt_set_discard(set: *mut Obj, elem: *mut Obj) {
    if set.is_null() || elem.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_discard");
        let set_obj = set as *mut SetObj;
        let hash = hash_hashable_obj(elem);
        let slot = find_set_slot(set_obj, elem, hash, false);

        if slot >= 0 {
            let entry = (*set_obj).entries.add(slot as usize);
            // Mark as tombstone
            (*entry).elem = TOMBSTONE;
            (*set_obj).len -= 1;
        }
        // If not found, just do nothing (unlike remove)
    }
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

/// Clear all elements from set
#[no_mangle]
pub extern "C" fn rt_set_clear(set: *mut Obj) {
    if set.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_clear");
        let set_obj = set as *mut SetObj;
        let capacity = (*set_obj).capacity;
        let entries = (*set_obj).entries;

        for i in 0..capacity {
            let entry = entries.add(i);
            (*entry).hash = 0;
            (*entry).elem = std::ptr::null_mut();
        }
        (*set_obj).len = 0;
    }
}

/// Create a shallow copy of set
/// Returns: pointer to new SetObj
#[no_mangle]
pub extern "C" fn rt_set_copy(set: *mut Obj) -> *mut Obj {
    if set.is_null() {
        return rt_make_set(8);
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_copy");
        let src = set as *mut SetObj;
        // Size the new set based on the number of live elements rather than the
        // source capacity. This eliminates tombstones and avoids copying a
        // potentially over-sized or tombstone-saturated table.
        let new_capacity = ((*src).len * 4 / 3 + 1).next_power_of_two().max(8);
        let new_set = rt_make_set(new_capacity as i64);

        // Copy all non-empty, non-tombstone entries
        let capacity = (*src).capacity;
        for i in 0..capacity {
            let src_entry = (*src).entries.add(i);
            let elem = (*src_entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                rt_set_add(new_set, elem);
            }
        }

        new_set
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

/// Create a new set with all elements from both sets (union)
/// Returns: pointer to new SetObj containing elements from a and b
#[no_mangle]
pub extern "C" fn rt_set_union(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    if a.is_null() || b.is_null() {
        let msg = b"TypeError: unsupported operand type(s) for set operation";
        unsafe {
            crate::exceptions::rt_exc_raise(
                pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                msg.as_ptr(),
                msg.len(),
            )
        }
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Set, "rt_set_union");
        debug_assert_type_tag!(b, TypeTagKind::Set, "rt_set_union");

        let b_obj = b as *mut SetObj;

        // Copy set a
        let result = rt_set_copy(a);

        // Add all elements from b
        let b_capacity = (*b_obj).capacity;
        for i in 0..b_capacity {
            let entry = (*b_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                rt_set_add(result, elem);
            }
        }

        result
    }
}

/// Create a new set with elements in both sets (intersection)
/// Returns: pointer to new SetObj containing elements in both a and b
#[no_mangle]
pub extern "C" fn rt_set_intersection(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    if a.is_null() || b.is_null() {
        let msg = b"TypeError: unsupported operand type(s) for set operation";
        unsafe {
            crate::exceptions::rt_exc_raise(
                pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                msg.as_ptr(),
                msg.len(),
            )
        }
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Set, "rt_set_intersection");
        debug_assert_type_tag!(b, TypeTagKind::Set, "rt_set_intersection");

        let a_obj = a as *mut SetObj;
        let b_obj = b as *mut SetObj;

        // Create new empty set
        let result = rt_make_set(8);

        // Iterate through a, add elements that are also in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot >= 0 {
                    rt_set_add(result, elem);
                }
            }
        }

        result
    }
}

/// Create a new set with elements in a but not in b (difference)
/// Returns: pointer to new SetObj containing elements in a but not in b
#[no_mangle]
pub extern "C" fn rt_set_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    if a.is_null() || b.is_null() {
        let msg = b"TypeError: unsupported operand type(s) for set operation";
        unsafe {
            crate::exceptions::rt_exc_raise(
                pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                msg.as_ptr(),
                msg.len(),
            )
        }
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Set, "rt_set_difference");
        debug_assert_type_tag!(b, TypeTagKind::Set, "rt_set_difference");

        let a_obj = a as *mut SetObj;
        let b_obj = b as *mut SetObj;

        // Create new empty set
        let result = rt_make_set(8);

        // Iterate through a, add elements that are NOT in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot < 0 {
                    rt_set_add(result, elem);
                }
            }
        }

        result
    }
}

/// Create a new set with elements in exactly one of the sets (symmetric difference)
/// Returns: pointer to new SetObj containing elements in a or b but not both
#[no_mangle]
pub extern "C" fn rt_set_symmetric_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    if a.is_null() || b.is_null() {
        let msg = b"TypeError: unsupported operand type(s) for set operation";
        unsafe {
            crate::exceptions::rt_exc_raise(
                pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                msg.as_ptr(),
                msg.len(),
            )
        }
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Set, "rt_set_symmetric_difference");
        debug_assert_type_tag!(b, TypeTagKind::Set, "rt_set_symmetric_difference");

        let a_obj = a as *mut SetObj;
        let b_obj = b as *mut SetObj;

        // Create new empty set
        let result = rt_make_set(8);

        // Add elements from a that are not in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot < 0 {
                    rt_set_add(result, elem);
                }
            }
        }

        // Add elements from b that are not in a
        let b_capacity = (*b_obj).capacity;
        for i in 0..b_capacity {
            let entry = (*b_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(a_obj, elem, hash, false);
                if slot < 0 {
                    rt_set_add(result, elem);
                }
            }
        }

        result
    }
}

/// Check if all elements of a are in b (subset test)
/// Returns: 1 if a is subset of b, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_set_issubset(a: *mut Obj, b: *mut Obj) -> i8 {
    if a.is_null() || b.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Set, "rt_set_issubset");
        debug_assert_type_tag!(b, TypeTagKind::Set, "rt_set_issubset");

        let a_obj = a as *mut SetObj;
        let b_obj = b as *mut SetObj;

        // Iterate through a, check each element is in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot < 0 {
                    return 0; // Found element in a that is not in b
                }
            }
        }

        1 // All elements of a are in b
    }
}

/// Check if all elements of b are in a (superset test)
/// Returns: 1 if a is superset of b, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_set_issuperset(a: *mut Obj, b: *mut Obj) -> i8 {
    // a is superset of b if b is subset of a
    rt_set_issubset(b, a)
}

/// Check if sets have no elements in common (disjoint test)
/// Returns: 1 if sets are disjoint, 0 otherwise
#[no_mangle]
pub extern "C" fn rt_set_isdisjoint(a: *mut Obj, b: *mut Obj) -> i8 {
    if a.is_null() || b.is_null() {
        return 1;
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Set, "rt_set_isdisjoint");
        debug_assert_type_tag!(b, TypeTagKind::Set, "rt_set_isdisjoint");

        let a_obj = a as *mut SetObj;
        let b_obj = b as *mut SetObj;

        // Iterate through a, check if any element is in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot >= 0 {
                    return 0; // Found element in both sets
                }
            }
        }

        1 // No elements in common
    }
}

// Type alias for key function pointer
type KeyFn = extern "C" fn(*mut Obj) -> *mut Obj;

/// Find minimum element in a set with key function
/// needs_unbox: 0 = pass boxed objects directly (for builtin key functions)
///              1 = unbox integers before passing (for user-defined key functions expecting int)
#[no_mangle]
pub extern "C" fn rt_set_min_with_key(set: *mut Obj, key_fn: KeyFn, needs_unbox: i64) -> *mut Obj {
    unsafe { find_set_extremum_with_key(set, key_fn, needs_unbox, true) }
}

/// Find maximum element in a set with key function
/// needs_unbox: 0 = pass boxed objects directly (for builtin key functions)
///              1 = unbox integers before passing (for user-defined key functions expecting int)
#[no_mangle]
pub extern "C" fn rt_set_max_with_key(set: *mut Obj, key_fn: KeyFn, needs_unbox: i64) -> *mut Obj {
    unsafe { find_set_extremum_with_key(set, key_fn, needs_unbox, false) }
}

/// Find extremum (min or max) element in a set using a key function
///
/// Note: Sets store all elements as heap objects (*mut Obj).
/// - For builtin key functions: pass boxed objects directly (needs_unbox=0)
/// - For user-defined key functions: unbox integers before passing (needs_unbox=1)
unsafe fn find_set_extremum_with_key(
    set: *mut Obj,
    key_fn: KeyFn,
    needs_unbox: i64,
    is_min: bool,
) -> *mut Obj {
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
                // Unbox integer: extract raw i64 and cast as pointer
                // (user-defined functions expect raw values like list[int] elements)
                let int_obj = elem as *mut IntObj;
                let raw_value = (*int_obj).value;
                return raw_value as *mut Obj;
            }
        }
        // For builtins or non-int types, pass boxed object directly
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
                extremum_key = key_fn(key_input);
                found_first = true;
            } else {
                let key_input = prepare_elem_for_key(elem);
                let key = key_fn(key_input);

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

    extremum_elem // Return boxed element (set storage model)
}

/// Convert set to list (for iteration support)
/// Returns: pointer to ListObj containing all set elements
#[no_mangle]
pub extern "C" fn rt_set_to_list(set: *mut Obj) -> *mut Obj {
    if set.is_null() {
        // Return empty list
        let size = std::mem::size_of::<ListObj>();
        let obj = gc::gc_alloc(size, TypeTagKind::List as u8);
        unsafe {
            let list = obj as *mut ListObj;
            (*list).len = 0;
            (*list).capacity = 0;
            (*list).data = std::ptr::null_mut();
        }
        return obj;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_to_list");
        let set_obj = set as *mut SetObj;
        let set_len = (*set_obj).len;
        let capacity = (*set_obj).capacity;

        // Allocate list with set length
        let list_size = std::mem::size_of::<ListObj>();
        let list_obj = gc::gc_alloc(list_size, TypeTagKind::List as u8);
        let list = list_obj as *mut ListObj;

        // Allocate data array
        let data_layout = Layout::array::<*mut Obj>(set_len)
            .expect("Allocation size overflow - capacity too large");
        let data = alloc_zeroed(data_layout) as *mut *mut Obj;

        (*list).elem_tag = ELEM_HEAP_OBJ;
        (*list).len = set_len;
        (*list).capacity = set_len;
        (*list).data = data;

        // Copy non-empty, non-tombstone elements to list
        let mut list_idx = 0;
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                *data.add(list_idx) = elem;
                list_idx += 1;
            }
        }

        list_obj
    }
}

/// Remove and return an arbitrary element from the set
/// Raises KeyError if the set is empty
/// Returns: removed element
#[no_mangle]
pub extern "C" fn rt_set_pop(set: *mut Obj) -> *mut Obj {
    if set.is_null() {
        unsafe {
            let msg = b"pop from an empty set";
            crate::exceptions::rt_exc_raise_key_error(msg.as_ptr(), msg.len());
        }
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_pop");
        let set_obj = set as *mut crate::object::SetObj;
        let capacity = (*set_obj).capacity;

        // Find first non-null, non-tombstone element
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                // Mark as tombstone and decrease length
                (*entry).elem = TOMBSTONE;
                (*set_obj).len -= 1;
                return elem;
            }
        }

        // Set is empty
        let msg = b"pop from an empty set";
        crate::exceptions::rt_exc_raise_key_error(msg.as_ptr(), msg.len());
    }
}

/// Add all elements from another set
#[no_mangle]
pub extern "C" fn rt_set_update(set: *mut Obj, other: *mut Obj) {
    if set.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_update");
        debug_assert_type_tag!(other, TypeTagKind::Set, "rt_set_update");

        let other_obj = other as *mut crate::object::SetObj;
        let capacity = (*other_obj).capacity;

        // Add each element from other set
        for i in 0..capacity {
            let entry = (*other_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                rt_set_add(set, elem);
            }
        }
    }
}

/// Update set to intersection with another set
#[no_mangle]
pub extern "C" fn rt_set_intersection_update(set: *mut Obj, other: *mut Obj) {
    if set.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_intersection_update");
        debug_assert_type_tag!(other, TypeTagKind::Set, "rt_set_intersection_update");

        let set_obj = set as *mut crate::object::SetObj;
        let capacity = (*set_obj).capacity;

        // Remove elements from set that are not in other
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE && rt_set_contains(other, elem) == 0 {
                // Not in other, remove from set
                (*entry).elem = TOMBSTONE;
                (*set_obj).len -= 1;
            }
        }
    }
}

/// Update set to difference (remove elements in another set)
#[no_mangle]
pub extern "C" fn rt_set_difference_update(set: *mut Obj, other: *mut Obj) {
    if set.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_difference_update");
        debug_assert_type_tag!(other, TypeTagKind::Set, "rt_set_difference_update");

        let set_obj = set as *mut crate::object::SetObj;
        let capacity = (*set_obj).capacity;

        // Remove elements from set that are in other
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE && rt_set_contains(other, elem) != 0 {
                // In other, remove from set
                (*entry).elem = TOMBSTONE;
                (*set_obj).len -= 1;
            }
        }
    }
}

/// Update set to symmetric difference
#[no_mangle]
pub extern "C" fn rt_set_symmetric_difference_update(set: *mut Obj, other: *mut Obj) {
    if set.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_symmetric_difference_update");
        debug_assert_type_tag!(
            other,
            TypeTagKind::Set,
            "rt_set_symmetric_difference_update"
        );

        let other_obj = other as *mut crate::object::SetObj;

        // Collect elements from other that are NOT in set (before modifying set)
        let other_capacity = (*other_obj).capacity;
        let mut to_add: Vec<*mut Obj> = Vec::new();
        for i in 0..other_capacity {
            let entry = (*other_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE && rt_set_contains(set, elem) == 0 {
                to_add.push(elem);
            }
        }

        // Remove elements from set that are in other
        let set_obj = set as *mut crate::object::SetObj;
        let capacity = (*set_obj).capacity;
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE && rt_set_contains(other, elem) != 0 {
                (*entry).elem = TOMBSTONE;
                (*set_obj).len -= 1;
            }
        }

        // Add collected elements
        for elem in to_add {
            rt_set_add(set, elem);
        }
    }
}
