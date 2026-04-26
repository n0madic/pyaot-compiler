//! Set comparison operations: issubset, issuperset, isdisjoint

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::hash_table_utils::hash_hashable_obj;
use crate::object::{Obj, SetObj, TypeTagKind, TOMBSTONE};

use super::core::find_set_slot;

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
            if elem.0 != 0 && elem != TOMBSTONE {
                let elem_ptr = elem.0 as *mut Obj;
                let hash = hash_hashable_obj(elem_ptr);
                let slot = find_set_slot(b_obj, elem_ptr, hash, false);
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
            if elem.0 != 0 && elem != TOMBSTONE {
                let elem_ptr = elem.0 as *mut Obj;
                let hash = hash_hashable_obj(elem_ptr);
                let slot = find_set_slot(b_obj, elem_ptr, hash, false);
                if slot >= 0 {
                    return 0; // Found element in both sets
                }
            }
        }

        1 // No elements in common
    }
}
