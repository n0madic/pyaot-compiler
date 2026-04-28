//! Dictionary get, set, delete, contains, update, merge, pop, setdefault operations

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::hash_table_utils::{eq_hashable_obj, hash_hashable_obj};
use crate::object::{DictObj, Obj, StrObj, TypeTagKind};
use crate::string::rt_make_str_interned;
use pyaot_core_defs::Value;

use super::core::{
    dict_resize, find_insert_slot, lookup_entry, real_entries_capacity, rt_make_dict, DUMMY_INDEX,
    EMPTY_INDEX, MAX_DICT_KEY_INTERN_LENGTH,
};

/// Set a key-value pair in the dictionary
/// If key exists, updates value. If not, inserts new entry.
/// String keys under 256 bytes are interned for memory efficiency.
pub fn rt_dict_set(dict: *mut Obj, mut key: *mut Obj, value: *mut Obj) {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() || key.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_set");

        // Intern string keys under the size threshold.
        //
        // rt_make_str_interned calls gc_alloc which may trigger a full collection.
        // Root dict, key, and value across the call so none of them are freed.
        // `data` is an interior pointer into `key`, so `key` must stay alive.
        //
        // Guard: tagged primitives (int/bool/none, bit0=1) are not heap objects;
        // skip the dereference entirely and go straight to hash/lookup.
        if pyaot_core_defs::Value(key as u64).is_ptr() && (*key).header.type_tag == TypeTagKind::Str
        {
            let str_obj = key as *mut StrObj;
            let len = (*str_obj).len;

            if len < MAX_DICT_KEY_INTERN_LENGTH {
                let data = (*str_obj).data.as_ptr();

                // Root dict, key, value across gc_alloc inside rt_make_str_interned.
                // value may be null (a valid dict value meaning None); include it
                // anyway since the GC shadow frame accepts null roots.
                let mut roots: [*mut Obj; 3] = [dict, key, value];
                let mut frame = ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 3,
                    roots: roots.as_mut_ptr(),
                };
                gc_push(&mut frame);
                let interned = rt_make_str_interned(data, len);
                gc_pop();

                // Use the interned key from now on.
                key = interned;
            }
        }

        let dict_obj = dict as *mut DictObj;

        // Check if we need to resize (len * 3 >= indices_capacity * 2 → >66% full)
        // Use live count (len), not entries_len which includes tombstones
        if (*dict_obj).len * 3 >= (*dict_obj).indices_capacity * 2 {
            dict_resize(dict_obj);
        }

        let hash = hash_hashable_obj(key);
        let (slot, entry_idx) = find_insert_slot(dict_obj, key, hash);

        if entry_idx >= 0 {
            // Key exists — update value in place
            let entry = (*dict_obj).entries.add(entry_idx as usize);
            (*entry).value = pyaot_core_defs::Value(value as u64);
        } else {
            // New key — append to entries array
            let new_idx = (*dict_obj).entries_len;

            // Grow entries array if needed. Use real_entries_capacity to
            // correctly handle DefaultDict objects with packed factory_tag.
            if new_idx >= real_entries_capacity(dict_obj) {
                // This shouldn't normally happen since resize handles it,
                // but handle it defensively. Avoid recursion to prevent stack overflow.
                dict_resize(dict_obj);
                // After resize, recompute slot and insert directly (no recursion)
                let (slot2, entry_idx2) = find_insert_slot(dict_obj, key, hash);
                if entry_idx2 >= 0 {
                    let entry = (*dict_obj).entries.add(entry_idx2 as usize);
                    (*entry).value = pyaot_core_defs::Value(value as u64);
                } else {
                    let new_idx2 = (*dict_obj).entries_len;
                    let entry = (*dict_obj).entries.add(new_idx2);
                    (*entry).hash = hash;
                    (*entry).key = pyaot_core_defs::Value(key as u64);
                    (*entry).value = pyaot_core_defs::Value(value as u64);
                    *(*dict_obj).indices.add(slot2) = new_idx2 as i64;
                    (*dict_obj).entries_len += 1;
                    (*dict_obj).len += 1;
                }
                return;
            }

            let entry = (*dict_obj).entries.add(new_idx);
            (*entry).hash = hash;
            (*entry).key = pyaot_core_defs::Value(key as u64);
            (*entry).value = pyaot_core_defs::Value(value as u64);

            // Update indices table
            *(*dict_obj).indices.add(slot) = new_idx as i64;

            (*dict_obj).entries_len += 1;
            (*dict_obj).len += 1;
        }
    }
}
#[export_name = "rt_dict_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_set_abi(dict: Value, key: Value, value: Value) {
    rt_dict_set(dict.unwrap_ptr(), key.unwrap_ptr(), value.unwrap_ptr())
}


