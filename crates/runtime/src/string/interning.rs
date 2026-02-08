//! String interning for memory efficiency
//!
//! This module implements a string pool to deduplicate strings at runtime.
//! Strings under 256 bytes are interned (shared), reducing memory usage
//! for repetitive strings like dictionary keys in JSON workloads.
//!
//! ## Sharded Locks
//!
//! The pool uses 8 shards, each with its own mutex. This reduces lock contention
//! when multiple threads access the pool concurrently. The shard is selected by
//! `hash % NUM_SHARDS`.
//!
//! ## GC Integration
//!
//! The string pool holds raw pointers to StrObj allocations. During GC sweep,
//! `prune_string_pool()` removes entries whose strings were not marked as
//! reachable, preventing dangling pointers.

use crate::object::{Obj, StrObj, TypeTagKind};
use crate::string::core::rt_make_str_impl;
use std::collections::HashMap;
use std::sync::Mutex;

/// Maximum string length to intern (balanced approach)
const MAX_INTERN_LENGTH: usize = 256;

/// Number of shards for reduced lock contention
const NUM_SHARDS: usize = 8;

/// Entry in the string pool
struct PoolEntry {
    /// Pointer to the interned StrObj
    str_ptr: *mut Obj,
}

// PoolEntry needs to be Send because it's stored in a static Mutex
// The raw pointers are only accessed while holding the mutex
unsafe impl Send for PoolEntry {}

/// A single shard of the string pool
struct PoolShard {
    map: Option<HashMap<u64, Vec<PoolEntry>>>,
}

/// Global sharded string pool - 8 shards for reduced lock contention
/// Each shard is independently locked, so concurrent access to different
/// shards doesn't block.
static STRING_POOL_SHARDS: [Mutex<PoolShard>; NUM_SHARDS] = [
    Mutex::new(PoolShard { map: None }),
    Mutex::new(PoolShard { map: None }),
    Mutex::new(PoolShard { map: None }),
    Mutex::new(PoolShard { map: None }),
    Mutex::new(PoolShard { map: None }),
    Mutex::new(PoolShard { map: None }),
    Mutex::new(PoolShard { map: None }),
    Mutex::new(PoolShard { map: None }),
];

/// Get the shard index for a given hash
#[inline]
fn get_shard_index(hash: u64) -> usize {
    (hash as usize) % NUM_SHARDS
}

