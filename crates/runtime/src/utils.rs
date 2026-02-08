//! Internal utility functions for runtime
//!
//! This module provides convenience wrappers for common operations
//! used throughout the runtime. These are internal helpers, not
//! part of the public runtime API (no `#[no_mangle]` or `extern "C"`).

use crate::exceptions::ExceptionType;
use crate::object::{Obj, StrObj, TypeTagKind};
use crate::string::{rt_str_data, rt_str_len};

/// Create a string object from a Rust &str
///
/// This is a convenience wrapper for internal use.
/// For the public API, use `rt_make_str`.
///
/// # Safety
/// The returned pointer is valid until the next GC cycle.
pub unsafe fn make_str_from_rust(s: &str) -> *mut Obj {
    crate::string::rt_make_str_impl(s.as_ptr(), s.len())
}

/// Extract a Rust String from a StrObj
///
/// This is a convenience wrapper for internal use.
/// Returns an empty string if the pointer is null.
///
/// # Safety
/// The str_obj must be a valid StrObj pointer or null.
pub unsafe fn str_obj_to_rust_string(str_obj: *mut Obj) -> String {
    let data = rt_str_data(str_obj);
    let len = rt_str_len(str_obj);
    if data.is_null() || len == 0 {
        return String::new();
    }
    let slice = std::slice::from_raw_parts(data, len);
    String::from_utf8_lossy(slice).into_owned()
}

/// Extract a Rust String from a StrObj with type tag validation
///
/// Returns None if:
/// - The pointer is null
/// - The object is not a StrObj (type tag mismatch)
/// - UTF-8 conversion fails
///
/// # Safety
/// The obj must be a valid Obj pointer or null.
pub unsafe fn extract_str_checked(obj: *mut Obj) -> Option<String> {
    if obj.is_null() {
        return None;
    }

    if (*obj).header.type_tag != TypeTagKind::Str {
        return None;
    }

    let str_obj = obj as *const StrObj;
    let len = (*str_obj).len;

    if len == 0 {
        return Some(String::new());
    }

    let data = (*str_obj).data.as_ptr();
    let bytes = std::slice::from_raw_parts(data, len);

    String::from_utf8(bytes.to_vec()).ok()
}

/// Extract a Rust String from a StrObj without validation (for performance)
///
/// Unlike `extract_str_checked`, this does not validate the type tag.
/// Use when you know the object is a StrObj.
///
/// # Safety
/// The obj must be a valid StrObj pointer (not null).
pub unsafe fn extract_str_unchecked(obj: *mut Obj) -> String {
    let str_obj = obj as *const StrObj;
    let len = (*str_obj).len;

    if len == 0 {
        return String::new();
    }

    let data = (*str_obj).data.as_ptr();
    let bytes = std::slice::from_raw_parts(data, len);

    String::from_utf8_lossy(bytes).to_string()
}

/// Check if a pointer looks like a valid heap object (vs a raw primitive value)
///
/// This uses a heuristic approach because container values can be:
/// 1. Heap objects (actual pointers to allocated objects with headers)
/// 2. Raw primitive values (integers stored directly as pointer values, e.g., (void*)42)
///
/// We cannot safely dereference case 2, so we use address heuristics:
/// - Valid heap pointers are aligned (multiple of 8) and at high addresses (>= 0x10000)
/// - Raw integers are typically small values (< 0x10000)
///
/// Note: This heuristic can theoretically fail for:
/// - Very small heap addresses (unlikely on modern systems)
/// - Very large integer values that happen to be aligned (unlikely in typical use)
///
/// # Safety
/// This function is safe to call with any pointer value.
#[inline]
pub unsafe fn is_heap_obj(ptr: *mut Obj) -> bool {
    let addr = ptr as usize;
    addr >= 0x10000 && (addr & 0x7) == 0
}

/// Raise a ValueError exception with the given message
///
/// # Safety
/// This function never returns (marked with `!`).
#[inline(never)]
pub unsafe fn raise_value_error(msg: &str) -> ! {
    crate::exceptions::rt_exc_raise(ExceptionType::ValueError as u8, msg.as_ptr(), msg.len())
}

/// Raise an IOError exception with the given message
///
/// # Safety
/// This function never returns (marked with `!`).
#[inline(never)]
pub unsafe fn raise_io_error(msg: &str) -> ! {
    crate::exceptions::rt_exc_raise(ExceptionType::IOError as u8, msg.as_ptr(), msg.len())
}

/// Raise a RuntimeError exception with the given message
///
/// # Safety
/// This function never returns (marked with `!`).
#[inline(never)]
pub unsafe fn raise_runtime_error(msg: &str) -> ! {
    crate::exceptions::rt_exc_raise(ExceptionType::RuntimeError as u8, msg.as_ptr(), msg.len())
}