/// Get a value from the dictionary by key
/// Returns: pointer to value, or null if key not found
pub fn rt_dict_get(dict: *mut Obj, key: *mut Obj) -> *mut Obj {
    if dict.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_get");
        let dict_obj = dict as *mut DictObj;
        let hash = hash_hashable_obj(key);
        let entry_idx = lookup_entry(dict_obj, key, hash);

        if entry_idx >= 0 {
            let entry = (*dict_obj).entries.add(entry_idx as usize);
            (*entry).value.0 as *mut Obj
        } else {
            std::ptr::null_mut()
        }
    }
}
#[export_name = "rt_dict_get"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_get_abi(dict: Value, key: Value) -> Value {
    Value::from_ptr(rt_dict_get(dict.unwrap_ptr(), key.unwrap_ptr()))
}


/// Check if key exists in dictionary
/// Returns: 1 (true) or 0 (false)
pub fn rt_dict_contains(dict: *mut Obj, key: *mut Obj) -> i8 {
    if dict.is_null() || key.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_contains");
        let dict_obj = dict as *mut DictObj;
        let hash = hash_hashable_obj(key);
        let entry_idx = lookup_entry(dict_obj, key, hash);
        if entry_idx >= 0 {
            1
        } else {
            0
        }
    }
}
#[export_name = "rt_dict_contains"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_contains_abi(dict: Value, key: Value) -> i8 {
    rt_dict_contains(dict.unwrap_ptr(), key.unwrap_ptr())
}


/// Get value with default if key not found
/// Returns: value if found, otherwise default
pub fn rt_dict_get_default(
    dict: *mut Obj,
    key: *mut Obj,
    default: *mut Obj,
) -> *mut Obj {
    let result = rt_dict_get(dict, key);
    if result.is_null() {
        default
    } else {
        result
    }
}
#[export_name = "rt_dict_get_default"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_get_default_abi(
    dict: Value,
    key: Value,
    default: Value,
) -> Value {
    Value::from_ptr(rt_dict_get_default(dict.unwrap_ptr(), key.unwrap_ptr(), default.unwrap_ptr()))
}


/// Pop (remove and return) value for key
/// Returns: value if found and removed, otherwise null
pub fn rt_dict_pop(dict: *mut Obj, key: *mut Obj) -> *mut Obj {
    if dict.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_pop");
        let dict_obj = dict as *mut DictObj;
        let hash = hash_hashable_obj(key);

        // Find the slot in the indices table
        let cap = (*dict_obj).indices_capacity;
        if cap == 0 {
            return std::ptr::null_mut();
        }
        let mask = cap - 1;
        let base = hash as usize;

        for probe in 0..cap {
            let offset = (probe * (probe + 1)) >> 1;
            let slot = (base + offset) & mask;
            let entry_idx = *(*dict_obj).indices.add(slot);

            if entry_idx == EMPTY_INDEX {
                return std::ptr::null_mut();
            }
            if entry_idx == DUMMY_INDEX {
                continue;
            }

            let entry = (*dict_obj).entries.add(entry_idx as usize);
            if (*entry).hash == hash && eq_hashable_obj((*entry).key.0 as *mut Obj, key) {
                let value = (*entry).value.0 as *mut Obj;

                // Mark entry as deleted
                (*entry).key = pyaot_core_defs::Value(0);
                (*entry).value = pyaot_core_defs::Value(0);

                // Mark index slot as dummy (tombstone for probe chain)
                *(*dict_obj).indices.add(slot) = DUMMY_INDEX;

                (*dict_obj).len -= 1;
                return value;
            }
        }

        std::ptr::null_mut()
    }
}
#[export_name = "rt_dict_pop"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_pop_abi(dict: Value, key: Value) -> Value {
    Value::from_ptr(rt_dict_pop(dict.unwrap_ptr(), key.unwrap_ptr()))
}


