//! Shared utilities for hash table-based collections (dict, set)
//!
//! This module provides common hashing and equality functions
//! used by both dictionaries and sets.

use crate::object::{FloatObj, Obj, StrObj, TupleObj, TypeTagKind};
use pyaot_core_defs::Value;

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
    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        return hash_int(v.unwrap_int());
    }
    if v.is_bool() {
        return hash_int(if v.unwrap_bool() { 1 } else { 0 });
    }
    if v.is_none() {
        return 0;
    }
    match (*obj).type_tag() {
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
    // Fast path: pointer equality (catches interned strings, pooled ints, bool singletons)
    if a == b {
        return true;
    }
    if a.is_null() || b.is_null() {
        return false;
    }

    // Extract the "semantic type" for each side using Value tags for primitives,
    // heap header for pointer values. This avoids dereferencing tagged primitives.
    let va = Value(a as u64);
    let vb = Value(b as u64);

    // Helper: extract int-like value from a tagged primitive (Int or Bool as i64).
    // Returns None if this side is a heap pointer.
    let as_int_primitive = |v: Value| -> Option<i64> {
        if v.is_int() {
            Some(v.unwrap_int())
        } else if v.is_bool() {
            Some(v.unwrap_bool() as i64)
        } else {
            None
        }
    };

    // If both are primitives, use Value-level comparison.
    if !va.is_ptr() && !vb.is_ptr() {
        // Same tag: Int==Int or Bool==Bool bit comparison.
        if va.is_int() && vb.is_int() {
            return va.unwrap_int() == vb.unwrap_int();
        }
        if va.is_bool() && vb.is_bool() {
            return va.unwrap_bool() == vb.unwrap_bool();
        }
        // None==None
        if va.is_none() && vb.is_none() {
            return true;
        }
        // Cross-type Int/Bool equality (Python: 1 == True, 0 == False)
        if let (Some(ia), Some(ib)) = (as_int_primitive(va), as_int_primitive(vb)) {
            return ia == ib;
        }
        return false;
    }

    // One is a primitive, one is a heap pointer.
    // Only Int/Bool primitives can equal heap Floats (e.g., 1 == 1.0).
    if !va.is_ptr() || !vb.is_ptr() {
        let (prim_v, heap_ptr) = if !va.is_ptr() { (va, b) } else { (vb, a) };
        if let Some(int_val) = as_int_primitive(prim_v) {
            // Check if heap side is Float with matching value
            if !heap_ptr.is_null() && (*heap_ptr).type_tag() == TypeTagKind::Float {
                let fv = (*(heap_ptr as *mut FloatObj)).value;
                return fv.fract() == 0.0 && fv.is_finite() && fv as i64 == int_val;
            }
        }
        return false;
    }

    // Both are heap pointers — safe to dereference.
    let tag_a = (*a).type_tag();
    let tag_b = (*b).type_tag();
    if tag_a != tag_b {
        // Cross-type equality for heap types: Float vs Int (heap Int not possible in Stage C,
        // but keep the check for correctness during the mixed-migration period).
        return match (tag_a, tag_b) {
            (TypeTagKind::Int, TypeTagKind::Float) | (TypeTagKind::Float, TypeTagKind::Int) => {
                let (int_val, float_val) = if tag_a == TypeTagKind::Int {
                    (Value(a as u64).unwrap_int(), (*(b as *mut FloatObj)).value)
                } else {
                    (Value(b as u64).unwrap_int(), (*(a as *mut FloatObj)).value)
                };
                float_val.fract() == 0.0 && float_val.is_finite() && float_val as i64 == int_val
            }
            _ => false,
        };
    }
    match tag_a {
        TypeTagKind::Str => {
            let str_a = a as *mut StrObj;
            let str_b = b as *mut StrObj;
            let len = (*str_a).len;
            if len != (*str_b).len {
                return false;
            }
            let data_a = (*str_a).data.as_ptr();
            let data_b = (*str_b).data.as_ptr();
            std::slice::from_raw_parts(data_a, len) == std::slice::from_raw_parts(data_b, len)
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
            for i in 0..(*tuple_a).len {
                let ea = *(*tuple_a).data.as_ptr().add(i);
                let eb = *(*tuple_b).data.as_ptr().add(i);
                if !eq_hashable_obj(ea.0 as *mut Obj, eb.0 as *mut Obj) {
                    return false;
                }
            }
            true
        }
        TypeTagKind::None => true, // None singleton — always equal
        _ => a == b,               // Pointer equality for other types
    }
}

/// Probe configuration for compact hash table slot search.
pub struct CompactProbeConfig {
    /// Value meaning "slot is empty" (e.g., `EMPTY_INDEX = -1`)
    pub empty: i64,
    /// Value meaning "slot is a tombstone" (e.g., `DUMMY_INDEX = -2`)
    pub dummy: i64,
    /// When true, track tombstones and return the best insertion slot
    pub for_insert: bool,
}

/// Find a slot in a compact-layout (dict-style) hash table's indices array.
///
/// Dict uses a two-level structure:
///   - **indices table** (`capacity` slots): each slot holds an entry index, or an
///     `empty_sentinel` / `dummy_sentinel` (tombstone).
///   - **entries array** (dense): entry_idx points to a `(hash, key, value)` record.
///
/// Uses the same triangular probing sequence as [`find_slot_generic`].
///
/// # Returns
/// `(slot, entry_idx)` where:
/// * Key found: `slot` = matching slot in indices table, `entry_idx >= 0`
/// * Key not found + `for_insert`: `slot` = best insertion slot, `entry_idx = -1`
/// * Key not found + `!for_insert`: `(0, -1)` — slot value is not meaningful
///
/// # Safety
/// Caller must ensure closures access valid memory and `capacity` is a power of 2.
pub unsafe fn find_compact_slot_generic<F, G, H>(
    capacity: usize,
    hash: u64,
    get_index: F,
    get_entry_key: G,
    get_entry_hash: H,
    config: CompactProbeConfig,
    search_key: *mut Obj,
) -> (usize, i64)
where
    F: Fn(usize) -> i64,
    G: Fn(i64) -> *mut Obj,
    H: Fn(i64) -> u64,
{
    if capacity == 0 {
        return (0, -1);
    }

    let mask = capacity - 1;
    let base = hash as usize;
    let mut first_available: i64 = -1;

    for probe in 0..capacity {
        let offset = (probe * (probe + 1)) >> 1;
        let slot = (base + offset) & mask;
        let entry_idx = get_index(slot);

        if entry_idx == config.empty {
            if config.for_insert {
                let insert_slot = if first_available >= 0 {
                    first_available as usize
                } else {
                    slot
                };
                return (insert_slot, -1);
            }
            return (0, -1);
        }
        if entry_idx == config.dummy {
            if config.for_insert && first_available < 0 {
                first_available = slot as i64;
            }
            continue;
        }
        // Valid entry — check key match
        if get_entry_hash(entry_idx) == hash
            && eq_hashable_obj(get_entry_key(entry_idx), search_key)
        {
            return (slot, entry_idx);
        }
    }

    // Table full (shouldn't happen with proper load factor)
    if config.for_insert {
        (first_available.max(0) as usize, -1)
    } else {
        (0, -1)
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
