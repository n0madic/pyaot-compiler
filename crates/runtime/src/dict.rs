//! Dictionary operations for Python runtime
//!
//! Uses CPython 3.6+ compact dict design to preserve insertion order:
//! - `indices`: hash index table mapping hash slots to entry indices
//! - `entries`: dense array of DictEntry stored in insertion order

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::hash_table_utils::{eq_hashable_obj, hash_hashable_obj};
use crate::list::{rt_list_push, rt_make_list};
use crate::object::{
    DictEntry, DictObj, ListObj, Obj, StrObj, TypeTagKind, ELEM_HEAP_OBJ, ELEM_RAW_INT,
};
use crate::string::rt_make_str_interned;
use crate::tuple::{rt_make_tuple, rt_tuple_set};

/// Sentinel value for empty slot in indices table
const EMPTY_INDEX: i64 = -1;
/// Sentinel value for deleted slot in indices table (tombstone for probe chain)
const DUMMY_INDEX: i64 = -2;

/// Maximum string length to intern for dict keys
const MAX_DICT_KEY_INTERN_LENGTH: usize = 256;

/// Round up to the next power of 2 (required for mask-based probing).
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

/// Look up a key in the dict's index table.
/// Returns the entry index (>= 0) if found, or -1 if not found.
#[inline]
unsafe fn lookup_entry(dict: *mut DictObj, key: *mut Obj, hash: u64) -> i64 {
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
        // Valid entry — check if key matches
        let entry = (*dict).entries.add(entry_idx as usize);
        if (*entry).hash == hash && eq_hashable_obj((*entry).key, key) {
            return entry_idx;
        }
    }
    -1
}

/// Find a slot in the indices table for insertion.
/// Returns (best_slot_for_insert, entry_index_if_found).
/// If found: entry_index >= 0 (existing entry to update)
/// If not found: entry_index == -1, slot is the best position for a new index entry
#[inline]
unsafe fn find_insert_slot(dict: *mut DictObj, key: *mut Obj, hash: u64) -> (usize, i64) {
    let cap = (*dict).indices_capacity;
    let mask = cap - 1;
    let base = hash as usize;
    let mut first_available: i64 = -1;

    for probe in 0..cap {
        let offset = (probe * (probe + 1)) >> 1;
        let slot = (base + offset) & mask;
        let entry_idx = *(*dict).indices.add(slot);

        if entry_idx == EMPTY_INDEX {
            let insert_slot = if first_available >= 0 {
                first_available as usize
            } else {
                slot
            };
            return (insert_slot, -1);
        }
        if entry_idx == DUMMY_INDEX {
            if first_available < 0 {
                first_available = slot as i64;
            }
            continue;
        }
        // Valid entry — check if key matches
        let entry = (*dict).entries.add(entry_idx as usize);
        if (*entry).hash == hash && eq_hashable_obj((*entry).key, key) {
            return (slot, entry_idx);
        }
    }

    // Table full (shouldn't happen with proper load factor)
    (first_available.max(0) as usize, -1)
}

