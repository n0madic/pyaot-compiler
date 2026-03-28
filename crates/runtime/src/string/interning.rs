//! String interning for memory efficiency
//!
//! This module implements a string pool to deduplicate strings at runtime.
//! Strings under 256 bytes are interned (shared), reducing memory usage
//! for repetitive strings like dictionary keys in JSON workloads.
//!
//! ## Lock-free Design
//!
//! The runtime is single-threaded (AOT-compiled Python has no threading),
//! so the pool uses `UnsafeCell` for zero-overhead access instead of Mutex.
//!
//! ## GC Integration
//!
//! The string pool holds raw pointers to StrObj allocations. During GC sweep,
//! `prune_string_pool()` removes entries whose strings were not marked as
//! reachable, preventing dangling pointers.

use crate::object::{Obj, StrObj, TypeTagKind};
use crate::string::core::rt_make_str_impl;
use std::cell::UnsafeCell;
use std::collections::HashMap;

/// Maximum string length to intern (balanced approach)
const MAX_INTERN_LENGTH: usize = 256;

/// Entry in the string pool
struct PoolEntry {
    /// Pointer to the interned StrObj
    str_ptr: *mut Obj,
}

/// Lock-free string pool for single-threaded access
struct StringPool {
    data: UnsafeCell<Option<HashMap<u64, Vec<PoolEntry>>>>,
}

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for StringPool {}

static STRING_POOL: StringPool = StringPool {
    data: UnsafeCell::new(None),
};

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

/// Compare two byte slices for equality using optimized slice comparison
///
/// # Safety
/// Both pointers must be valid for their respective lengths.
#[inline]
unsafe fn bytes_equal(a: *const u8, a_len: usize, b: *const u8, b_len: usize) -> bool {
    if a_len != b_len {
        return false;
    }
    if a_len == 0 {
        return true;
    }
    std::slice::from_raw_parts(a, a_len) == std::slice::from_raw_parts(b, b_len)
}

/// Initialize the string pool (lazy - strings interned on demand)
/// Called from rt_init()
pub fn init_string_pool() {
    unsafe {
        *STRING_POOL.data.get() = Some(HashMap::new());
    }
}

/// Shutdown the string pool
/// Called from rt_shutdown()
pub fn shutdown_string_pool() {
    unsafe {
        *STRING_POOL.data.get() = None;
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
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_make_str_interned(data: *const u8, len: usize) -> *mut Obj {
    // Fall back to regular allocation for large strings
    if len >= MAX_INTERN_LENGTH {
        return rt_make_str_impl(data, len);
    }

    let hash = compute_fnv1a_hash(data, len);
    let pool = &mut *STRING_POOL.data.get();

    if let Some(ref pool_map) = pool {
        if let Some(entries) = pool_map.get(&hash) {
            for entry in entries {
                let str_obj = entry.str_ptr as *mut StrObj;

                if (*entry.str_ptr).header.type_tag != TypeTagKind::Str {
                    continue;
                }

                let existing_len = (*str_obj).len;
                let existing_data = (*str_obj).data.as_ptr();

                if bytes_equal(data, len, existing_data, existing_len) {
                    return entry.str_ptr;
                }
            }
        }
    }

    // Copy the input bytes to a local stack buffer before calling gc_alloc.
    // The `data` pointer may point into a GC-managed StrObj.  If a GC
    // collection fires inside rt_make_str_impl → gc_alloc, the source object
    // may be swept and `data` would become a dangling pointer.  Copying to the
    // stack keeps the bytes alive independent of the GC heap.
    //
    // MAX_INTERN_LENGTH is 256, so the stack array is bounded and safe.
    let mut local_buf = [0u8; MAX_INTERN_LENGTH];
    if len > 0 && !data.is_null() {
        std::ptr::copy_nonoverlapping(data, local_buf.as_mut_ptr(), len);
    }
    let stable_data = local_buf.as_ptr();

    // Create new string (may trigger GC which calls prune_string_pool)
    let new_str = rt_make_str_impl(stable_data, len);

    // Insert into pool
    let pool = &mut *STRING_POOL.data.get();
    if let Some(ref mut pool_map) = pool {
        pool_map
            .entry(hash)
            .or_insert_with(Vec::new)
            .push(PoolEntry { str_ptr: new_str });
    }

    new_str
}

/// Prune dead strings from the pool during GC sweep
///
/// Called BEFORE clearing mark bits in sweep phase.
/// Removes entries whose strings were not marked as reachable.
///
/// # Safety
/// Must only be called during GC sweep phase before marks are cleared.
pub unsafe fn prune_string_pool() {
    let pool = &mut *STRING_POOL.data.get();

    if let Some(ref mut pool_map) = pool {
        pool_map.retain(|_hash, entries| {
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
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        setup();

        unsafe {
            let data1 = b"hello";
            let data2 = b"world";

            let str1 = rt_make_str_interned(data1.as_ptr(), data1.len());

            // Root str1 across the second allocation so GC stress mode does not
            // sweep and reuse its slab slot for str2, which would make the
            // assert_ne! below incorrectly fail.
            let mut roots: [*mut crate::object::Obj; 1] = [str1];
            let mut frame = crate::gc::ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 1,
                roots: roots.as_mut_ptr(),
            };
            crate::gc::gc_push(&mut frame);
            let str2 = rt_make_str_interned(data2.as_ptr(), data2.len());
            crate::gc::gc_pop();

            // Different strings should have different pointers
            assert_ne!(
                roots[0], str2,
                "Different strings should have different allocations"
            );
        }

        teardown();
    }

    #[test]
    fn test_interning_size_threshold() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        setup();

        unsafe {
            // Create a string exactly at the threshold
            let large_data = vec![b'x'; MAX_INTERN_LENGTH];
            let str1 = rt_make_str_interned(large_data.as_ptr(), large_data.len());

            // Root str1 across the second allocation so GC stress mode does not
            // sweep and reuse its slab slot for str2.
            let mut roots: [*mut crate::object::Obj; 1] = [str1];
            let mut frame = crate::gc::ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 1,
                roots: roots.as_mut_ptr(),
            };
            crate::gc::gc_push(&mut frame);
            let str2 = rt_make_str_interned(large_data.as_ptr(), large_data.len());
            crate::gc::gc_pop();

            // Strings >= 256 bytes should NOT be interned (different pointers)
            assert_ne!(roots[0], str2, "Large strings should not be interned");
        }

        teardown();
    }

    #[test]
    fn test_interning_empty_string() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
