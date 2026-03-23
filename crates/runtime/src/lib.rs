//! Python AOT Runtime Library
//!
//! This is a staticlib that provides runtime support for compiled Python code.
//! It includes:
//! - Object representation
//! - Garbage collection
//! - Built-in operations
//! - Exception handling

#![allow(unsafe_code)] // Runtime needs unsafe for FFI and memory management
#![allow(clippy::not_unsafe_ptr_arg_deref)] // FFI functions inherently work with raw pointers
#![allow(clippy::missing_safety_doc)] // FFI functions are internal; callers are generated code

/// Debug assertion to verify an object has the expected type tag.
/// Only active in debug builds; compiles to nothing in release.
///
/// # Usage
/// ```ignore
/// unsafe {
///     debug_assert_type_tag!(obj, TypeTagKind::List, "rt_list_get");
///     let list_obj = obj as *mut ListObj;
///     // ...
/// }
/// ```
#[macro_export]
macro_rules! debug_assert_type_tag {
    ($obj:expr, $expected:expr, $func_name:expr) => {
        debug_assert_eq!(
            (*$obj).header.type_tag,
            $expected,
            "{}: expected {:?}, got {:?}",
            $func_name,
            $expected,
            (*$obj).header.type_tag
        );
    };
}

// Core modules
pub mod exceptions;
pub mod gc;
pub mod object;
pub mod ops;

// Type operations
pub mod boxing;
pub mod conversions;
pub mod hash;
pub mod instance;
pub mod math_ops;

// Collection types
pub mod bytes;
pub mod dict;
pub mod hash_table_utils;
pub mod list;
pub mod minmax_utils;
pub mod set;
pub mod slice_utils;
pub mod tuple;

// String operations
pub mod string;

// Internal utilities
pub mod utils;

// Iteration and sorting
pub mod iterator;
pub mod sorted;

// I/O operations
pub mod file;
pub mod print;

// Global variable storage
pub mod globals;

// Class attribute storage
pub mod class_attrs;

// Cell objects for nonlocal variables
pub mod cell;

// Generator support
pub mod generator;

// VTable and inheritance support
pub mod vtable;

// First-class builtin function support
pub mod builtins;

// Standard library modules
pub mod abc;
pub mod base64_mod;
pub mod copy;
pub mod format;
pub mod functools;
pub mod hashlib;
pub mod json;
pub mod os;
pub mod random;
pub mod re;
pub mod stringio;
pub mod subprocess;
pub mod sys;
pub mod time;
pub mod urllib_parse;
pub mod urllib_request;

// Tests
#[cfg(test)]
mod tests;

use std::ffi::CStr;
use std::io::Write;

// Re-export commonly used types
pub use object::Obj;

/// Initialize the runtime (called at program startup)
/// Takes argc/argv from main() to initialize sys.argv
///
/// # Safety
/// `argv` must be a valid pointer to an array of at least `argc` null-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn rt_init(argc: i32, argv: *const *const i8) {
    exceptions::assert_jmp_buf_size();
    gc::init();
    string::init_string_pool();
    boxing::init_small_int_pool();
    boxing::init_bool_pool();
    globals::init_globals();
    class_attrs::init_class_attrs();
    vtable::rt_init_builtin_exception_classes();
    sys::init_sys_argv(argc, argv);
}

/// Shutdown the runtime (called at program exit)
#[no_mangle]
pub extern "C" fn rt_shutdown() {
    // Flush stdout to ensure all print output is visible (e.g. when end="" is used)
    let _ = std::io::stdout().flush();

    class_attrs::shutdown_class_attrs();
    globals::shutdown_globals();
    boxing::shutdown_bool_pool();
    boxing::shutdown_small_int_pool();
    string::shutdown_string_pool();
    gc::shutdown();
}

/// Assertion failure - called when assert condition is false
/// msg_ptr is a pointer to a null-terminated C string, or null if no message
///
/// Raises an AssertionError through the exception handling system so it can be caught.
///
/// # Safety
/// `msg_ptr` must be null or a valid pointer to a null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn rt_assert_fail(msg_ptr: *const i8) -> ! {
    // Type tag 1 = AssertionError
    if msg_ptr.is_null() {
        exceptions::rt_exc_raise(1, std::ptr::null(), 0)
    } else {
        let msg = CStr::from_ptr(msg_ptr);
        let bytes = msg.to_bytes();
        exceptions::rt_exc_raise(1, bytes.as_ptr(), bytes.len())
    }
}

/// Assertion failure with string object - called when assert condition is false
/// str_obj is a pointer to a StrObj, or null if no message
///
/// Raises an AssertionError through the exception handling system so it can be caught.
///
/// # Safety
/// `str_obj` must be null or a valid pointer to a StrObj.
#[no_mangle]
pub unsafe extern "C" fn rt_assert_fail_obj(str_obj: *const object::Obj) -> ! {
    // Type tag 1 = AssertionError
    if str_obj.is_null() {
        exceptions::rt_exc_raise(1, std::ptr::null(), 0)
    } else {
        let str_obj = str_obj as *const object::StrObj;
        let len = (*str_obj).len;
        if len > 0 {
            let data = (*str_obj).data.as_ptr();
            exceptions::rt_exc_raise(1, data, len)
        } else {
            exceptions::rt_exc_raise(1, std::ptr::null(), 0)
        }
    }
}
