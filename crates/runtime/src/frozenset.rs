//! frozenset runtime support — an immutable, hashable set.
//!
//! Physically a [`SetObj`](crate::object::SetObj) (identical memory layout)
//! tagged [`TypeTagKind::FrozenSet`]. Every read-only set primitive (len,
//! contains, iter, eq, subset/superset/disjoint, union/intersection/difference/
//! symmetric_difference, copy) operates on it unchanged via the set-family seam
//! guard (`debug_assert_set_family!`); only the tag differs. Construction builds
//! a regular `Set` and retags it to `FrozenSet` as the final step, so the
//! MUTATING set primitives (`rt_set_add`) only ever observe a `Set`.
//!
//! The one genuinely new capability is the content hash: a frozenset is
//! hashable (a `set` is not), so it can be a dict key / set element / element
//! of another frozenset. `frozenset_content_hash` is the order-independent
//! XOR-fold (CPython `frozenset_hash` algorithm) wired into
//! `hash_table_utils::hash_hashable_obj` and the `hash()` builtin.

use crate::gc::{gc_pop, gc_push, ShadowFrame};
use crate::object::{Obj, SetObj, TypeTagKind, TOMBSTONE};
use crate::set::{rt_make_set, rt_set_add};
use pyaot_core_defs::Value;

/// Retag a freshly-built `Set` as a `FrozenSet`. Sound ONLY on an object that
/// nothing else aliases yet (a brand-new construction / algebra result): the
/// memory layout is identical and the GC trace + finalize treat both tags
/// the same, so flipping the header tag is the entire conversion.
#[inline]
unsafe fn retag_frozenset(obj: *mut Obj) -> *mut Obj {
    if !obj.is_null() {
        (*obj).header.type_tag = TypeTagKind::FrozenSet;
    }
    obj
}

/// `frozenset()` — empty.
pub fn rt_make_frozenset_empty() -> *mut Obj {
    unsafe { retag_frozenset(rt_make_set(8)) }
}
#[export_name = "rt_make_frozenset_empty"]
pub extern "C" fn rt_make_frozenset_empty_abi() -> Value {
    Value::from_ptr(rt_make_frozenset_empty())
}

/// `frozenset(iterable)` — build a set from the iterable, then freeze it. The
/// argument is normalized to an iterator internally (idempotent for objects that
/// already are iterators), so the frontend passes the raw iterable.
pub fn rt_make_frozenset_from_iter(iterable: *mut Obj) -> *mut Obj {
    let set = rt_make_set(8);

    if iterable.is_null() {
        return unsafe { retag_frozenset(set) };
    }

    unsafe {
        // Root the set, the iterable, and the current element across the loop —
        // rt_iter_value_dyn / rt_iter_next_no_exc / rt_set_add (resize) may
        // allocate and trigger a GC sweep.
        let mut roots: [*mut Obj; 3] = [set, iterable, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        roots[1] = crate::iterator::rt_iter_value_dyn(roots[1]);

        loop {
            let elem = crate::iterator::rt_iter_next_no_exc(roots[1]);
            if elem.is_null() {
                break;
            }
            roots[2] = elem;
            rt_set_add(roots[0], roots[2]);
        }

        gc_pop();

        retag_frozenset(roots[0])
    }
}
#[export_name = "rt_make_frozenset_from_iter"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_frozenset_from_iter_abi(iterable: Value) -> Value {
    Value::from_ptr(rt_make_frozenset_from_iter(iterable.unwrap_ptr()))
}

/// `fs | other` / `fs.union(other)` — a new frozenset of the union.
pub fn rt_frozenset_union(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe { retag_frozenset(crate::set::rt_set_union(a, b)) }
}
#[export_name = "rt_frozenset_union"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_frozenset_union_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_frozenset_union(a.unwrap_ptr(), b.unwrap_ptr()))
}

/// `fs & other` / `fs.intersection(other)` — a new frozenset of the intersection.
pub fn rt_frozenset_intersection(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe { retag_frozenset(crate::set::rt_set_intersection(a, b)) }
}
#[export_name = "rt_frozenset_intersection"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_frozenset_intersection_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_frozenset_intersection(a.unwrap_ptr(), b.unwrap_ptr()))
}

/// `fs - other` / `fs.difference(other)` — a new frozenset of the difference.
pub fn rt_frozenset_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe { retag_frozenset(crate::set::rt_set_difference(a, b)) }
}
#[export_name = "rt_frozenset_difference"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_frozenset_difference_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_frozenset_difference(a.unwrap_ptr(), b.unwrap_ptr()))
}

/// `fs ^ other` / `fs.symmetric_difference(other)` — a new frozenset.
pub fn rt_frozenset_symmetric_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    unsafe { retag_frozenset(crate::set::rt_set_symmetric_difference(a, b)) }
}
#[export_name = "rt_frozenset_symmetric_difference"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_frozenset_symmetric_difference_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_frozenset_symmetric_difference(
        a.unwrap_ptr(),
        b.unwrap_ptr(),
    ))
}

/// `fs.copy()` — CPython returns a new frozenset (immutable, so it may share,
/// but a fresh object is always correct). Mirrors `set.copy` + retag.
pub fn rt_frozenset_copy(s: *mut Obj) -> *mut Obj {
    unsafe { retag_frozenset(crate::set::rt_set_copy(s)) }
}
#[export_name = "rt_frozenset_copy"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_frozenset_copy_abi(s: Value) -> Value {
    Value::from_ptr(rt_frozenset_copy(s.unwrap_ptr()))
}

/// Order-independent content hash of a frozenset — CPython's `frozenset_hash`
/// XOR-fold over the per-element hashes (stored in each `SetEntry.hash`). Used
/// by `hash_table_utils::hash_hashable_obj` (frozenset as dict key / set
/// element) and the `hash()` builtin, so that two equal frozensets hash equal
/// regardless of element insertion order.
///
/// # Safety
/// `obj` must be null or a valid `FrozenSet`/`Set`-layout pointer.
pub unsafe fn frozenset_content_hash(obj: *mut Obj) -> u64 {
    if obj.is_null() {
        return 0;
    }
    let set = obj as *mut SetObj;
    let capacity = (*set).capacity;
    let entries = (*set).entries;

    let mut hash: u64 = 0;
    for i in 0..capacity {
        let entry = entries.add(i);
        let elem = (*entry).elem;
        if elem.0 != 0 && elem != TOMBSTONE {
            let h = (*entry).hash;
            hash ^= (h ^ 89869747u64 ^ (h << 16)).wrapping_mul(3644798167u64);
        }
    }
    let n = (*set).len as u64;
    hash ^= (n.wrapping_add(1)).wrapping_mul(1927868237u64);
    hash ^= (hash >> 11) ^ (hash >> 25);
    hash = hash.wrapping_mul(69069u64).wrapping_add(907133923u64);
    hash
}

/// `rt_frozenset_hash(fs) -> i64` — the boxed-int-free raw hash, exposed for the
/// `hash()` builtin path.
pub fn rt_frozenset_hash(obj: *mut Obj) -> i64 {
    unsafe { frozenset_content_hash(obj) as i64 }
}
#[export_name = "rt_frozenset_hash"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_frozenset_hash_abi(obj: Value) -> i64 {
    rt_frozenset_hash(obj.unwrap_ptr())
}
