//! Core set operations: creation, find_slot, resize, finalization, min/max

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::hash_table_utils::find_slot_generic;
use crate::object::{Obj, SetObj, TypeTagKind, TOMBSTONE};
use pyaot_core_defs::Value;

/// Round up to the next power of 2 (required for mask-based probing).
/// Returns the smallest power of 2 that is >= n.
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

pub(super) fn find_set_slot(set: *mut SetObj, elem: *mut Obj, hash: u64, for_insert: bool) -> i64 {
    unsafe {
        find_slot_generic(
            (*set).capacity,
            hash,
            for_insert,
            |i| (*(*set).entries.add(i)).elem.0 as *mut Obj,
            |i| (*(*set).entries.add(i)).hash,
            TOMBSTONE.0 as *mut Obj,
            elem,
        )
    }
}

pub(super) fn set_resize(set: *mut SetObj, new_capacity: usize) {
    use crate::object::SetEntry;
    use std::alloc::{alloc_zeroed, dealloc, Layout};

    unsafe {
        let old_entries = (*set).entries;
        let old_capacity = (*set).capacity;

        // Allocate new entries array
        let new_layout = Layout::array::<SetEntry>(new_capacity)
            .expect("Allocation size overflow - capacity too large");
        let new_entries = alloc_zeroed(new_layout) as *mut SetEntry;

        // Initialize new entries
        for i in 0..new_capacity {
            let entry = new_entries.add(i);
            (*entry).hash = 0;
            (*entry).elem = pyaot_core_defs::Value(0);
        }

        // Update set with new entries
        (*set).entries = new_entries;
        (*set).capacity = new_capacity;
        (*set).len = 0;

        // Rehash existing entries using triangular probing (power-of-2 capacity)
        let mask = new_capacity - 1;
        for i in 0..old_capacity {
            let old_entry = old_entries.add(i);
            let elem = (*old_entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE {
                let hash = (*old_entry).hash;
                let base = hash as usize;

                // Find empty slot in new table using triangular probing.
                // offset = i*(i+1)/2: 0, 1, 3, 6, 10, 15, ...
                // With a power-of-2 capacity and a load factor <= 2/3, triangular
                // probing visits every slot exactly once before cycling, so an empty
                // slot is always found within `new_capacity` steps.  The bound check
                // below is a defensive safety guard; it should never trigger in
                // practice because set_resize is only called when there is spare
                // capacity, and the new_capacity is always a power of 2.
                let mut probe_i = 0usize;
                loop {
                    if probe_i >= new_capacity {
                        // This can only happen if new_capacity is too small for the
                        // existing elements, which indicates a bug in the caller.
                        // Panic here rather than looping forever.
                        panic!(
                            "set_resize: failed to find empty slot after {} probes \
                             (new_capacity={}, len={}); resize target too small",
                            probe_i,
                            new_capacity,
                            (*set).len,
                        );
                    }
                    let offset = (probe_i * (probe_i + 1)) >> 1;
                    let index = (base + offset) & mask;
                    let entry = new_entries.add(index);
                    if (*entry).elem.0 == 0 {
                        (*entry).hash = hash;
                        (*entry).elem = elem;
                        (*set).len += 1;
                        break;
                    }
                    probe_i += 1;
                }
            }
        }

        // Free old entries array
        if old_capacity > 0 && !old_entries.is_null() {
            let old_layout = Layout::array::<SetEntry>(old_capacity)
                .expect("Allocation size overflow - capacity too large");
            dealloc(old_entries as *mut u8, old_layout);
        }
    }
}

/// Allocate a new empty set with given initial capacity
/// Returns: pointer to SetObj
pub fn rt_make_set(capacity: i64) -> *mut Obj {
    use crate::object::{SetEntry, SetObj, TypeTagKind};
    use std::alloc::{alloc_zeroed, Layout};

    // Ensure capacity is power of 2 for efficient mask-based probing
    let requested = if capacity <= 0 {
        8
    } else {
        capacity.max(8) as usize
    };
    let capacity = next_power_of_2(requested);

    // Allocate SetObj using GC
    let set_size = std::mem::size_of::<SetObj>();
    let obj = gc::gc_alloc(set_size, TypeTagKind::Set as u8);

    unsafe {
        let set = obj as *mut SetObj;
        (*set).len = 0;
        (*set).capacity = capacity;

        // Allocate entries array separately
        let entries_layout = Layout::array::<SetEntry>(capacity)
            .expect("Allocation size overflow - capacity too large");
        let entries_ptr = alloc_zeroed(entries_layout) as *mut SetEntry;
        (*set).entries = entries_ptr;

        // Initialize all entries to empty (null elements)
        for i in 0..capacity {
            let entry = entries_ptr.add(i);
            (*entry).hash = 0;
            (*entry).elem = pyaot_core_defs::Value(0);
        }
    }

    obj
}
#[export_name = "rt_make_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_set_abi(capacity: i64) -> Value {
    Value::from_ptr(rt_make_set(capacity))
}