/// Rebuild indices table and compact entries array.
/// Called when load factor is too high.
unsafe fn dict_resize(dict: *mut DictObj) {
    use std::alloc::{alloc_zeroed, dealloc, Layout};

    let old_entries = (*dict).entries;
    let old_entries_len = (*dict).entries_len;
    let old_entries_capacity = (*dict).entries_capacity;
    let old_indices = (*dict).indices;
    let old_indices_capacity = (*dict).indices_capacity;
    let active_count = (*dict).len;

    // Calculate new indices capacity: at least 2x active entries, power of 2, min 8
    let min_indices = if active_count == 0 {
        8
    } else {
        active_count * 3 // ~33% load factor after resize
    };
    let new_indices_capacity = next_power_of_2(min_indices.max(8));

    // New entries capacity matches indices capacity
    let new_entries_capacity = new_indices_capacity;

    // Allocate new indices table
    let indices_layout = Layout::array::<i64>(new_indices_capacity)
        .expect("Allocation size overflow - capacity too large");
    let new_indices = alloc_zeroed(indices_layout) as *mut i64;
    // Initialize to EMPTY_INDEX (-1)
    // Note: alloc_zeroed gives us 0, but we need -1
    for i in 0..new_indices_capacity {
        *new_indices.add(i) = EMPTY_INDEX;
    }

    // Allocate new entries array
    let entries_layout = Layout::array::<DictEntry>(new_entries_capacity)
        .expect("Allocation size overflow - capacity too large");
    let new_entries = alloc_zeroed(entries_layout) as *mut DictEntry;

    // Compact: copy only active entries (skip deleted), rebuild indices
    let mask = new_indices_capacity - 1;
    let mut new_len: usize = 0;

    for i in 0..old_entries_len {
        let old_entry = old_entries.add(i);
        let key = (*old_entry).key;
        if key.is_null() {
            continue; // Skip deleted entries
        }

        // Copy entry to new position
        let new_entry = new_entries.add(new_len);
        (*new_entry).hash = (*old_entry).hash;
        (*new_entry).key = key;
        (*new_entry).value = (*old_entry).value;

        // Insert into new indices table
        let hash = (*old_entry).hash;
        let base = hash as usize;
        for probe in 0..new_indices_capacity {
            let offset = (probe * (probe + 1)) >> 1;
            let slot = (base + offset) & mask;
            if *new_indices.add(slot) == EMPTY_INDEX {
                *new_indices.add(slot) = new_len as i64;
                break;
            }
        }

        new_len += 1;
    }

    // Update dict
    (*dict).indices = new_indices;
    (*dict).indices_capacity = new_indices_capacity;
    (*dict).entries = new_entries;
    (*dict).entries_len = new_len;
    (*dict).entries_capacity = new_entries_capacity;
    // len stays the same (active_count)

    // Free old arrays
    if !old_indices.is_null() && old_indices_capacity > 0 {
        let layout = Layout::array::<i64>(old_indices_capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(old_indices as *mut u8, layout);
    }
    if !old_entries.is_null() && old_entries_capacity > 0 {
        let layout = Layout::array::<DictEntry>(old_entries_capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(old_entries as *mut u8, layout);
    }
}

/// Create a new dictionary with given initial capacity
/// Returns: pointer to allocated DictObj
#[no_mangle]
pub extern "C" fn rt_make_dict(capacity: i64) -> *mut Obj {
    use std::alloc::{alloc_zeroed, Layout};

    // Ensure capacity is power of 2 for efficient mask-based probing
    let requested = if capacity <= 0 {
        8
    } else {
        capacity.max(8) as usize
    };
    let indices_capacity = next_power_of_2(requested);
    let entries_capacity = indices_capacity;

    // Allocate DictObj using GC
    let dict_size = std::mem::size_of::<DictObj>();
    let obj = gc::gc_alloc(dict_size, TypeTagKind::Dict as u8);

    unsafe {
        let dict = obj as *mut DictObj;
        (*dict).len = 0;
        (*dict).entries_len = 0;

        // Allocate indices table
        let indices_layout = Layout::array::<i64>(indices_capacity)
            .expect("Allocation size overflow - capacity too large");
        let indices_ptr = alloc_zeroed(indices_layout) as *mut i64;
        // Initialize to EMPTY_INDEX (-1)
        for i in 0..indices_capacity {
            *indices_ptr.add(i) = EMPTY_INDEX;
        }
        (*dict).indices = indices_ptr;
        (*dict).indices_capacity = indices_capacity;

        // Allocate entries array
        let entries_layout = Layout::array::<DictEntry>(entries_capacity)
            .expect("Allocation size overflow - capacity too large");
        let entries_ptr = alloc_zeroed(entries_layout) as *mut DictEntry;
        (*dict).entries = entries_ptr;
        (*dict).entries_capacity = entries_capacity;
    }

    obj
}

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

        // Check if we need to resize (entries_len * 3 >= indices_capacity * 2 → >66% full)
        if (*dict_obj).entries_len * 3 >= (*dict_obj).indices_capacity * 2 {
            dict_resize(dict_obj);
        }

        let hash = hash_hashable_obj(key);
        let (slot, entry_idx) = find_insert_slot(dict_obj, key, hash);

        if entry_idx >= 0 {
            // Key exists — update value in place
            let entry = (*dict_obj).entries.add(entry_idx as usize);
            (*entry).value = value;
        } else {
            // New key — append to entries array
            let new_idx = (*dict_obj).entries_len;

            // Grow entries array if needed
            if new_idx >= (*dict_obj).entries_capacity {
                // This shouldn't normally happen since resize handles it,
                // but handle it defensively
                dict_resize(dict_obj);
                // After resize, insert again (indices changed)
                rt_dict_set(dict, key, value);
                return;
            }

            let entry = (*dict_obj).entries.add(new_idx);
            (*entry).hash = hash;
            (*entry).key = key;
            (*entry).value = value;

            // Update indices table
            *(*dict_obj).indices.add(slot) = new_idx as i64;

            (*dict_obj).entries_len += 1;
            (*dict_obj).len += 1;
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
        let entry_idx = lookup_entry(dict_obj, key, hash);

        if entry_idx >= 0 {
            let entry = (*dict_obj).entries.add(entry_idx as usize);
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
        let entry_idx = lookup_entry(dict_obj, key, hash);
        if entry_idx >= 0 {
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
            if (*entry).hash == hash && eq_hashable_obj((*entry).key, key) {
                let value = (*entry).value;

                // Mark entry as deleted
                (*entry).key = std::ptr::null_mut();
                (*entry).value = std::ptr::null_mut();

                // Mark index slot as dummy (tombstone for probe chain)
                *(*dict_obj).indices.add(slot) = DUMMY_INDEX;

                (*dict_obj).len -= 1;
                return value;
            }
        }

        std::ptr::null_mut()
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

        // Reset indices to EMPTY
        for i in 0..(*dict_obj).indices_capacity {
            *(*dict_obj).indices.add(i) = EMPTY_INDEX;
        }

        // Clear entries
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            (*entry).hash = 0;
            (*entry).key = std::ptr::null_mut();
            (*entry).value = std::ptr::null_mut();
        }

        (*dict_obj).len = 0;
        (*dict_obj).entries_len = 0;
    }
}

/// Create a shallow copy of dictionary (preserves insertion order)
/// Returns: pointer to new DictObj
#[no_mangle]
pub extern "C" fn rt_dict_copy(dict: *mut Obj) -> *mut Obj {
    if dict.is_null() {
        return rt_make_dict(8);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_copy");
        let src = dict as *mut DictObj;
        let new_dict = rt_make_dict((*src).len as i64);

        // Iterate entries in insertion order
        for i in 0..(*src).entries_len {
            let entry = (*src).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
                rt_dict_set(new_dict, key, (*entry).value);
            }
        }

        new_dict
    }
}

