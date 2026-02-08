//! Method call lowering
//!
//! This module handles lowering of method calls for different types.
//! Each type's methods are implemented in a separate file:
//!
//! - `dispatch`: Main method call dispatcher (routes to type-specific handlers)
//! - `string`: String methods (upper, lower, strip, etc.)
//! - `list`: List methods (append, pop, insert, etc.)
//! - `dict`: Dict methods (get, keys, values, etc.)
//! - `set`: Set methods (add, remove, discard, etc.)
//! - `file`: File methods (read, write, close, etc.)
//! - `generator`: Generator/iterator methods (send, close)
//! - `class`: Class method calls with virtual dispatch

// Module declarations
pub(super) mod bytes;
pub(super) mod class;
pub(super) mod dict;
pub(super) mod dispatch;
pub(super) mod file;
pub(super) mod generator;
pub(super) mod list;
pub(super) mod set;
pub(super) mod string;
pub(super) mod tuple;
