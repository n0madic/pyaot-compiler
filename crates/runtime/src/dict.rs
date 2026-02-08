//! Dictionary operations for Python runtime

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::hash_table_utils::hash_hashable_obj;
use crate::list::{rt_list_push, rt_make_list};
use crate::object::{DictObj, Obj, StrObj, TypeTagKind, ELEM_HEAP_OBJ, TOMBSTONE};
use crate::string::rt_make_str_interned;
use crate::tuple::{rt_make_tuple, rt_tuple_set};

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

fn find_slot(dict: *mut crate::object::DictObj, key: *mut Obj, hash: u64, for_insert: bool) -> i64 {
    unsafe {
        find_slot_generic(
            (*dict).capacity,
            hash,
            for_insert,
            |i| (*(*dict).entries.add(i)).key,
            |i| (*(*dict).entries.add(i)).hash,
            TOMBSTONE,
            key,
        )
    }
}

fn dict_resize(dict: *mut crate::object::DictObj, new_capacity: usize) {
    use crate::object::DictEntry;
    use std::alloc::{alloc_zeroed, dealloc, Layout};

    unsafe {
        let old_entries = (*dict).entries;
        let old_capacity = (*dict).capacity;

        // Allocate new entries array
        let new_layout = Layout::array::<DictEntry>(new_capacity)
            .expect("Allocation size overflow - capacity too large");
        let new_entries = alloc_zeroed(new_layout) as *mut DictEntry;

        // Initialize new entries
        for i in 0..new_capacity {
            let entry = new_entries.add(i);
            (*entry).hash = 0;
            (*entry).key = std::ptr::null_mut();
            (*entry).value = std::ptr::null_mut();
        }

        // Update dict with new entries
        (*dict).entries = new_entries;
        (*dict).capacity = new_capacity;
        (*dict).len = 0;

        // Rehash existing entries using triangular probing (power-of-2 capacity)
        let mask = new_capacity - 1;
        for i in 0..old_capacity {
            let old_entry = old_entries.add(i);
            let key = (*old_entry).key;
            if !key.is_null() && key != TOMBSTONE {
                let hash = (*old_entry).hash;
                let base = hash as usize;

                // Find empty slot in new table using triangular probing
                // offset = i*(i+1)/2: 0, 1, 3, 6, 10, 15, ...
                let mut probe_i = 0usize;
                loop {
                    let offset = (probe_i * (probe_i + 1)) >> 1;
                    let index = (base + offset) & mask;
                    let entry = new_entries.add(index);
                    if (*entry).key.is_null() {
                        (*entry).hash = hash;
                        (*entry).key = key;
                        (*entry).value = (*old_entry).value;
                        (*dict).len += 1;
                        break;
                    }
                    probe_i += 1;
                }
            }
        }

        // Free old entries array
        if old_capacity > 0 && !old_entries.is_null() {
            let old_layout = Layout::array::<DictEntry>(old_capacity)
                .expect("Allocation size overflow - capacity too large");
            dealloc(old_entries as *mut u8, old_layout);
        }
    }
}

/// Create a new dictionary with given initial capacity
/// Returns: pointer to allocated DictObj
#[no_mangle]
pub extern "C" fn rt_make_dict(capacity: i64) -> *mut Obj {
    use crate::object::{DictEntry, DictObj, TypeTagKind};
    use std::alloc::{alloc_zeroed, Layout};

    // Ensure capacity is power of 2 for efficient mask-based probing
    let requested = if capacity <= 0 {
        8
    } else {
        capacity.max(8) as usize
    };
    let capacity = next_power_of_2(requested);

    // Allocate DictObj using GC
    let dict_size = std::mem::size_of::<DictObj>();
    let obj = gc::gc_alloc(dict_size, TypeTagKind::Dict as u8);

    unsafe {
        let dict = obj as *mut DictObj;
        (*dict).len = 0;
        (*dict).capacity = capacity;

        // Allocate entries array separately
        let entries_layout = Layout::array::<DictEntry>(capacity)
            .expect("Allocation size overflow - capacity too large");
        let entries_ptr = alloc_zeroed(entries_layout) as *mut DictEntry;
        (*dict).entries = entries_ptr;

        // Initialize all entries to empty (null keys)
        for i in 0..capacity {
            let entry = entries_ptr.add(i);
            (*entry).hash = 0;
            (*entry).key = std::ptr::null_mut();
            (*entry).value = std::ptr::null_mut();
        }
    }

    obj
}

/// Maximum string length to intern for dict keys
const MAX_DICT_KEY_INTERN_LENGTH: usize = 256;