/// Get list of all keys in dictionary (insertion order)
/// elem_tag controls the result list's storage format:
///   0 (ELEM_HEAP_OBJ) — store as-is (keys are already *mut Obj)
///   1 (ELEM_RAW_INT) — unbox IntObj to raw i64
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_dict_keys(dict: *mut Obj, elem_tag: u8) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0, elem_tag);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_keys");
        let dict_obj = dict as *mut DictObj;
        let len = (*dict_obj).len;

        let keys_list = rt_make_list(len as i64, elem_tag);
        let list_obj = keys_list as *mut ListObj;

        // Protect keys_list from GC during iteration. Although the loop body
        // does not currently perform GC allocations, rooting it here is a
        // safety net against future changes and matches the pattern used
        // throughout the runtime.
        let mut roots: [*mut Obj; 1] = [keys_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate entries in insertion order
        let mut idx = 0usize;
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
                if elem_tag == ELEM_RAW_INT {
                    let raw_val = crate::boxing::rt_unbox_int(key);
                    *(*list_obj).data.add(idx) = raw_val as *mut Obj;
                } else {
                    *(*list_obj).data.add(idx) = key;
                }
                idx += 1;
            }
        }
        (*list_obj).len = idx;

        gc_pop();
        keys_list
    }
}

/// Get list of all values in dictionary (insertion order)
/// elem_tag controls the result list's storage format:
///   0 (ELEM_HEAP_OBJ) — store as-is (boxed values)
///   1 (ELEM_RAW_INT) — unbox IntObj to raw i64
/// Returns: pointer to new ListObj
#[no_mangle]
pub extern "C" fn rt_dict_values(dict: *mut Obj, elem_tag: u8) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0, elem_tag);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_values");
        let dict_obj = dict as *mut DictObj;
        let len = (*dict_obj).len;

        let values_list = rt_make_list(len as i64, elem_tag);
        let list_obj = values_list as *mut ListObj;

        // Protect values_list from GC during iteration. Although the loop body
        // does not currently perform GC allocations, rooting it here is a
        // safety net against future changes and matches the pattern used
        // throughout the runtime.
        let mut roots: [*mut Obj; 1] = [values_list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate entries in insertion order
        let mut idx = 0usize;
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
                let value = (*entry).value;
                if elem_tag == ELEM_RAW_INT {
                    let raw_val = crate::boxing::rt_unbox_int(value);
                    *(*list_obj).data.add(idx) = raw_val as *mut Obj;
                } else {
                    *(*list_obj).data.add(idx) = value;
                }
                idx += 1;
            }
        }
        (*list_obj).len = idx;

        gc_pop();
        values_list
    }
}

