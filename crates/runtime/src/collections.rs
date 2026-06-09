//! Collections module runtime support
//!
//! Provides runtime functions for OrderedDict.

use crate::dict::{real_entries_capacity, rt_dict_set, set_real_entries_capacity};
use crate::exceptions::ExceptionType;
use crate::hash_table_utils::{eq_hashable_obj, hash_hashable_obj};
use crate::object::{DictEntry, DictObj, Obj};
use crate::tuple::{rt_make_tuple, rt_tuple_set};
use pyaot_core_defs::Value;

// =============================================================================
// OrderedDict support
// =============================================================================

/// Sentinel value for empty slot in indices table
const EMPTY_INDEX: i64 = -1;
/// Sentinel value for deleted slot in indices table
const DUMMY_INDEX: i64 = -2;

/// OrderedDict.move_to_end(key, last=True)
/// Moves an existing key to either end of the ordered dict.
/// last=1 (default): move to end; last=0: move to beginning.
/// Raises KeyError if the key does not exist.
pub fn rt_dict_move_to_end(dict: *mut Obj, key: *mut Obj, last: i64) {
    if dict.is_null() || key.is_null() {
        return;
    }

    unsafe {
        let dict_obj = dict as *mut DictObj;
        let hash = hash_hashable_obj(key);

        // Find the entry
        let entry_idx = lookup_entry_index(dict_obj, key, hash);
        if entry_idx < 0 {
            raise_exc!(ExceptionType::KeyError, "key not found in OrderedDict");
        }

        let entry_idx = entry_idx as usize;
        let entry = (*dict_obj).entries.add(entry_idx);
        let saved_key = (*entry).key;
        let saved_value = (*entry).value;

        // Remove from current position
        delete_entry(dict_obj, entry_idx, hash);

        if last != 0 {
            // Move to end: shrink trailing nulls, then use rt_dict_set
            shrink_trailing_nulls(dict_obj);
            rt_dict_set(dict, saved_key.0 as *mut Obj, saved_value.0 as *mut Obj);
        } else {
            // Move to beginning: rebuild entries with this entry first
            rebuild_with_entry_first(
                dict_obj,
                saved_key.0 as *mut Obj,
                saved_value.0 as *mut Obj,
                hash,
            );
        }
    }
}
#[export_name = "rt_dict_move_to_end"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_move_to_end_abi(dict: Value, key: Value, last: i64) {
    rt_dict_move_to_end(dict.unwrap_ptr(), key.unwrap_ptr(), last)
}

/// OrderedDict.popitem(last=True)
/// Remove and return (key, value) pair.
/// last=1 (default): LIFO (from end); last=0: FIFO (from beginning).
/// Raises KeyError if empty.
pub fn rt_dict_popitem_ordered(dict: *mut Obj, last: i64) -> *mut Obj {
    if dict.is_null() {
        unsafe {
            raise_exc!(ExceptionType::KeyError, "popitem(): dictionary is empty");
        }
    }

    unsafe {
        let dict_obj = dict as *mut DictObj;

        if (*dict_obj).len == 0 {
            raise_exc!(ExceptionType::KeyError, "popitem(): dictionary is empty");
        }

        if last != 0 {
            pop_last_entry(dict_obj)
        } else {
            pop_first_entry(dict_obj)
        }
    }
}
#[export_name = "rt_dict_popitem_ordered"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_popitem_ordered_abi(dict: Value, last: i64) -> Value {
    Value::from_ptr(rt_dict_popitem_ordered(dict.unwrap_ptr(), last))
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Look up entry index by key. Returns >= 0 if found, -1 if not found.
unsafe fn lookup_entry_index(dict: *mut DictObj, key: *mut Obj, hash: u64) -> i64 {
    let cap = (*dict).indices_capacity;
    if cap == 0 {
        return -1;
    }
    let mask = cap - 1;
    let base = hash as usize;

    for probe in 0..cap {
        let offset = (probe * (probe + 1)) >> 1;
        let slot = (base + offset) & mask;
        let entry_idx = *(*dict).indices.add(slot);

        if entry_idx == EMPTY_INDEX {
            return -1;
        }
        if entry_idx == DUMMY_INDEX {
            continue;
        }
        let entry = (*dict).entries.add(entry_idx as usize);
        if (*entry).hash == hash && eq_hashable_obj((*entry).key.0 as *mut Obj, key) {
            return entry_idx;
        }
    }
    -1
}

/// Mark the index slot for a given entry_idx as DUMMY
unsafe fn mark_index_as_dummy(dict: *mut DictObj, hash: u64, entry_idx: usize) {
    let cap = (*dict).indices_capacity;
    let mask = cap - 1;
    let base = hash as usize;

    for probe in 0..cap {
        let offset = (probe * (probe + 1)) >> 1;
        let slot = (base + offset) & mask;
        let idx = *(*dict).indices.add(slot);
        if idx == entry_idx as i64 {
            *(*dict).indices.add(slot) = DUMMY_INDEX;
            return;
        }
        if idx == EMPTY_INDEX {
            return;
        }
    }
}