/// FNV-1a hash constants (64-bit)
const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// Compute FNV-1a hash of byte data
///
/// # Safety
/// `data` must be a valid pointer to at least `len` bytes, or `len` must be 0.
#[inline]
unsafe fn compute_fnv1a_hash(data: *const u8, len: usize) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for i in 0..len {
        hash ^= *data.add(i) as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Compare two byte slices for equality
///
/// # Safety
/// Both pointers must be valid for their respective lengths.
#[inline]
unsafe fn bytes_equal(a: *const u8, a_len: usize, b: *const u8, b_len: usize) -> bool {
    if a_len != b_len {
        return false;
    }
    for i in 0..a_len {
        if *a.add(i) != *b.add(i) {
            return false;
        }
    }
    true
}

/// Initialize the string pool (lazy - strings interned on demand)
/// Called from rt_init()
///
/// Uses lazy initialization instead of pre-populating single-char strings.
/// This reduces startup time by ~10-12ms while still providing deduplication
/// for strings that are actually used.
pub fn init_string_pool() {
    // Initialize all shards with empty maps
    // Strings will be interned on demand when rt_make_str_interned is called
    for shard in STRING_POOL_SHARDS.iter() {
        let mut guard = shard
            .lock()
            .expect("STRING_POOL shard mutex poisoned - another thread panicked");
        guard.map = Some(HashMap::new());
    }
}

/// Shutdown the string pool
/// Called from rt_shutdown()
pub fn shutdown_string_pool() {
    // Shutdown all shards
    for shard in STRING_POOL_SHARDS.iter() {
        let mut guard = shard
            .lock()
            .expect("STRING_POOL shard mutex poisoned - another thread panicked");
        guard.map = None;
    }
}

/// Create a new interned string object
///
/// For strings under 256 bytes, checks the pool first and returns
/// an existing string if found. Otherwise creates a new string and
/// adds it to the pool.
///
/// For strings >= 256 bytes, falls back to regular rt_make_str().
///
/// Uses sharded locks to reduce contention - only the relevant shard
/// is locked based on the string's hash.
///
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_make_str_interned(data: *const u8, len: usize) -> *mut Obj {
    // Fall back to regular allocation for large strings
    if len >= MAX_INTERN_LENGTH {
        return rt_make_str_impl(data, len);
    }

    let hash = compute_fnv1a_hash(data, len);
    let shard_idx = get_shard_index(hash);

    // Only lock the relevant shard - other shards remain accessible
    let mut shard_guard = STRING_POOL_SHARDS[shard_idx]
        .lock()
        .expect("STRING_POOL shard mutex poisoned - another thread panicked");

    if let Some(ref mut pool) = shard_guard.map {
        // Look for existing entry with matching hash
        if let Some(entries) = pool.get(&hash) {
            for entry in entries {
                // Verify the pointer is still valid (object not collected)
                // The string should have been marked if it's still reachable
                let str_obj = entry.str_ptr as *mut StrObj;

                // Validate it's still a string (sanity check)
                if (*entry.str_ptr).header.type_tag != TypeTagKind::Str {
                    continue;
                }

                let existing_len = (*str_obj).len;
                let existing_data = (*str_obj).data.as_ptr();

                // Compare actual bytes (hash collision handling)
                if bytes_equal(data, len, existing_data, existing_len) {
                    // Found matching string - return cached pointer
                    return entry.str_ptr;
                }
            }
        }

        // Not found - create new string and add to pool
        let new_str = rt_make_str_impl(data, len);

        pool.entry(hash)
            .or_insert_with(Vec::new)
            .push(PoolEntry { str_ptr: new_str });

        new_str
    } else {
        // Pool not initialized, fall back to regular allocation
        rt_make_str_impl(data, len)
    }
}

/// Prune dead strings from the pool during GC sweep
///
/// Called BEFORE clearing mark bits in sweep phase.
/// Removes entries whose strings were not marked as reachable.
///
/// Iterates through all shards, locking each one in turn.
///
/// # Safety
/// Must only be called during GC sweep phase before marks are cleared.
pub unsafe fn prune_string_pool() {
    // Prune all shards
    for shard in STRING_POOL_SHARDS.iter() {
        let mut guard = shard
            .lock()
            .expect("STRING_POOL shard mutex poisoned - another thread panicked");

        if let Some(ref mut pool) = guard.map {
            // Retain only entries where the string is still marked
            pool.retain(|_hash, entries| {
                entries.retain(|entry| {
                    if entry.str_ptr.is_null() {
                        return false;
                    }
                    // Keep entry only if the string object is marked
                    (*entry.str_ptr).is_marked()
                });
                // Remove hash bucket if empty
                !entries.is_empty()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc;

    fn setup() {
        gc::init();
        init_string_pool();
    }

    fn teardown() {
        shutdown_string_pool();
        gc::shutdown();
    }

    #[test]
    fn test_interning_deduplication() {
        setup();

        unsafe {
            let data1 = b"hello";
            let data2 = b"hello";

            let str1 = rt_make_str_interned(data1.as_ptr(), data1.len());
            let str2 = rt_make_str_interned(data2.as_ptr(), data2.len());

            // Both should return the same pointer
            assert_eq!(
                str1, str2,
                "Interned strings should share the same allocation"
            );

            // Verify the string content
            let str_obj = str1 as *mut StrObj;
            assert_eq!((*str_obj).len, 5);
        }

        teardown();
    }

    #[test]
    fn test_interning_different_strings() {
        setup();

        unsafe {
            let data1 = b"hello";
            let data2 = b"world";

            let str1 = rt_make_str_interned(data1.as_ptr(), data1.len());
            let str2 = rt_make_str_interned(data2.as_ptr(), data2.len());

            // Different strings should have different pointers
            assert_ne!(
                str1, str2,
                "Different strings should have different allocations"
            );
        }

        teardown();
    }

    #[test]
    fn test_interning_size_threshold() {
        setup();

        unsafe {
            // Create a string exactly at the threshold
            let large_data = vec![b'x'; MAX_INTERN_LENGTH];
            let str1 = rt_make_str_interned(large_data.as_ptr(), large_data.len());
            let str2 = rt_make_str_interned(large_data.as_ptr(), large_data.len());

            // Strings >= 256 bytes should NOT be interned (different pointers)
            assert_ne!(str1, str2, "Large strings should not be interned");
        }

        teardown();
    }

    #[test]
    fn test_interning_empty_string() {
        setup();

        unsafe {
            let empty: &[u8] = b"";

            let str1 = rt_make_str_interned(empty.as_ptr(), 0);
            let str2 = rt_make_str_interned(empty.as_ptr(), 0);

            // Empty strings should be interned
            assert_eq!(str1, str2, "Empty strings should be interned");

            // Verify length is 0
            let str_obj = str1 as *mut StrObj;
            assert_eq!((*str_obj).len, 0);
        }

        teardown();
    }

    #[test]
    fn test_fnv1a_hash() {
        unsafe {
            // Test known FNV-1a values
            let empty_hash = compute_fnv1a_hash(std::ptr::null(), 0);
            assert_eq!(empty_hash, FNV_OFFSET_BASIS);

            let hello = b"hello";
            let hash1 = compute_fnv1a_hash(hello.as_ptr(), hello.len());
            let hash2 = compute_fnv1a_hash(hello.as_ptr(), hello.len());
            assert_eq!(hash1, hash2, "Same data should produce same hash");

            let world = b"world";
            let hash3 = compute_fnv1a_hash(world.as_ptr(), world.len());
            assert_ne!(hash1, hash3, "Different data should produce different hash");
        }
    }

    #[test]
    fn test_bytes_equal() {
        unsafe {
            let a = b"hello";
            let b = b"hello";
            let c = b"world";
            let d = b"hell";

            assert!(bytes_equal(a.as_ptr(), a.len(), b.as_ptr(), b.len()));
            assert!(!bytes_equal(a.as_ptr(), a.len(), c.as_ptr(), c.len()));
            assert!(!bytes_equal(a.as_ptr(), a.len(), d.as_ptr(), d.len()));
        }
    }
}