/// Get list of (key, value) tuples for all entries (insertion order)
/// Returns: pointer to new ListObj containing TupleObj elements
#[no_mangle]
pub extern "C" fn rt_dict_items(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if dict.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_items");
        let dict_obj = dict as *mut DictObj;
        let len = (*dict_obj).len;

        let items_list = rt_make_list(len as i64, ELEM_HEAP_OBJ);

        // CRITICAL: Protect items_list from GC. rt_make_tuple and rt_list_push
        // both trigger GC allocations inside the loop, so items_list must be
        // rooted or it may be collected between iterations.
        //
        // The roots slot at index 1 is reserved for the per-iteration tuple so
        // that it is also reachable during the rt_list_push call that follows.
        let mut roots: [*mut Obj; 2] = [items_list, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Iterate entries in insertion order
        for i in 0..(*dict_obj).entries_len {
            let entry = (*dict_obj).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
                // Root the tuple before rt_list_push, which may allocate
                let tuple = rt_make_tuple(2, ELEM_HEAP_OBJ);
                roots[1] = tuple;
                rt_tuple_set(tuple, 0, key);
                rt_tuple_set(tuple, 1, (*entry).value);
                rt_list_push(items_list, tuple);
                roots[1] = std::ptr::null_mut();
            }
        }

        gc_pop();
        items_list
    }
}