/// Clear all entries from dictionary
pub fn rt_dict_clear(dict: *mut Obj) {
    if dict.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_clear");
        let dict_obj = dict as *mut DictObj;

        // Reset indices to EMPTY
        for i in 0..(*dict_obj).indices_capacity {
            *(*dict_obj).indices.add(i) = EMPTY_INDEX;
        }

        // Clear entries
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            (*entry).hash = 0;
            (*entry).key = pyaot_core_defs::Value(0);
            (*entry).value = pyaot_core_defs::Value(0);
        }

        (*dict_obj).len = 0;
        (*dict_obj).entries_len = 0;
    }
}
#[export_name = "rt_dict_clear"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_clear_abi(dict: Value) {
    rt_dict_clear(dict.unwrap_ptr())
}


/// Create a shallow copy of dictionary (preserves insertion order)
/// Returns: pointer to new DictObj
pub fn rt_dict_copy(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_dict(8);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_copy");
        let src = dict as *mut DictObj;
        let new_dict = rt_make_dict((*src).len as i64);

        // Root both the source dict and the new dict across all rt_dict_set calls.
        // rt_dict_set internally calls rt_make_str_interned and may trigger GC.
        let mut roots: [*mut Obj; 2] = [dict, new_dict];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate entries in insertion order.
        // Re-read src pointer from roots[0] after each call that may collect.
        let entries_len = (*(roots[0] as *mut DictObj)).entries_len;
        for i in 0..entries_len {
            let src_dict = roots[0] as *mut DictObj;
            let entry = (*src_dict).entries.add(i);
            let key = (*entry).key;
            if key.0 != 0 {
                let value = (*entry).value;
                rt_dict_set(roots[1], key.0 as *mut Obj, value.0 as *mut Obj);
            }
        }

        gc_pop();
        new_dict
    }
}
#[export_name = "rt_dict_copy"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_copy_abi(dict: Value) -> Value {
    Value::from_ptr(rt_dict_copy(dict.unwrap_ptr()))
}


/// Update dictionary with entries from another dictionary (preserves insertion order of other)
pub fn rt_dict_update(dict: *mut Obj, other: *mut Obj) {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_update");
        debug_assert_type_tag!(other, TypeTagKind::Dict, "rt_dict_update");

        // Root both dicts across rt_dict_set calls which may trigger GC.
        let mut roots: [*mut Obj; 2] = [dict, other];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Snapshot the entry count before iteration; re-read other from roots each time.
        let entries_len = (*(roots[1] as *mut DictObj)).entries_len;
        for i in 0..entries_len {
            let other_dict = roots[1] as *mut DictObj;
            let entry = (*other_dict).entries.add(i);
            let key = (*entry).key;
            if key.0 != 0 {
                let value = (*entry).value;
                rt_dict_set(roots[0], key.0 as *mut Obj, value.0 as *mut Obj);
            }
        }

        gc_pop();
    }
}
#[export_name = "rt_dict_update"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_update_abi(dict: Value, other: Value) {
    rt_dict_update(dict.unwrap_ptr(), other.unwrap_ptr())
}


/// dict.setdefault(key, default) - Get value for key, set to default if not present
/// If key exists in dict, returns the existing value.
/// If key not in dict, sets dict[key] = default and returns default.
/// Returns: value for key (existing or newly set)
pub fn rt_dict_setdefault(dict: *mut Obj, key: *mut Obj, default: *mut Obj) -> *mut Obj {
    if dict.is_null() || key.is_null() {
        return default;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_setdefault");

        let existing = rt_dict_get(dict, key);

        if existing.is_null() {
            rt_dict_set(dict, key, default);
            default
        } else {
            existing
        }
    }
}
#[export_name = "rt_dict_setdefault"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_setdefault_abi(dict: Value, key: Value, default: Value) -> Value {
    Value::from_ptr(rt_dict_setdefault(dict.unwrap_ptr(), key.unwrap_ptr(), default.unwrap_ptr()))
}