/// Get length of set (number of elements)
pub fn rt_set_len(set: *mut Obj) -> i64 {
    if set.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_len");
        let set_obj = set as *mut SetObj;
        (*set_obj).len as i64
    }
}
#[export_name = "rt_set_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_len_abi(set: Value) -> i64 {
    rt_set_len(set.unwrap_ptr())
}


/// Finalize a set by freeing its entries array
/// Called by GC during sweep phase before freeing the SetObj itself
///
/// # Safety
/// The caller must ensure that `set` is a valid pointer to a SetObj
/// that is about to be deallocated.
pub unsafe fn set_finalize(set: *mut Obj) {
    use crate::object::SetEntry;
    use std::alloc::{dealloc, Layout};

    if set.is_null() {
        return;
    }

    let set_obj = set as *mut SetObj;
    let entries = (*set_obj).entries;
    let capacity = (*set_obj).capacity;

    // Free the entries array if allocated
    if !entries.is_null() && capacity > 0 {
        let entries_layout = Layout::array::<SetEntry>(capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(entries as *mut u8, entries_layout);
    }
}

/// Generic set min/max for int and float elements.
/// is_min: 0=min, 1=max; elem_kind: 0=int, 1=float.
/// Returns i64 (for float, result is f64::to_bits()).
pub fn rt_set_minmax(set: *mut Obj, is_min: u8, elem_kind: u8) -> i64 {
    use crate::object::FloatObj;

    if set.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_minmax");
        let set_obj = set as *mut SetObj;
        let len = (*set_obj).len;
        let capacity = (*set_obj).capacity;

        if len == 0 {
            if is_min == 0 {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "min() arg is an empty sequence"
                );
            } else {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "max() arg is an empty sequence"
                );
            }
        }

        let entries = (*set_obj).entries;
        let want_min = is_min == 0;

        if elem_kind == 1 {
            // Float elements
            let mut result: Option<f64> = None;
            for i in 0..capacity {
                let entry = entries.add(i);
                let elem = (*entry).elem;
                if elem.0 != 0 && elem != TOMBSTONE {
                    let val = (*(elem.0 as *mut FloatObj)).value;
                    match result {
                        None => result = Some(val),
                        Some(current) => {
                            if (want_min && val < current) || (!want_min && val > current) {
                                result = Some(val);
                            }
                        }
                    }
                }
            }
            result.unwrap_or(0.0).to_bits() as i64
        } else {
            // Int elements
            let mut result: Option<i64> = None;
            for i in 0..capacity {
                let entry = entries.add(i);
                let elem = (*entry).elem;
                if elem.0 != 0 && elem != TOMBSTONE {
                    let val = elem.unwrap_int();
                    match result {
                        None => result = Some(val),
                        Some(current) => {
                            if (want_min && val < current) || (!want_min && val > current) {
                                result = Some(val);
                            }
                        }
                    }
                }
            }
            result.unwrap_or(0)
        }
    }
}
#[export_name = "rt_set_minmax"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_minmax_abi(set: Value, is_min: u8, elem_kind: u8) -> i64 {
    rt_set_minmax(set.unwrap_ptr(), is_min, elem_kind)
}


