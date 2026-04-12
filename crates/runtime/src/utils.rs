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

/// Return true when `obj` represents Python `None`: either a null pointer
/// (runtime-internal "no value" sentinel / default-filled stdlib optional)
/// or the canonical `NoneObj` singleton (how the compiler boxes a user-level
/// `None` value when it flows through an `Optional[Heap]` slot).
///
/// Runtime functions that accept an `Optional[Heap]` parameter must treat
/// both representations as "absent"; otherwise explicit `f(None)` calls from
/// user code would raise spurious type errors.
///
/// # Safety
/// `obj` must be either null or a valid heap object pointer.
pub unsafe fn is_none_or_null(obj: *mut Obj) -> bool {
    obj.is_null() || (*obj).header.type_tag == crate::object::TypeTagKind::None
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
    raise_exc!(ExceptionType::ValueError, "{}", msg)
}

/// Raise an IOError exception with the given message
///
/// # Safety
/// This function never returns (marked with `!`).
#[inline(never)]
pub unsafe fn raise_io_error(msg: &str) -> ! {
    raise_exc!(ExceptionType::IOError, "{}", msg)
}

/// Format a float value the way CPython does (shortest repr that round-trips).
///
/// CPython uses David Gay's dtoa algorithm with these rules:
/// - Special: `nan`, `inf`, `-inf`, `0.0`, `-0.0`
/// - Scientific notation when exponent >= 16 or <= -5
/// - Decimal notation otherwise
/// - Always has `.` or `e`; trailing zeros stripped (but `.0` kept for whole numbers)
pub fn format_float_python(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".to_string()
        } else {
            "0.0".to_string()
        };
    }

    // Determine the decimal exponent to decide notation
    let abs_val = value.abs();
    let exp10 = abs_val.log10().floor() as i32;

    // CPython uses scientific notation when exponent >= 16 or <= -5
    if exp10 >= 16 || exp10 <= -5 {
        return format_float_scientific(value);
    }

    // For values that are whole numbers, use fixed format with .1
    if value.fract() == 0.0 && abs_val < 1e16 {
        return format!("{:.1}", value);
    }

    // Use Rust's shortest-representation formatter
    let s = format!("{}", value);
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        format!("{}.0", s)
    } else {
        s
    }
}

/// Format a float in scientific notation matching CPython's style.
/// CPython format: `[-]d[.ddd]e±dd[d]` — shortest mantissa, exponent always has ± sign
/// and at least 2 digits.
fn format_float_scientific(value: f64) -> String {
    // Use Rust's shortest-representation scientific notation
    let s = format!("{:e}", value);

    // Split into mantissa and exponent parts: "1.5e10" or "1e308"
    let (mantissa, exp_str) = match s.split_once('e') {
        Some((m, e)) => (m, e),
        None => return s,
    };

    // Parse exponent value
    let exp_val: i32 = exp_str.parse().unwrap_or(0);

    // Format exponent CPython-style: always ± sign, at least 2 digits
    let formatted_exp = if exp_val >= 0 {
        if exp_val >= 100 {
            format!("e+{}", exp_val)
        } else {
            format!("e+{:02}", exp_val)
        }
    } else if exp_val <= -100 {
        format!("e{}", exp_val)
    } else {
        format!("e-{:02}", exp_val.unsigned_abs())
    };

    format!("{}{}", mantissa, formatted_exp)
}

/// Raise a RuntimeError exception with the given message
///
/// # Safety
/// This function never returns (marked with `!`).
#[inline(never)]
pub unsafe fn raise_runtime_error(msg: &str) -> ! {
    raise_exc!(ExceptionType::RuntimeError, "{}", msg)
}