/// dict.popitem() - Remove and return (key, value) tuple of last inserted item
/// Raises KeyError if dict is empty.
/// Returns: pointer to 2-tuple (key, value)
pub fn rt_dict_popitem(dict: *mut Obj) -> *mut Obj {
    use crate::exceptions::ExceptionType;
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if dict.is_null() {
        unsafe {
            raise_exc!(ExceptionType::KeyError, "popitem(): dictionary is empty");
        }
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_popitem");
        let dict_obj = dict as *mut DictObj;

        if (*dict_obj).len == 0 {
            raise_exc!(ExceptionType::KeyError, "popitem(): dictionary is empty");
        }

        // Scan entries backwards to find the last active entry (insertion order)
        let mut last_idx = (*dict_obj).entries_len;
        while last_idx > 0 {
            last_idx -= 1;
            let entry = (*dict_obj).entries.add(last_idx);
            if (*entry).key.0 != 0 {
                // Save entry data BEFORE allocating tuple (which may trigger GC)
                let key_val = (*entry).key; // Value
                let value_val = (*entry).value; // Value
                let hash = (*entry).hash;

                // Root dict and key/value on shadow stack during tuple allocation
                let mut roots: [*mut Obj; 3] =
                    [dict, key_val.0 as *mut Obj, value_val.0 as *mut Obj];
                let mut frame = crate::gc::ShadowFrame {
                    prev: std::ptr::null_mut(),
                    nroots: 3,
                    roots: roots.as_mut_ptr(),
                };
                crate::gc::gc_push(&mut frame);

                // Create result tuple (may trigger GC)
                let tuple = rt_make_tuple(2);
                rt_tuple_set(tuple, 0, roots[1]); // Use rooted key
                rt_tuple_set(tuple, 1, roots[2]); // Use rooted value

                crate::gc::gc_pop();

                // Delete the entry: null out key/value in entries, mark index as dummy
                let entry = (*dict_obj).entries.add(last_idx);
                (*entry).key = pyaot_core_defs::Value(0);
                (*entry).value = pyaot_core_defs::Value(0);

                // Find and mark the corresponding index slot as DUMMY
                // Must skip DUMMY_INDEX entries (tombstones) to follow the full probe chain
                let cap = (*dict_obj).indices_capacity;
                let mask = cap - 1;
                let base = hash as usize;
                for probe in 0..cap {
                    let offset = (probe * (probe + 1)) >> 1;
                    let slot = (base + offset) & mask;
                    let idx = *(*dict_obj).indices.add(slot);
                    if idx == last_idx as i64 {
                        *(*dict_obj).indices.add(slot) = DUMMY_INDEX;
                        break;
                    }
                    if idx == EMPTY_INDEX {
                        break;
                    }
                    // DUMMY_INDEX: continue probing (tombstone in probe chain)
                }

                (*dict_obj).len -= 1;

                // Shrink entries_len if we removed the last entry
                while (*dict_obj).entries_len > 0 {
                    let e = (*dict_obj).entries.add((*dict_obj).entries_len - 1);
                    if (*e).key.0 == 0 {
                        (*dict_obj).entries_len -= 1;
                    } else {
                        break;
                    }
                }

                return tuple;
            }
        }

        // Should not reach here if len > 0
        raise_exc!(ExceptionType::KeyError, "popitem(): dictionary is empty");
    }
}
#[export_name = "rt_dict_popitem"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_popitem_abi(dict: Value) -> Value {
    Value::from_ptr(rt_dict_popitem(dict.unwrap_ptr()))
}