/// Set a key-value pair in the dictionary
/// If key exists, updates value. If not, inserts new entry.
/// String keys under 256 bytes are interned for memory efficiency.
#[no_mangle]
pub extern "C" fn rt_dict_set(dict: *mut Obj, mut key: *mut Obj, value: *mut Obj) {
    if dict.is_null() || key.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_set");

        // Intern string keys under the size threshold
        if (*key).header.type_tag == TypeTagKind::Str {
            let str_obj = key as *mut StrObj;
            let len = (*str_obj).len;

            if len < MAX_DICT_KEY_INTERN_LENGTH {
                let data = (*str_obj).data.as_ptr();
                key = rt_make_str_interned(data, len);
            }
        }

        let dict_obj = dict as *mut DictObj;

        // Check load factor and resize if needed (> 0.75)
        if (*dict_obj).len * 4 >= (*dict_obj).capacity * 3 {
            let new_capacity = (*dict_obj).capacity * 2;
            dict_resize(dict_obj, new_capacity);
        }

        let hash = hash_hashable_obj(key);
        let slot = find_slot(dict_obj, key, hash, true);

        if slot >= 0 {
            let entry = (*dict_obj).entries.add(slot as usize);
            let is_new = (*entry).key.is_null() || (*entry).key == TOMBSTONE;
            (*entry).hash = hash;
            (*entry).key = key;
            (*entry).value = value;
            if is_new {
                (*dict_obj).len += 1;
            }
        }
    }
}

/// Get a value from the dictionary by key
/// Returns: pointer to value, or null if key not found
#[no_mangle]
pub extern "C" fn rt_dict_get(dict: *mut Obj, key: *mut Obj) -> *mut Obj {
    if dict.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_get");
        let dict_obj = dict as *mut DictObj;
        let hash = hash_hashable_obj(key);
        let slot = find_slot(dict_obj, key, hash, false);

        if slot >= 0 {
            let entry = (*dict_obj).entries.add(slot as usize);
            (*entry).value
        } else {
            std::ptr::null_mut()
        }
    }
}

/// Get length of dictionary (number of entries)
#[no_mangle]
pub extern "C" fn rt_dict_len(dict: *mut Obj) -> i64 {
    if dict.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_len");
        let dict_obj = dict as *mut DictObj;
        (*dict_obj).len as i64
    }
}

/// Check if key exists in dictionary
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_dict_contains(dict: *mut Obj, key: *mut Obj) -> i8 {
    if dict.is_null() || key.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_contains");
        let dict_obj = dict as *mut DictObj;
        let hash = hash_hashable_obj(key);
        let slot = find_slot(dict_obj, key, hash, false);
        if slot >= 0 {
            1
        } else {
            0
        }
    }
}

/// Get value with default if key not found
/// Returns: value if found, otherwise default
#[no_mangle]
pub extern "C" fn rt_dict_get_default(
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

/// Pop (remove and return) value for key
/// Returns: value if found and removed, otherwise null
#[no_mangle]
pub extern "C" fn rt_dict_pop(dict: *mut Obj, key: *mut Obj) -> *mut Obj {
    if dict.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_pop");
        let dict_obj = dict as *mut DictObj;
        let hash = hash_hashable_obj(key);
        let slot = find_slot(dict_obj, key, hash, false);

        if slot >= 0 {
            let entry = (*dict_obj).entries.add(slot as usize);
            let value = (*entry).value;
            // Mark as tombstone
            (*entry).key = TOMBSTONE;
            (*entry).value = std::ptr::null_mut();
            (*dict_obj).len -= 1;
            value
        } else {
            std::ptr::null_mut()
        }
    }
}

/// Clear all entries from dictionary
#[no_mangle]
pub extern "C" fn rt_dict_clear(dict: *mut Obj) {
    if dict.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_clear");
        let dict_obj = dict as *mut DictObj;
        let capacity = (*dict_obj).capacity;
        let entries = (*dict_obj).entries;

        for i in 0..capacity {
            let entry = entries.add(i);
            (*entry).hash = 0;
            (*entry).key = std::ptr::null_mut();
            (*entry).value = std::ptr::null_mut();
        }
        (*dict_obj).len = 0;
    }
}

/// Create a shallow copy of dictionary
/// Returns: pointer to new DictObj
#[no_mangle]
pub extern "C" fn rt_dict_copy(dict: *mut Obj) -> *mut Obj {
    if dict.is_null() {
        return rt_make_dict(8);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_copy");
        let src = dict as *mut DictObj;
        let capacity = (*src).capacity;
        let new_dict = rt_make_dict(capacity as i64);

        // Copy all non-empty, non-tombstone entries
        for i in 0..capacity {
            let src_entry = (*src).entries.add(i);
            let key = (*src_entry).key;
            if !key.is_null() && key != TOMBSTONE {
                rt_dict_set(new_dict, key, (*src_entry).value);
            }
        }

        new_dict
    }
}

