//! Central definition of runtime type tags.
//!
//! This module re-exports type tag definitions from `pyaot-core-defs`, which serves
//! as the single source of truth for type tags shared between the compiler and runtime.
//!
//! See `pyaot-core-defs` for usage examples and documentation.
//!
//! # Adding New Type Tags
//!
//! To add a new type tag, edit `crates/core-defs/src/tag_kinds.rs`.
//! The change will automatically propagate to both the types and runtime crates.

#![forbid(unsafe_code)]

// Re-export everything from pyaot-core-defs
pub use pyaot_core_defs::{is_type_tag_name, type_tag_to_name, TypeTagKind, TYPE_TAG_COUNT};
