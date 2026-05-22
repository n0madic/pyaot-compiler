//! Dictionary comparison operations: structural equality.

use crate::hash_table_utils::hash_hashable_obj;
use crate::object::{DictObj, Obj};
use pyaot_core_defs::Value;

use super::core::lookup_entry;

/// Compare two dicts for structural equality (Python `==`).
///
/// Two dicts are equal iff they have the same number of active entries and
/// every key of `a` is present in `b` with an equal value
/// (order-independent). Values are compared structurally via `rt_obj_eq`
/// so nested containers compare by content, not identity. Returns 1 if
/// equal, 0 otherwise.
///
/// Works on any `DictObj`-layout object (plain dict, `DefaultDict` and
/// `Counter`) — only `len` / `entries` / `entries_len` / the index table
/// are read.
pub fn rt_dict_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    if a.is_null() && b.is_null() {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }
    unsafe {
        let da = a as *mut DictObj;
        let db = b as *mut DictObj;
        // Equal active-entry count is necessary; combined with "every key
        // of `a` is present in `b` with an equal value" it is sufficient.
        if (*da).len != (*db).len {
            return 0;
        }
        // Walk `a`'s dense entries array (insertion order); skip deleted
        // slots, which carry `DictEntry.key == Value(0)`.
        for i in 0..(*da).entries_len {
            let entry = (*da).entries.add(i);
            let key = (*entry).key;
            if key.0 == 0 {
                continue;
            }
            let key_ptr = key.0 as *mut Obj;
            let hash = hash_hashable_obj(key_ptr);
            let b_idx = lookup_entry(db, key_ptr, hash);
            if b_idx < 0 {
                return 0; // key missing from `b`
            }
            let a_value = (*entry).value;
            let b_value = (*(*db).entries.add(b_idx as usize)).value;
            if crate::ops::rt_obj_eq(a_value.0 as *mut Obj, b_value.0 as *mut Obj) == 0 {
                return 0;
            }
        }
        1
    }
}

#[export_name = "rt_dict_eq"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_eq_abi(a: Value, b: Value) -> i8 {
    rt_dict_eq(a.unwrap_ptr(), b.unwrap_ptr())
}