/// Create dict from keys with optional value
/// keys_list: list of keys
/// value: value for all keys (None if null)
/// Returns: pointer to new DictObj
pub fn rt_dict_fromkeys(keys_list: *mut Obj, value: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::list::rt_list_len;
    use crate::object::ListObj;

    if keys_list.is_null() {
        return rt_make_dict(0);
    }

    unsafe {
        let len = rt_list_len(keys_list);

        let dict = rt_make_dict(len);

        // Root the new dict and the source keys_list across rt_dict_set calls
        // which may trigger GC.
        let mut roots: [*mut Obj; 2] = [dict, keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        for i in 0..len as usize {
            let list_obj = roots[1] as *mut ListObj;
            let key = (*(*list_obj).data.add(i)).0 as *mut crate::object::Obj;
            // When no value is supplied, Python uses None as the default.
            // Storing null would make rt_dict_get indistinguishable from a
            // missing key, so we store the None singleton instead.
            let val = if value.is_null() {
                crate::object::none_obj()
            } else {
                value
            };
            rt_dict_set(roots[0], key, val);
        }

        gc_pop();
        roots[0]
    }
}
#[export_name = "rt_dict_fromkeys"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_fromkeys_abi(keys_list: Value, value: Value) -> Value {
    Value::from_ptr(rt_dict_fromkeys(keys_list.unwrap_ptr(), value.unwrap_ptr()))
}


/// Merge two dicts into a new dict (preserves insertion order)
/// Returns: pointer to new DictObj
pub fn rt_dict_merge(dict1: *mut Obj, dict2: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    let result = rt_make_dict(0);

    // Root result, dict1, and dict2 across all rt_dict_set calls which may trigger GC.
    // Null pointers are safe to include in the roots array — the GC skips them.
    let mut roots: [*mut Obj; 3] = [result, dict1, dict2];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    if !dict1.is_null() {
        unsafe {
            debug_assert_type_tag!(dict1, TypeTagKind::Dict, "rt_dict_merge");
            let entries_len = (*(roots[1] as *mut DictObj)).entries_len;
            for i in 0..entries_len {
                let d1 = roots[1] as *mut DictObj;
                let entry = (*d1).entries.add(i);
                if (*entry).key.0 != 0 {
                    let key = (*entry).key;
                    let value = (*entry).value;
                    rt_dict_set(roots[0], key.0 as *mut Obj, value.0 as *mut Obj);
                }
            }
        }
    }

    if !dict2.is_null() {
        unsafe {
            debug_assert_type_tag!(dict2, TypeTagKind::Dict, "rt_dict_merge");
            let entries_len = (*(roots[2] as *mut DictObj)).entries_len;
            for i in 0..entries_len {
                let d2 = roots[2] as *mut DictObj;
                let entry = (*d2).entries.add(i);
                if (*entry).key.0 != 0 {
                    let key = (*entry).key;
                    let value = (*entry).value;
                    rt_dict_set(roots[0], key.0 as *mut Obj, value.0 as *mut Obj);
                }
            }
        }
    }

    gc_pop();
    roots[0]
}
#[export_name = "rt_dict_merge"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_merge_abi(dict1: Value, dict2: Value) -> Value {
    Value::from_ptr(rt_dict_merge(dict1.unwrap_ptr(), dict2.unwrap_ptr()))
}


/// Create a dict from a list of (key, value) pairs
/// Each element of the list should be a 2-tuple
/// Returns: pointer to new DictObj
pub fn rt_dict_from_pairs(pairs: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::{ListObj, TupleObj};

    let dict = rt_make_dict(8);

    if pairs.is_null() {
        return dict;
    }

    unsafe {
        debug_assert_type_tag!(pairs, TypeTagKind::List, "rt_dict_from_pairs");

        // Root both the new dict and the source pairs list across rt_dict_set calls
        // which may trigger GC.
        let mut roots: [*mut Obj; 2] = [dict, pairs];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let len = (*(roots[1] as *mut ListObj)).len;
        for i in 0..len {
            let list = roots[1] as *mut ListObj;
            let pair = (*(*list).data.add(i)).0 as *mut crate::object::Obj;
            if pair.is_null() {
                continue;
            }

            // Each pair should be a 2-tuple
            let tuple = pair as *mut TupleObj;
            if (*tuple).len >= 2 {
                let key = (*(*tuple).data.as_ptr()).0 as *mut Obj;
                let value = (*(*tuple).data.as_ptr().add(1)).0 as *mut Obj;
                rt_dict_set(roots[0], key, value);
            }
        }

        gc_pop();
    }

    dict
}
#[export_name = "rt_dict_from_pairs"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_from_pairs_abi(pairs: Value) -> Value {
    Value::from_ptr(rt_dict_from_pairs(pairs.unwrap_ptr()))
}

