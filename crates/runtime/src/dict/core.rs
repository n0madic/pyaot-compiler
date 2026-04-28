//! Core dictionary operations: creation, lookup, resize, finalization

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::hash_table_utils::{find_compact_slot_generic, CompactProbeConfig};
use crate::object::{DictEntry, DictObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Sentinel value for empty slot in indices table
pub(super) const EMPTY_INDEX: i64 = -1;
/// Sentinel value for deleted slot in indices table (tombstone for probe chain)
pub(super) const DUMMY_INDEX: i64 = -2;

/// Maximum string length to intern for dict keys
pub(super) const MAX_DICT_KEY_INTERN_LENGTH: usize = 256;

/// Bit position of the factory_tag byte packed into the high byte of
/// `DictObj::entries_capacity` for DefaultDict objects.
pub(crate) const FACTORY_TAG_SHIFT: usize = 56;
/// Mask for the real entries_capacity stored in the lower 56 bits.
pub(crate) const CAPACITY_MASK: usize = (1usize << FACTORY_TAG_SHIFT) - 1;

/// Return the real entries_capacity for any DictObj.
///
/// DefaultDict objects pack a factory_tag into the high byte of
/// `entries_capacity`. This helper strips that byte so all dict
/// operations use the correct physical capacity.
#[inline]
pub(crate) fn real_entries_capacity(dict: *mut DictObj) -> usize {
    unsafe { (*dict).entries_capacity & CAPACITY_MASK }
}

/// Write a new real entries_capacity while preserving any packed factory_tag
/// in the high byte.
#[inline]
pub(crate) unsafe fn set_real_entries_capacity(dict: *mut DictObj, capacity: usize) {
    let tag_byte = (*dict).entries_capacity & !CAPACITY_MASK; // high byte(s) only
    (*dict).entries_capacity = tag_byte | (capacity & CAPACITY_MASK);
}

/// Round up to the next power of 2 (required for mask-based probing).
#[inline]
pub(super) fn next_power_of_2(n: usize) -> usize {
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
pub(super) unsafe fn lookup_entry(dict: *mut DictObj, key: *mut Obj, hash: u64) -> i64 {
    let (_, entry_idx) = find_compact_slot_generic(
        (*dict).indices_capacity,
        hash,
        |slot| *(*dict).indices.add(slot),
        |ei| (*(*dict).entries.add(ei as usize)).key.0 as *mut Obj,
        |ei| (*(*dict).entries.add(ei as usize)).hash,
        CompactProbeConfig {
            empty: EMPTY_INDEX,
            dummy: DUMMY_INDEX,
            for_insert: false,
        },
        key,
    );
    entry_idx
}

/// Find a slot in the indices table for insertion.
/// Returns (best_slot_for_insert, entry_index_if_found).
/// If found: entry_index >= 0 (existing entry to update)
/// If not found: entry_index == -1, slot is the best position for a new index entry
#[inline]
pub(super) unsafe fn find_insert_slot(
    dict: *mut DictObj,
    key: *mut Obj,
    hash: u64,
) -> (usize, i64) {
    find_compact_slot_generic(
        (*dict).indices_capacity,
        hash,
        |slot| *(*dict).indices.add(slot),
        |ei| (*(*dict).entries.add(ei as usize)).key.0 as *mut Obj,
        |ei| (*(*dict).entries.add(ei as usize)).hash,
        CompactProbeConfig {
            empty: EMPTY_INDEX,
            dummy: DUMMY_INDEX,
            for_insert: true,
        },
        key,
    )
}

/// Rebuild indices table and compact entries array.
/// Called when load factor is too high.
pub(super) unsafe fn dict_resize(dict: *mut DictObj) {
    use std::alloc::{alloc_zeroed, dealloc, Layout};

    let old_entries = (*dict).entries;
    let old_entries_len = (*dict).entries_len;
    // Use real capacity: DefaultDict packs factory_tag into the high byte.
    let old_entries_capacity = real_entries_capacity(dict);
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
        if key.0 == 0 {
            continue; // Skip deleted entries (Value(0) = empty/deleted)
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

    // Update dict. Use set_real_entries_capacity to preserve any packed
    // factory_tag in the high byte (DefaultDict objects).
    (*dict).indices = new_indices;
    (*dict).indices_capacity = new_indices_capacity;
    (*dict).entries = new_entries;
    (*dict).entries_len = new_len;
    set_real_entries_capacity(dict, new_entries_capacity);
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
pub fn rt_make_dict(capacity: i64) -> *mut Obj {
    use std::alloc::{alloc_zeroed, Layout};

    // Ensure capacity is power of 2 for efficient mask-based probing.
    // Account for load factor: if the caller requests N item slots, we need
    // the indices table to be large enough that N items fit at ≤66% load.
    // Using N * 3/2 as the minimum indices size achieves this.
    let indices_capacity = if capacity <= 0 {
        8
    } else {
        let needed = (capacity as usize * 3 / 2).max(8);
        next_power_of_2(needed)
    };
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
#[export_name = "rt_make_dict"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_dict_abi(capacity: i64) -> Value {
    Value::from_ptr(rt_make_dict(capacity))
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

    // Free entries array. Use real_entries_capacity to strip any packed
    // factory_tag (DefaultDict objects pack it into the high byte).
    let entries = (*dict_obj).entries;
    let entries_capacity = real_entries_capacity(dict_obj);
    if !entries.is_null() && entries_capacity > 0 {
        let layout = Layout::array::<DictEntry>(entries_capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(entries as *mut u8, layout);
    }
}

/// Get length of dictionary (number of entries)
pub fn rt_dict_len(dict: *mut Obj) -> i64 {
    if dict.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(dict, TypeTagKind::Dict, "rt_dict_len");
        let dict_obj = dict as *mut DictObj;
        (*dict_obj).len as i64
    }
}
#[export_name = "rt_dict_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_dict_len_abi(dict: Value) -> i64 {
    rt_dict_len(dict.unwrap_ptr())
}

