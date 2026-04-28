//! Set element operations: add, remove, discard, contains, pop, clear, copy, update

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::hash_table_utils::hash_hashable_obj;
use crate::object::{Obj, SetObj, TypeTagKind, TOMBSTONE};
use pyaot_core_defs::Value;

use super::core::{find_set_slot, rt_make_set, set_resize};

/// Add an element to the set
/// If element exists, no change. If not, inserts new element.
pub fn rt_set_add(set: *mut Obj, elem: *mut Obj) {
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
            let is_new = (*entry).elem.0 == 0 || (*entry).elem == TOMBSTONE;
            if is_new {
                (*entry).hash = hash;
                (*entry).elem = pyaot_core_defs::Value(elem as u64);
                (*set_obj).len += 1;
            }
            // If element already exists, do nothing
        }
    }
}
#[export_name = "rt_set_add"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_add_abi(set: Value, elem: Value) {
    rt_set_add(set.unwrap_ptr(), elem.unwrap_ptr())
}

/// Check if element exists in set
/// Returns: 1 (true) or 0 (false)
pub fn rt_set_contains(set: *mut Obj, elem: *mut Obj) -> i8 {
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
#[export_name = "rt_set_contains"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_contains_abi(set: Value, elem: Value) -> i8 {
    rt_set_contains(set.unwrap_ptr(), elem.unwrap_ptr())
}

/// Remove element from set (raises KeyError if missing)
pub fn rt_set_remove(set: *mut Obj, elem: *mut Obj) {
    if set.is_null() || elem.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::KeyError,
                "set.remove() called with null"
            );
        }
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
            raise_exc!(
                crate::exceptions::ExceptionType::KeyError,
                "element not in set"
            );
        }
    }
}
#[export_name = "rt_set_remove"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_remove_abi(set: Value, elem: Value) {
    rt_set_remove(set.unwrap_ptr(), elem.unwrap_ptr())
}

/// Remove element from set if present (no error if missing)
pub fn rt_set_discard(set: *mut Obj, elem: *mut Obj) {
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
#[export_name = "rt_set_discard"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_discard_abi(set: Value, elem: Value) {
    rt_set_discard(set.unwrap_ptr(), elem.unwrap_ptr())
}

/// Clear all elements from set
pub fn rt_set_clear(set: *mut Obj) {
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
            (*entry).elem = pyaot_core_defs::Value(0);
        }
        (*set_obj).len = 0;
    }
}
#[export_name = "rt_set_clear"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_clear_abi(set: Value) {
    rt_set_clear(set.unwrap_ptr())
}

/// Create a shallow copy of set
/// Returns: pointer to new SetObj
pub fn rt_set_copy(set: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

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

        // Root new_set while rt_set_add may trigger GC on resize
        let mut roots: [*mut Obj; 1] = [new_set];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Copy all non-empty, non-tombstone entries
        let capacity = (*src).capacity;
        for i in 0..capacity {
            let src_entry = (*src).entries.add(i);
            let elem = (*src_entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE {
                rt_set_add(roots[0], elem.0 as *mut Obj);
            }
        }

        gc_pop();

        roots[0]
    }
}
#[export_name = "rt_set_copy"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_copy_abi(set: Value) -> Value {
    Value::from_ptr(rt_set_copy(set.unwrap_ptr()))
}

/// Remove and return an arbitrary element from the set
/// Raises KeyError if the set is empty
/// Returns: removed element
pub fn rt_set_pop(set: *mut Obj) -> *mut Obj {
    if set.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::KeyError,
                "pop from an empty set"
            );
        }
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_pop");
        let set_obj = set as *mut SetObj;
        let capacity = (*set_obj).capacity;

        // Find first non-null, non-tombstone element
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE {
                // Mark as tombstone and decrease length
                (*entry).elem = TOMBSTONE;
                (*set_obj).len -= 1;
                return elem.0 as *mut Obj;
            }
        }

        // Set is empty
        raise_exc!(
            crate::exceptions::ExceptionType::KeyError,
            "pop from an empty set"
        );
    }
}
#[export_name = "rt_set_pop"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_pop_abi(set: Value) -> Value {
    Value::from_ptr(rt_set_pop(set.unwrap_ptr()))
}

/// Add all elements from another set
pub fn rt_set_update(set: *mut Obj, other: *mut Obj) {
    if set.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_update");
        debug_assert_type_tag!(other, TypeTagKind::Set, "rt_set_update");

        let other_obj = other as *mut SetObj;
        let capacity = (*other_obj).capacity;

        // Add each element from other set
        for i in 0..capacity {
            let entry = (*other_obj).entries.add(i);
            let elem = (*entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE {
                rt_set_add(set, elem.0 as *mut Obj);
            }
        }
    }
}
#[export_name = "rt_set_update"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_update_abi(set: Value, other: Value) {
    rt_set_update(set.unwrap_ptr(), other.unwrap_ptr())
}

/// Update set to intersection with another set
pub fn rt_set_intersection_update(set: *mut Obj, other: *mut Obj) {
    if set.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_intersection_update");
        debug_assert_type_tag!(other, TypeTagKind::Set, "rt_set_intersection_update");

        let set_obj = set as *mut SetObj;
        let capacity = (*set_obj).capacity;

        // Remove elements from set that are not in other
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE && rt_set_contains(other, elem.0 as *mut Obj) == 0 {
                // Not in other, remove from set
                (*entry).elem = TOMBSTONE;
                (*set_obj).len -= 1;
            }
        }
    }
}
#[export_name = "rt_set_intersection_update"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_intersection_update_abi(set: Value, other: Value) {
    rt_set_intersection_update(set.unwrap_ptr(), other.unwrap_ptr())
}

/// Update set to difference (remove elements in another set)
pub fn rt_set_difference_update(set: *mut Obj, other: *mut Obj) {
    if set.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_difference_update");
        debug_assert_type_tag!(other, TypeTagKind::Set, "rt_set_difference_update");

        let set_obj = set as *mut SetObj;
        let capacity = (*set_obj).capacity;

        // Remove elements from set that are in other
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE && rt_set_contains(other, elem.0 as *mut Obj) != 0 {
                // In other, remove from set
                (*entry).elem = TOMBSTONE;
                (*set_obj).len -= 1;
            }
        }
    }
}
#[export_name = "rt_set_difference_update"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_difference_update_abi(set: Value, other: Value) {
    rt_set_difference_update(set.unwrap_ptr(), other.unwrap_ptr())
}

/// Update set to symmetric difference
pub fn rt_set_symmetric_difference_update(set: *mut Obj, other: *mut Obj) {
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

        let other_obj = other as *mut SetObj;

        // Collect elements from other that are NOT in set (before modifying set)
        let other_capacity = (*other_obj).capacity;
        let mut to_add: Vec<*mut Obj> = Vec::new();
        for i in 0..other_capacity {
            let entry = (*other_obj).entries.add(i);
            let elem = (*entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE && rt_set_contains(set, elem.0 as *mut Obj) == 0 {
                to_add.push(elem.0 as *mut Obj);
            }
        }

        // Remove elements from set that are in other
        let set_obj = set as *mut SetObj;
        let capacity = (*set_obj).capacity;
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE && rt_set_contains(other, elem.0 as *mut Obj) != 0 {
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
#[export_name = "rt_set_symmetric_difference_update"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_symmetric_difference_update_abi(set: Value, other: Value) {
    rt_set_symmetric_difference_update(set.unwrap_ptr(), other.unwrap_ptr())
}