/// Generic set min/max with key function.
/// `key_return_tag`: 0=heap, 1=Int(raw i64), 2=Bool(raw 0/1).
/// is_min: 0=min, 1=max
pub fn rt_set_minmax_with_key(
    set: *mut Obj,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: i64,
    is_min: u8,
    key_return_tag: u8,
) -> *mut Obj {
    unsafe {
        find_set_extremum_with_key(
            set,
            key_fn,
            captures,
            capture_count as u8,
            is_min == 0,
            key_return_tag,
        )
    }
}
#[export_name = "rt_set_minmax_with_key"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_minmax_with_key_abi(
    set: Value,
    key_fn: i64,
    captures: Value,
    capture_count: i64,
    is_min: u8,
    key_return_tag: u8,
) -> Value {
    Value::from_ptr(rt_set_minmax_with_key(set.unwrap_ptr(), key_fn, captures.unwrap_ptr(), capture_count, is_min, key_return_tag))
}


/// Find extremum (min or max) element in a set using a key function.
/// Set entries store tagged Values; `key_return_tag` tells how to wrap the key fn result.
unsafe fn find_set_extremum_with_key(
    set: *mut Obj,
    key_fn: i64,
    captures: *mut Obj,
    capture_count: u8,
    is_min: bool,
    key_return_tag: u8,
) -> *mut Obj {
    use crate::iterator::call_map_with_captures;
    use crate::sorted::{compare_key_values, unwrap_slot_for_key_fn, wrap_key_result};

    if set.is_null() {
        return std::ptr::null_mut();
    }

    let set_obj = set as *mut SetObj;
    let len = (*set_obj).len;
    let capacity = (*set_obj).capacity;

    if len == 0 {
        if is_min {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "min() arg is an empty sequence"
            );
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "max() arg is an empty sequence"
            );
        }
    }

    let entries = (*set_obj).entries;
    let mut extremum_slot = pyaot_core_defs::Value(0);
    let mut extremum_key = pyaot_core_defs::Value(0);
    let mut found_first = false;

    for i in 0..capacity {
        let entry = entries.add(i);
        let elem = (*entry).elem;
        if elem.0 != 0 && elem != TOMBSTONE {
            if !found_first {
                extremum_slot = elem;
                extremum_key = wrap_key_result(
                    call_map_with_captures(
                        key_fn,
                        captures,
                        capture_count,
                        unwrap_slot_for_key_fn(elem, key_return_tag),
                    ),
                    key_return_tag,
                );
                found_first = true;
            } else {
                let key = wrap_key_result(
                    call_map_with_captures(
                        key_fn,
                        captures,
                        capture_count,
                        unwrap_slot_for_key_fn(elem, key_return_tag),
                    ),
                    key_return_tag,
                );

                let cmp = compare_key_values(key.0 as *mut Obj, extremum_key.0 as *mut Obj);
                let is_better = if is_min {
                    cmp == std::cmp::Ordering::Less
                } else {
                    cmp == std::cmp::Ordering::Greater
                };

                if is_better {
                    extremum_slot = elem;
                    extremum_key = key;
                }
            }
        }
    }

    if !found_first {
        return std::ptr::null_mut();
    }

    if extremum_slot.is_int() {
        extremum_slot.unwrap_int() as *mut Obj
    } else if extremum_slot.is_bool() {
        i64::from(extremum_slot.unwrap_bool()) as *mut Obj
    } else {
        extremum_slot.0 as *mut Obj
    }
}