/// Get list of all keys in dictionary
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_dict_keys(dict: *mut Obj) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_keys");
        let dict_obj = dict as *mut DictObj;
        let capacity = (*dict_obj).capacity;
        let len = (*dict_obj).len;

        let keys_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

        for i in 0..capacity {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() && key != TOMBSTONE {
                rt_list_push(keys_list, key);
            }
        }

        keys_list
    }
}

/// Get list of all values in dictionary
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_dict_values(dict: *mut Obj) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_values");
        let dict_obj = dict as *mut DictObj;
        let capacity = (*dict_obj).capacity;
        let len = (*dict_obj).len;

        let values_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

        for i in 0..capacity {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() && key != TOMBSTONE {
                rt_list_push(values_list, (*entry).value);
            }
        }

        values_list
    }
}

/// Get list of (key, value) tuples for all entries
/// Returns: pointer to new ListObj containing TupleObj elements
#[no_mangle]
pub extern "C" fn rt_dict_items(dict: *mut Obj) -> *mut Obj {
    if dict.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_items");
        let dict_obj = dict as *mut DictObj;
        let capacity = (*dict_obj).capacity;
        let len = (*dict_obj).len;

        let items_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

        for i in 0..capacity {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() && key != TOMBSTONE {
                // Create a 2-tuple (key, value)
                let tuple = rt_make_tuple(2, ELEM_HEAP_OBJ);
                rt_tuple_set(tuple, 0, key);
                rt_tuple_set(tuple, 1, (*entry).value);
                rt_list_push(items_list, tuple);
            }
        }

        items_list
    }
}

/// Finalize a dictionary by freeing its entries array
/// Called by GC during sweep phase before freeing the DictObj itself
///
/// # Safety
/// The caller must ensure that `dict` is a valid pointer to a DictObj
/// that is about to be deallocated.
pub unsafe fn dict_finalize(dict: *mut Obj) {
    use crate::object::DictEntry;
    use std::alloc::{dealloc, Layout};

    if dict.is_null() {
        return;
    }

    let dict_obj = dict as *mut DictObj;
    let entries = (*dict_obj).entries;
    let capacity = (*dict_obj).capacity;

    // Free the entries array if allocated
    if !entries.is_null() && capacity > 0 {
        let entries_layout = Layout::array::<DictEntry>(capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(entries as *mut u8, entries_layout);
    }
}

/// Update dictionary with entries from another dictionary
#[no_mangle]
pub extern "C" fn rt_dict_update(dict: *mut Obj, other: *mut Obj) {
    if dict.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_update");
        debug_assert_type_tag!(other, TypeTagKind::Dict, "rt_dict_update");
        let other_dict = other as *mut DictObj;
        let capacity = (*other_dict).capacity;

        for i in 0..capacity {
            let entry = (*other_dict).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() && key != TOMBSTONE {
                rt_dict_set(dict, key, (*entry).value);
            }
        }
    }
}

/// Create a dict from a list of (key, value) pairs
/// Each element of the list should be a 2-tuple
/// Returns: pointer to new DictObj
#[no_mangle]
pub extern "C" fn rt_dict_from_pairs(pairs: *mut Obj) -> *mut Obj {
    use crate::object::{ListObj, TupleObj};

    let dict = rt_make_dict(8);

    if pairs.is_null() {
        return dict;
    }

    unsafe {
        debug_assert_type_tag!(pairs, TypeTagKind::List, "rt_dict_from_pairs");
        let list = pairs as *mut ListObj;
        let len = (*list).len;
        let data = (*list).data;

        for i in 0..len {
            let pair = *data.add(i);
            if pair.is_null() {
                continue;
            }

            // Each pair should be a 2-tuple
            let tuple = pair as *mut TupleObj;
            if (*tuple).len >= 2 {
                let key = *(*tuple).data.as_ptr();
                let value = *(*tuple).data.as_ptr().add(1);

                // Dict values are stored as boxed pointers for GC tracking.
                // Tuple elements are already properly boxed, so pass them directly.
                rt_dict_set(dict, key, value);
            }
        }
    }

    dict
}

/// dict.setdefault(key, default) - Get value for key, set to default if not present
/// If key exists in dict, returns the existing value.
/// If key not in dict, sets dict[key] = default and returns default.
/// Returns: value for key (existing or newly set)
#[no_mangle]
pub extern "C" fn rt_dict_setdefault(dict: *mut Obj, key: *mut Obj, default: *mut Obj) -> *mut Obj {
    if dict.is_null() || key.is_null() {
        return default;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_setdefault");

        // Try to get existing value
        let existing = rt_dict_get(dict, key);

        if existing.is_null() {
            // Key not found, set it to default
            rt_dict_set(dict, key, default);
            default
        } else {
            // Key exists, return existing value
            existing
        }
    }
}

