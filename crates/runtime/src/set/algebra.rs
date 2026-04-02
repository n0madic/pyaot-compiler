//! Set algebra operations: union, intersection, difference, symmetric_difference

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::hash_table_utils::hash_hashable_obj;
use crate::object::{Obj, SetObj, TypeTagKind, TOMBSTONE};

use super::core::{find_set_slot, rt_make_set};
use super::ops::{rt_set_add, rt_set_copy};

/// Create a new set with all elements from both sets (union)
/// Returns: pointer to new SetObj containing elements from a and b
#[no_mangle]
pub extern "C" fn rt_set_union(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

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

        // Copy set a — result must be rooted before rt_set_add may trigger GC
        let result = rt_set_copy(a);

        let mut roots: [*mut Obj; 1] = [result];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Add all elements from b
        let b_capacity = (*b_obj).capacity;
        for i in 0..b_capacity {
            let entry = (*b_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                rt_set_add(roots[0], elem);
            }
        }

        gc_pop();

        roots[0]
    }
}

/// Create a new set with elements in both sets (intersection)
/// Returns: pointer to new SetObj containing elements in both a and b
#[no_mangle]
pub extern "C" fn rt_set_intersection(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

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

        // Create new empty set; root it while rt_set_add may trigger GC
        let result = rt_make_set(8);

        let mut roots: [*mut Obj; 1] = [result];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate through a, add elements that are also in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot >= 0 {
                    rt_set_add(roots[0], elem);
                }
            }
        }

        gc_pop();

        roots[0]
    }
}

/// Create a new set with elements in a but not in b (difference)
/// Returns: pointer to new SetObj containing elements in a but not in b
#[no_mangle]
pub extern "C" fn rt_set_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

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

        // Create new empty set; root it while rt_set_add may trigger GC
        let result = rt_make_set(8);

        let mut roots: [*mut Obj; 1] = [result];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate through a, add elements that are NOT in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot < 0 {
                    rt_set_add(roots[0], elem);
                }
            }
        }

        gc_pop();

        roots[0]
    }
}

/// Create a new set with elements in exactly one of the sets (symmetric difference)
/// Returns: pointer to new SetObj containing elements in a or b but not both
#[no_mangle]
pub extern "C" fn rt_set_symmetric_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

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

        // Create new empty set; root it while rt_set_add may trigger GC
        let result = rt_make_set(8);

        let mut roots: [*mut Obj; 1] = [result];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Add elements from a that are not in b
        let a_capacity = (*a_obj).capacity;
        for i in 0..a_capacity {
            let entry = (*a_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                let hash = hash_hashable_obj(elem);
                let slot = find_set_slot(b_obj, elem, hash, false);
                if slot < 0 {
                    rt_set_add(roots[0], elem);
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
                    rt_set_add(roots[0], elem);
                }
            }
        }

        gc_pop();

        roots[0]
    }
}