/// Finalize a dictionary by freeing its indices and entries arrays.
/// Called by GC during sweep phase before freeing the DictObj itself.
///
/// # Safety
/// The caller must ensure that `dict` is a valid pointer to a DictObj
/// that is about to be deallocated.
pub unsafe fn dict_finalize(dict: *mut Obj) {
    use std::alloc::{dealloc, Layout};

    if dict.is_null() {
        return;
    }

    let dict_obj = dict as *mut DictObj;

    // Free indices array
    let indices = (*dict_obj).indices;
    let indices_capacity = (*dict_obj).indices_capacity;
    if !indices.is_null() && indices_capacity > 0 {
        let layout = Layout::array::<i64>(indices_capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(indices as *mut u8, layout);
    }

    // Free entries array
    let entries = (*dict_obj).entries;
    let entries_capacity = (*dict_obj).entries_capacity;
    if !entries.is_null() && entries_capacity > 0 {
        let layout = Layout::array::<DictEntry>(entries_capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(entries as *mut u8, layout);
    }
}

/// Update dictionary with entries from another dictionary (preserves insertion order of other)
#[no_mangle]
pub extern "C" fn rt_dict_update(dict: *mut Obj, other: *mut Obj) {
    if dict.is_null() || other.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_update");
        debug_assert_type_tag!(other, TypeTagKind::Dict, "rt_dict_update");
        let other_dict = other as *mut DictObj;

        // Iterate other's entries in insertion order
        for i in 0..(*other_dict).entries_len {
            let entry = (*other_dict).entries.add(i);
            let key = (*entry).key;
            if !key.is_null() {
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

        let existing = rt_dict_get(dict, key);

        if existing.is_null() {
            rt_dict_set(dict, key, default);
            default
        } else {
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

        // Scan entries backwards to find the last active entry (insertion order)
        let mut last_idx = (*dict_obj).entries_len;
        while last_idx > 0 {
            last_idx -= 1;
            let entry = (*dict_obj).entries.add(last_idx);
            if !(*entry).key.is_null() {
                let key = (*entry).key;
                let value = (*entry).value;

                // Create result tuple
                let tuple = rt_make_tuple(2, ELEM_HEAP_OBJ);
                rt_tuple_set(tuple, 0, key);
                rt_tuple_set(tuple, 1, value);

                // Delete the entry: null out key/value in entries, mark index as dummy
                let hash = (*entry).hash;
                (*entry).key = std::ptr::null_mut();
                (*entry).value = std::ptr::null_mut();

                // Find and mark the corresponding index slot as DUMMY
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
                        break; // Shouldn't happen, but be safe
                    }
                }

                (*dict_obj).len -= 1;

                // Shrink entries_len if we removed the last entry
                while (*dict_obj).entries_len > 0 {
                    let e = (*dict_obj).entries.add((*dict_obj).entries_len - 1);
                    if (*e).key.is_null() {
                        (*dict_obj).entries_len -= 1;
                    } else {
                        break;
                    }
                }

                return tuple;
            }
        }

        // Should not reach here if len > 0
        let msg = b"KeyError: 'popitem(): dictionary is empty'";
        rt_exc_raise(ExceptionType::KeyError as u8, msg.as_ptr(), msg.len());
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
            let val = if value.is_null() {
                std::ptr::null_mut()
            } else {
                value
            };
            rt_dict_set(dict, key, val);
        }

        dict
    }
}

/// Merge two dicts into a new dict (preserves insertion order)
/// Returns: pointer to new DictObj
#[no_mangle]
pub extern "C" fn rt_dict_merge(dict1: *mut Obj, dict2: *mut Obj) -> *mut Obj {
    let result = rt_make_dict(0);

    if !dict1.is_null() {
        unsafe {
            debug_assert_type_tag!(dict1, TypeTagKind::Dict, "rt_dict_merge");
            let d1 = dict1 as *mut DictObj;
            for i in 0..(*d1).entries_len {
                let entry = (*d1).entries.add(i);
                if !(*entry).key.is_null() {
                    rt_dict_set(result, (*entry).key, (*entry).value);
                }
            }
        }
    }

    if !dict2.is_null() {
        unsafe {
            debug_assert_type_tag!(dict2, TypeTagKind::Dict, "rt_dict_merge");
            let d2 = dict2 as *mut DictObj;
            for i in 0..(*d2).entries_len {
                let entry = (*d2).entries.add(i);
                if !(*entry).key.is_null() {
                    rt_dict_set(result, (*entry).key, (*entry).value);
                }
            }
        }
    }

    result
}
