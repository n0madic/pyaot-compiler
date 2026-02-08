//! Central definition of built-in exception types.
//!
//! This module re-exports exception types from `pyaot-core-defs`, which serves
//! as the single source of truth for exception definitions shared between
//! the compiler and runtime.
//!
//! See `pyaot-core-defs` for usage examples and documentation.
//!
//! # Adding New Exceptions
//!
//! To add a new built-in exception, edit `crates/core-defs/src/exceptions.rs`.
//! The change will automatically propagate to both the types and runtime crates.

#![forbid(unsafe_code)]

// Re-export everything from pyaot-core-defs
pub use pyaot_core_defs::{
    exception_name_to_tag, exception_tag_to_name, is_builtin_exception_name, BuiltinException,
    BuiltinExceptionKind, BUILTIN_EXCEPTIONS, BUILTIN_EXCEPTION_COUNT,
};