/// dict.popitem() - Remove and return (key, value) tuple of last inserted item
/// Raises KeyError if dict is empty.
/// Returns: pointer to 2-tuple (key, value)
#[no_mangle]
pub extern "C" fn rt_dict_popitem(dict: *mut Obj) -> *mut Obj {
    use crate::exceptions::{rt_exc_raise, ExceptionType};

    if dict.is_null() {
        let msg = b"KeyError: 'popitem(): dictionary is empty'";
        unsafe {
            rt_exc_raise(ExceptionType::KeyError as u8, msg.as_ptr(), msg.len());
        }
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_popitem");
        let dict_obj = dict as *mut DictObj;

        if (*dict_obj).len == 0 {
            let msg = b"KeyError: 'popitem(): dictionary is empty'";
            rt_exc_raise(ExceptionType::KeyError as u8, msg.as_ptr(), msg.len());
        }

        let capacity = (*dict_obj).capacity;
        let entries = (*dict_obj).entries;

        // Find the last non-null, non-tombstone entry
        // We scan backwards to get the "last inserted" item (insertion order)
        let mut last_key: *mut Obj = std::ptr::null_mut();
        let mut last_value: *mut Obj = std::ptr::null_mut();
        let mut last_index: usize = 0;

        for i in (0..capacity).rev() {
            let entry = entries.add(i);
            let key = (*entry).key;
            if !key.is_null() && key != TOMBSTONE {
                last_key = key;
                last_value = (*entry).value;
                last_index = i;
                break;
            }
        }

        // Create tuple with (key, value)
        let tuple = rt_make_tuple(2, ELEM_HEAP_OBJ);
        rt_tuple_set(tuple, 0, last_key);
        rt_tuple_set(tuple, 1, last_value);

        // Remove the entry by marking as tombstone
        let entry = entries.add(last_index);
        (*entry).key = TOMBSTONE;
        (*entry).value = std::ptr::null_mut();
        (*dict_obj).len -= 1;

        tuple
    }
}

/// Create dict from keys with optional value
/// keys_list: list of keys
/// value: value for all keys (None if null)
/// Returns: pointer to new DictObj
#[no_mangle]
pub extern "C" fn rt_dict_fromkeys(keys_list: *mut Obj, value: *mut Obj) -> *mut Obj {
    use crate::list::rt_list_len;
    use crate::object::ListObj;

    if keys_list.is_null() {
        return rt_make_dict(0);
    }

    unsafe {
        let list_obj = keys_list as *mut ListObj;
        let len = rt_list_len(keys_list);

        let dict = rt_make_dict(len);

        for i in 0..len as usize {
            let key = *(*list_obj).data.add(i);
            // Use provided value or None
            let val = if value.is_null() {
                std::ptr::null_mut() // Will be interpreted as None
            } else {
                value
            };
            rt_dict_set(dict, key, val);
        }

        dict
    }
}

/// Merge two dicts into a new dict
/// Returns: pointer to new DictObj
#[no_mangle]
pub extern "C" fn rt_dict_merge(dict1: *mut Obj, dict2: *mut Obj) -> *mut Obj {
    let result = rt_make_dict(0);

    if !dict1.is_null() {
        unsafe {
            debug_assert_type_tag!(dict1, TypeTagKind::Dict, "rt_dict_merge");
            let dict1_obj = dict1 as *mut crate::object::DictObj;
            let capacity = (*dict1_obj).capacity;

            // Copy all entries from dict1
            for i in 0..capacity {
                let entry = (*dict1_obj).entries.add(i);
                let key = (*entry).key;
                if !key.is_null() && key != TOMBSTONE {
                    let value = (*entry).value;
                    rt_dict_set(result, key, value);
                }
            }
        }
    }

    if !dict2.is_null() {
        unsafe {
            debug_assert_type_tag!(dict2, TypeTagKind::Dict, "rt_dict_merge");
            let dict2_obj = dict2 as *mut crate::object::DictObj;
            let capacity = (*dict2_obj).capacity;

            // Copy all entries from dict2 (overwrites dict1 entries with same key)
            for i in 0..capacity {
                let entry = (*dict2_obj).entries.add(i);
                let key = (*entry).key;
                if !key.is_null() && key != TOMBSTONE {
                    let value = (*entry).value;
                    rt_dict_set(result, key, value);
                }
            }
        }
    }

    result
}
