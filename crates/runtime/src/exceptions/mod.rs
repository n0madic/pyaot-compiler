//! Exception handling support using setjmp/longjmp
//!
//! This module provides exception handling infrastructure for the AOT compiler.
//! It uses setjmp/longjmp for control flow with thread-local exception state.
//!
//! # Exception Type Definitions
//!
//! Exception types are defined in `pyaot-core-defs` crate, which serves as the
//! single source of truth shared between the compiler and runtime.
//!
//! # Module Structure
//!
//! - `core`  — Core types (`ExceptionObject`, `ExceptionFrame`) and internal raise machinery
//! - `state` — Thread-local exception state and accessors
//! - `ffi`   — All `extern "C"` functions exported to generated code

pub mod core;
mod ffi;
mod state;

// Re-export the public API so callers can use `crate::exceptions::*` as before.

pub use core::{
    assert_jmp_buf_size, exception_type_from_tag, get_exception_pointers, ExceptionFrame,
    ExceptionObject, ExceptionType, JMP_BUF_SIZE, NOT_CUSTOM_CLASS,
};

pub use ffi::{
    rt_exc_class_name, rt_exc_clear, rt_exc_end_handling, rt_exc_get_class_id, rt_exc_get_current,
    rt_exc_get_current_message, rt_exc_get_message, rt_exc_get_type, rt_exc_has_exception,
    rt_exc_instance_str, rt_exc_isinstance, rt_exc_isinstance_class, rt_exc_pop_frame,
    rt_exc_print_current, rt_exc_push_frame, rt_exc_raise, rt_exc_raise_attr_error,
    rt_exc_raise_custom, rt_exc_raise_custom_with_instance, rt_exc_raise_from,
    rt_exc_raise_from_none, rt_exc_raise_index_error, rt_exc_raise_instance,
    rt_exc_raise_key_error, rt_exc_raise_owned, rt_exc_raise_type_error, rt_exc_raise_value_error,
    rt_exc_register_class_name, rt_exc_reraise, rt_exc_start_handling,
};
