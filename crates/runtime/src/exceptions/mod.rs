//! Exception handling support — table-based stack unwinding.
//!
//! A raise stores the exception in thread-local state, walks the
//! frame-pointer chain looking each return address up in the registered
//! PC→handler table (baked into the binary by codegen), restores the handler
//! frame's SP/FP and jumps. The happy path of a `try` block costs nothing at
//! runtime — no frame push, no setjmp.
//!
//! # Exception Type Definitions
//!
//! Exception types are defined in `pyaot-core-defs` crate, which serves as the
//! single source of truth shared between the compiler and runtime.
//!
//! # Module Structure
//!
//! - `core`   — Core types (`ExceptionObject`) and internal raise machinery
//! - `state`  — Thread-local exception state and accessors
//! - `unwind` — The unwind table, frame-pointer walker and resume stub
//! - `ffi`    — All `extern "C"` functions exported to generated code

pub mod core;
mod ffi;
mod state;
mod unwind;

// Re-export the public API so callers can use `crate::exceptions::*` as before.

pub use core::{
    exception_type_from_tag, get_exception_pointers, ExceptionObject, ExceptionType,
    NOT_CUSTOM_CLASS,
};

pub use ffi::{
    rt_exc_arm_cause_builtin, rt_exc_arm_cause_value, rt_exc_arm_suppress, rt_exc_class_name,
    rt_exc_clear, rt_exc_end_handling, rt_exc_get_class_id, rt_exc_get_current,
    rt_exc_get_current_message, rt_exc_get_message, rt_exc_get_type, rt_exc_has_exception,
    rt_exc_instance_str, rt_exc_isinstance, rt_exc_isinstance_class, rt_exc_print_current,
    rt_exc_raise, rt_exc_raise_attr_error, rt_exc_raise_custom, rt_exc_raise_custom_with_instance,
    rt_exc_raise_index_error, rt_exc_raise_instance, rt_exc_raise_key_error, rt_exc_raise_owned,
    rt_exc_raise_type_error, rt_exc_raise_value_error, rt_exc_register_class_name, rt_exc_reraise,
    rt_exc_start_handling,
};

pub use unwind::rt_exc_register_table;
pub(crate) use unwind::walk_return_pcs;