/// Delete an entry: null key/value, mark index as DUMMY, decrement len
unsafe fn delete_entry(dict: *mut DictObj, entry_idx: usize, hash: u64) {
    let entry = (*dict).entries.add(entry_idx);
    (*entry).key = pyaot_core_defs::Value(0);
    (*entry).value = pyaot_core_defs::Value(0);
    mark_index_as_dummy(dict, hash, entry_idx);
    (*dict).len -= 1;
}

/// Shrink entries_len by removing trailing null entries
unsafe fn shrink_trailing_nulls(dict: *mut DictObj) {
    while (*dict).entries_len > 0 {
        let e = (*dict).entries.add((*dict).entries_len - 1);
        if (*e).key.0 == 0 {
            (*dict).entries_len -= 1;
        } else {
            break;
        }
    }
}

/// Rebuild entries array with a specific entry placed first, preserving order of others.
unsafe fn rebuild_with_entry_first(dict: *mut DictObj, key: *mut Obj, value: *mut Obj, hash: u64) {
    use std::alloc::{alloc_zeroed, dealloc, Layout};

    let old_entries = (*dict).entries;
    let old_entries_len = (*dict).entries_len;
    // Use real_entries_capacity to strip any packed factory_tag (DefaultDict).
    let old_entries_capacity = real_entries_capacity(dict);
    let active_count = (*dict).len; // Already decremented

    let new_capacity = old_entries_capacity;
    let entries_layout =
        Layout::array::<DictEntry>(new_capacity).expect("Allocation size overflow");
    let new_entries = alloc_zeroed(entries_layout) as *mut DictEntry;

    // Place the target entry first
    let first = new_entries;
    (*first).hash = hash;
    (*first).key = pyaot_core_defs::Value(key as u64);
    (*first).value = pyaot_core_defs::Value(value as u64);
    let mut new_len: usize = 1;

    // Copy remaining active entries in order
    for i in 0..old_entries_len {
        let old_entry = old_entries.add(i);
        if (*old_entry).key.0 != 0 {
            let dst = new_entries.add(new_len);
            (*dst).hash = (*old_entry).hash;
            (*dst).key = (*old_entry).key;
            (*dst).value = (*old_entry).value;
            new_len += 1;
        }
    }

    // Rebuild indices table from scratch
    let cap = (*dict).indices_capacity;
    for i in 0..cap {
        *(*dict).indices.add(i) = EMPTY_INDEX;
    }
    let mask = cap - 1;
    for i in 0..new_len {
        let entry = new_entries.add(i);
        let h = (*entry).hash;
        let base = h as usize;
        for probe in 0..cap {
            let offset = (probe * (probe + 1)) >> 1;
            let slot = (base + offset) & mask;
            if *(*dict).indices.add(slot) == EMPTY_INDEX {
                *(*dict).indices.add(slot) = i as i64;
                break;
            }
        }
    }

    // Free old entries
    if !old_entries.is_null() && old_entries_capacity > 0 {
        let layout =
            Layout::array::<DictEntry>(old_entries_capacity).expect("Allocation size overflow");
        dealloc(old_entries as *mut u8, layout);
    }

    (*dict).entries = new_entries;
    (*dict).entries_len = new_len;
    // Use set_real_entries_capacity to preserve any packed factory_tag.
    set_real_entries_capacity(dict, new_capacity);
    (*dict).len = active_count + 1; // Re-add the moved entry
}

/// Pop the last (most recently inserted) entry
unsafe fn pop_last_entry(dict: *mut DictObj) -> *mut Obj {
    let mut last_idx = (*dict).entries_len;
    while last_idx > 0 {
        last_idx -= 1;
        let entry = (*dict).entries.add(last_idx);
        if (*entry).key.0 != 0 {
            return pop_entry_at(dict, last_idx);
        }
    }

    raise_exc!(ExceptionType::KeyError, "popitem(): dictionary is empty");
}

/// Pop the first (oldest) entry
unsafe fn pop_first_entry(dict: *mut DictObj) -> *mut Obj {
    let entries_len = (*dict).entries_len;
    for i in 0..entries_len {
        let entry = (*dict).entries.add(i);
        if (*entry).key.0 != 0 {
            return pop_entry_at(dict, i);
        }
    }

    raise_exc!(ExceptionType::KeyError, "popitem(): dictionary is empty");
}

/// Remove entry at given index and return (key, value) tuple
unsafe fn pop_entry_at(dict: *mut DictObj, idx: usize) -> *mut Obj {
    let entry = (*dict).entries.add(idx);
    let key = (*entry).key;
    let value = (*entry).value;
    let hash = (*entry).hash;

    let tuple = rt_make_tuple(2);
    rt_tuple_set(tuple, 0, key.0 as *mut Obj);
    rt_tuple_set(tuple, 1, value.0 as *mut Obj);

    delete_entry(dict, idx, hash);
    shrink_trailing_nulls(dict);

    tuple
}
