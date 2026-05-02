//! Runtime dispatch for Python's format() builtin and f-string `FormatSpec` nodes.
//!
//! All parsing and formatting logic lives in `pyaot-format-shared`.
//! This module provides the extern-C ABI wrappers that the compiled code calls.

use crate::object::{FloatObj, Obj, StrObj};
use pyaot_core_defs::Value;
use pyaot_core_defs::{BuiltinExceptionKind, TypeTagKind};
use pyaot_format_shared as fmt;

unsafe fn raise_value_error(msg: &str) -> ! {
    raise_exc!(BuiltinExceptionKind::ValueError, "{}", msg)
}

/// Format a value according to the format specification.
///
/// This is the backing function for both f-string `FormatSpec` nodes and the
/// `format(value, spec)` Python builtin.
///
/// # Safety
/// - `value` must be a valid Value (tagged int/bool, or heap pointer to an Obj).
/// - `spec` must be null or a valid heap pointer to a StrObj.
pub unsafe fn rt_format(value: *mut Obj, spec: *mut Obj) -> *mut Obj {
    let spec_str = if spec.is_null() {
        ""
    } else {
        let spec_obj = &*(spec as *const StrObj);
        if spec_obj.header.type_tag != TypeTagKind::Str {
            raise_value_error("format spec must be a string");
        }
        if spec_obj.len == 0 {
            ""
        } else {
            let bytes = std::slice::from_raw_parts(spec_obj.data.as_ptr(), spec_obj.len);
            std::str::from_utf8(bytes)
                .unwrap_or_else(|_| raise_value_error("Invalid UTF-8 in format spec"))
        }
    };

    if spec_str.is_empty() {
        return crate::conversions::rt_obj_to_str(value);
    }

    let format_spec = match fmt::parse_format_spec(spec_str) {
        Ok(s) => s,
        Err(e) => crate::raise_exc!(
            crate::exceptions::ExceptionType::ValueError,
            "Invalid format specifier: {}",
            e
        ),
    };

    let v = Value(value as u64);
    let formatted = if v.is_int() {
        match fmt::format_int(v.unwrap_int(), &format_spec) {
            Ok(s) => s,
            Err(e) => crate::raise_exc_string!(crate::exceptions::ExceptionType::ValueError, e),
        }
    } else if v.is_bool() {
        match fmt::format_bool(v.unwrap_bool(), &format_spec) {
            Ok(s) => s,
            Err(e) => crate::raise_exc_string!(crate::exceptions::ExceptionType::ValueError, e),
        }
    } else {
        let header = &(*value).header;
        let type_tag = header.type_tag;
        match type_tag {
            TypeTagKind::Float => {
                let float_obj = &*(value as *const FloatObj);
                match fmt::format_float(float_obj.value, &format_spec) {
                    Ok(s) => s,
                    Err(e) => {
                        crate::raise_exc_string!(crate::exceptions::ExceptionType::ValueError, e)
                    }
                }
            }
            TypeTagKind::Str => {
                let str_obj = &*(value as *const StrObj);
                let bytes = std::slice::from_raw_parts(str_obj.data.as_ptr(), str_obj.len);
                let s = std::str::from_utf8(bytes)
                    .unwrap_or_else(|_| raise_value_error("Invalid UTF-8 in string"));
                match fmt::format_str(s, &format_spec) {
                    Ok(s) => s,
                    Err(e) => {
                        crate::raise_exc_string!(crate::exceptions::ExceptionType::ValueError, e)
                    }
                }
            }
            _ => {
                let type_name = type_tag.type_name();
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "unsupported format string passed to {}.__format__",
                    type_name
                );
            }
        }
    };

    crate::string::rt_make_str(formatted.as_ptr(), formatted.len())
}

#[export_name = "rt_format"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_format_abi(value: Value, spec: Value) -> Value {
    Value::from_ptr(unsafe { rt_format(value.unwrap_ptr(), spec.unwrap_ptr()) })
}
