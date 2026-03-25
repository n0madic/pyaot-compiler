//! Element storage tags for containers.
//!
//! Shared between compiler and runtime to avoid magic numbers.
//! These indicate whether elements in lists/tuples are boxed heap objects or raw values.

/// Elements are `*mut Obj` with valid headers
pub const ELEM_HEAP_OBJ: u8 = 0;
/// Elements are raw i64 values
pub const ELEM_RAW_INT: u8 = 1;
/// Elements are raw i8 cast to pointer
pub const ELEM_RAW_BOOL: u8 = 2;
