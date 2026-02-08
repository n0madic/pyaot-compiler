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
pub mod span;

pub use ids::*;
pub use interner::*;
pub use span::*;
