//! Shared utilities for hash table-based collections (dict, set)
//!
//! This module provides common hashing and equality functions
//! used by both dictionaries and sets.

use crate::object::{BoolObj, FloatObj, IntObj, Obj, StrObj, TupleObj, TypeTagKind, ELEM_HEAP_OBJ};

// FNV-1a hash constants
pub const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
pub const FNV_PRIME: u64 = 1099511628211;

/// Hash an integer using SplitMix64 finalizer.
/// Provides excellent distribution even for sequential integers.
#[inline]
fn hash_int(value: i64) -> u64 {
    // SplitMix64 finalizer - ensures good distribution for sequential values
    let mut x = value as u64;
    x = x ^ (x >> 30);
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x = x ^ (x >> 27);
    x = x.wrapping_mul(0x94d049bb133111eb);
    x = x ^ (x >> 31);
    x
}

/// Hash a Python object for use in hash tables.
/// Supports Int, Str, Bool types. Other types return 0.
///
/// # Safety
/// `obj` must be null or a valid pointer to an Obj.
pub unsafe fn hash_hashable_obj(obj: *mut Obj) -> u64 {
    if obj.is_null() {
        return 0;
    }
    match (*obj).type_tag() {
        TypeTagKind::Int => {
            let int_obj = obj as *mut IntObj;
            hash_int((*int_obj).value)
        }
        TypeTagKind::Str => {
            let str_obj = obj as *mut StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            // FNV-1a hash
            let mut hash = FNV_OFFSET_BASIS;
            for i in 0..len {
                hash ^= *data.add(i) as u64;
                hash = hash.wrapping_mul(FNV_PRIME);
            }
            hash
        }
        TypeTagKind::Bool => {
            let bool_obj = obj as *mut BoolObj;
            // True == 1, False == 0 in Python; use int hash for cross-type invariant
            hash_int(if (*bool_obj).value { 1 } else { 0 })
        }
        TypeTagKind::Float => {
            let float_obj = obj as *mut FloatObj;
            let v = (*float_obj).value;
            if v == 0.0 {
                // CPython compat: hash(-0.0) == hash(0.0) == 0
                return 0;
            }
            if v.fract() == 0.0 && v.is_finite() {
                // Integer-valued float: same hash as the equivalent integer
                hash_int(v as i64)
            } else {
                // Non-integer float: use bit representation as input to the scramble
                hash_int(v.to_bits() as i64)
            }
        }
        TypeTagKind::Tuple => crate::hash::rt_hash_tuple(obj) as u64,
        TypeTagKind::None => 0, // CPython: hash(None) == 0
        _ => 0,                 // Unhashable types return 0
    }
}

