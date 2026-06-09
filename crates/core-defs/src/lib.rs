//! Core definitions shared between compiler and runtime.
//!
//! This crate is a leaf crate with no dependencies on other pyaot crates,
//! allowing both the compiler (`types`) and runtime (`runtime`) crates to
//! depend on it without creating circular dependencies.
//!
//! # Contents
//!
//! - [`BuiltinExceptionKind`] - Enum of all built-in exception types (0-13)
//! - [`TypeTagKind`] - Enum of all runtime type tags (0-15)
//! - [`Value`] and [`tag`] - Phase 2 universal tagged value representation
//!
//! # Single Source of Truth Pattern
//!
//! Previously, exception kinds and type tags were defined separately in both
//! the `types` and `runtime` crates with comments warning to keep them in sync.
//! This crate eliminates that duplication by providing a single definition.

#![forbid(unsafe_code)]

pub mod builtins;
pub mod exceptions;
pub mod layout;
pub mod runtime_func_def;
pub mod tag;
pub mod tag_kinds;
pub mod value;

pub use exceptions::{
    exception_name_to_tag, exception_tag_to_name, is_builtin_exception_name, BuiltinException,
    BuiltinExceptionKind, BUILTIN_EXCEPTIONS, BUILTIN_EXCEPTION_COUNT, FIRST_USER_CLASS_ID,
    RESERVED_BUILTIN_TYPE_SLOTS, RESERVED_STDLIB_EXCEPTION_SLOTS,
};

pub use tag_kinds::{is_type_tag_name, type_tag_to_name, TypeTagKind, TYPE_TAG_COUNT};

pub use builtins::{BuiltinFunctionKind, BUILTIN_FUNCTION_COUNT};

pub use runtime_func_def::{ParamType, ReturnType, RuntimeFuncDef};

pub use tag::{
    int_fits, BOOL_SHIFT, BOOL_TAG, INT_MAX, INT_MIN, INT_SHIFT, INT_TAG, NONE_TAG, PTR_TAG,
    RESERVED_TAG, TAG_MASK,
};
pub use value::Value;
