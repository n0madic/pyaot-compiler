//! Utilities for the Python AOT compiler
//!
//! This module provides:
//! - ID types (FuncId, TypeId, etc.)
//! - Arena allocators
//! - String interning
//! - Common data structures

#![forbid(unsafe_code)]

pub mod ids;
pub mod interner;
pub mod line_map;
pub mod span;

pub use ids::*;
pub use interner::*;
pub use line_map::LineMap;
pub use span::*;

/// FNV-1a hash for method name strings (used by Protocol dispatch).
/// Must match the runtime implementation for correct lookup.
pub fn fnv1a_hash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