/// Check equality of two hashable Python objects.
/// Used for hash table key/element comparison.
///
/// # Safety
/// `a` and `b` must be null or valid pointers to Obj.
pub unsafe fn eq_hashable_obj(a: *mut Obj, b: *mut Obj) -> bool {
    if a.is_null() || b.is_null() {
        return a == b;
    }
    let tag_a = (*a).type_tag();
    let tag_b = (*b).type_tag();
    if tag_a != tag_b {
        // Cross-type equality: Int == Bool, Int == Float, Bool == Float
        return match (tag_a, tag_b) {
            (TypeTagKind::Int, TypeTagKind::Bool) | (TypeTagKind::Bool, TypeTagKind::Int) => {
                let int_val = if tag_a == TypeTagKind::Int {
                    (*(a as *mut IntObj)).value
                } else if (*(a as *mut BoolObj)).value {
                    1
                } else {
                    0
                };
                let other_val = if tag_b == TypeTagKind::Int {
                    (*(b as *mut IntObj)).value
                } else if (*(b as *mut BoolObj)).value {
                    1
                } else {
                    0
                };
                int_val == other_val
            }
            (TypeTagKind::Int, TypeTagKind::Float) | (TypeTagKind::Float, TypeTagKind::Int) => {
                let (int_val, float_val) = if tag_a == TypeTagKind::Int {
                    ((*(a as *mut IntObj)).value, (*(b as *mut FloatObj)).value)
                } else {
                    ((*(b as *mut IntObj)).value, (*(a as *mut FloatObj)).value)
                };
                float_val.fract() == 0.0 && float_val.is_finite() && float_val as i64 == int_val
            }
            (TypeTagKind::Bool, TypeTagKind::Float) | (TypeTagKind::Float, TypeTagKind::Bool) => {
                let (bool_val, float_val) = if tag_a == TypeTagKind::Bool {
                    (
                        if (*(a as *mut BoolObj)).value { 1i64 } else { 0 },
                        (*(b as *mut FloatObj)).value,
                    )
                } else {
                    (
                        if (*(b as *mut BoolObj)).value { 1i64 } else { 0 },
                        (*(a as *mut FloatObj)).value,
                    )
                };
                float_val.fract() == 0.0 && float_val.is_finite() && float_val as i64 == bool_val
            }
            _ => false,
        };
    }
    match tag_a {
        TypeTagKind::Int => {
            let int_a = a as *mut IntObj;
            let int_b = b as *mut IntObj;
            (*int_a).value == (*int_b).value
        }
        TypeTagKind::Str => {
            let str_a = a as *mut StrObj;
            let str_b = b as *mut StrObj;
            if (*str_a).len != (*str_b).len {
                return false;
            }
            let len = (*str_a).len;
            let data_a = (*str_a).data.as_ptr();
            let data_b = (*str_b).data.as_ptr();
            for i in 0..len {
                if *data_a.add(i) != *data_b.add(i) {
                    return false;
                }
            }
            true
        }
        TypeTagKind::Bool => {
            let bool_a = a as *mut BoolObj;
            let bool_b = b as *mut BoolObj;
            (*bool_a).value == (*bool_b).value
        }
        TypeTagKind::Float => {
            let float_a = a as *mut FloatObj;
            let float_b = b as *mut FloatObj;
            (*float_a).value == (*float_b).value
        }
        TypeTagKind::Tuple => {
            let tuple_a = a as *mut TupleObj;
            let tuple_b = b as *mut TupleObj;
            if (*tuple_a).len != (*tuple_b).len {
                return false;
            }
            if (*tuple_a).elem_tag != (*tuple_b).elem_tag {
                return false;
            }
            let elem_tag = (*tuple_a).elem_tag;
            for i in 0..(*tuple_a).len {
                let ea = *(*tuple_a).data.as_ptr().add(i);
                let eb = *(*tuple_b).data.as_ptr().add(i);
                if elem_tag == ELEM_HEAP_OBJ {
                    if !eq_hashable_obj(ea, eb) {
                        return false;
                    }
                } else {
                    // Raw values (ELEM_RAW_INT, ELEM_RAW_BOOL): compare as raw bits
                    if ea != eb {
                        return false;
                    }
                }
            }
            true
        }
        TypeTagKind::None => true, // None singleton — always equal
        _ => a == b,               // Pointer equality for other types
    }
}

/// Generic hash table slot finder using triangular probing with mask-based indexing.
/// Used by both dict and set implementations.
///
/// Uses triangular number probing: offset = i*(i+1)/2 for probe step i.
/// This avoids primary clustering (unlike linear probing) and guarantees
/// visiting all slots exactly once for power-of-2 table sizes.
///
/// Probe sequence: hash, hash+1, hash+3, hash+6, hash+10, hash+15, ...
///
/// # Arguments
/// * `capacity` - Size of the hash table (MUST be power of 2)
/// * `hash` - Hash of the key/element to find
/// * `for_insert` - Whether we're looking for an insertion slot
/// * `get_entry_key` - Closure that returns the key/element at index i
/// * `get_entry_hash` - Closure that returns the stored hash at index i
/// * `tombstone` - The tombstone marker pointer
/// * `search_key` - The key/element we're searching for
///
/// # Returns
/// Index of found slot, or -1 if not found
///
/// # Safety
/// Caller must ensure closures access valid memory and capacity is power of 2.
pub unsafe fn find_slot_generic<F, G>(
    capacity: usize,
    hash: u64,
    for_insert: bool,
    get_entry_key: F,
    get_entry_hash: G,
    tombstone: *mut Obj,
    search_key: *mut Obj,
) -> i64
where
    F: Fn(usize) -> *mut Obj,
    G: Fn(usize) -> u64,
{
    if capacity == 0 {
        return -1;
    }

    // Use mask-based indexing (requires power-of-2 capacity)
    let mask = capacity - 1;
    let base = hash as usize;
    let mut tombstone_idx: i64 = -1;

    // Triangular probing: index = (base + i*(i+1)/2) & mask
    // This visits all slots exactly once for power-of-2 capacity
    for i in 0..capacity {
        // Triangular number: 0, 1, 3, 6, 10, 15, 21, ...
        let offset = (i * (i + 1)) >> 1;
        let index = (base + offset) & mask;

        let entry_key = get_entry_key(index);

        if entry_key.is_null() {
            // Empty slot
            if for_insert {
                return if tombstone_idx >= 0 {
                    tombstone_idx
                } else {
                    index as i64
                };
            }
            return -1;
        } else if entry_key == tombstone {
            // Tombstone - remember for insertion
            if tombstone_idx < 0 {
                tombstone_idx = index as i64;
            }
        } else if get_entry_hash(index) == hash && eq_hashable_obj(entry_key, search_key) {
            // Found matching key
            return index as i64;
        }
    }

    // Table is full (shouldn't happen with proper load factor)
    if for_insert {
        tombstone_idx
    } else {
        -1
    }
}
