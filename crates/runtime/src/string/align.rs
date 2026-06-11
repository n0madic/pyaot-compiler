//! Alignment operations: center, ljust, rjust, zfill
//!
//! Widths are measured in Unicode codepoints (CPython semantics), and the
//! fill character may be multi-byte. Source bytes are copied into Rust
//! `String`s BEFORE any allocation, so no GC rooting is needed.

use crate::object::{Obj, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

use super::core::rt_make_str;

/// Decode a StrObj's bytes to a Rust `String` (lossy on malformed UTF-8).
unsafe fn str_obj_to_string(str_obj: *mut Obj) -> String {
    let src = str_obj as *mut StrObj;
    debug_assert!((*src).header.type_tag == TypeTagKind::Str);
    let bytes = std::slice::from_raw_parts((*src).data.as_ptr(), (*src).len);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Extract the fill character (first codepoint of `fillchar`, default ' ').
unsafe fn fill_char(fillchar: *mut Obj) -> char {
    if fillchar.is_null() {
        return ' ';
    }
    str_obj_to_string(fillchar).chars().next().unwrap_or(' ')
}

/// Center string with fill character
/// Returns: new string
pub fn rt_str_center(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }
    unsafe {
        let s = str_obj_to_string(str_obj);
        let chars = s.chars().count() as i64;
        if chars >= width {
            return str_obj;
        }
        let fill = fill_char(fillchar);
        let marg = width - chars;
        // CPython's stringlib pad: left = marg/2 + (marg & width & 1).
        let left = marg / 2 + (marg & width & 1);
        let mut result = String::with_capacity(s.len() + marg as usize * fill.len_utf8());
        for _ in 0..left {
            result.push(fill);
        }
        result.push_str(&s);
        for _ in 0..(marg - left) {
            result.push(fill);
        }
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_center"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_center_abi(str_obj: Value, width: i64, fillchar: Value) -> Value {
    Value::from_ptr(rt_str_center(
        str_obj.unwrap_ptr(),
        width,
        fillchar.unwrap_ptr(),
    ))
}

/// Left justify string with fill character
/// Returns: new string
pub fn rt_str_ljust(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }
    unsafe {
        let s = str_obj_to_string(str_obj);
        let chars = s.chars().count() as i64;
        if chars >= width {
            return str_obj;
        }
        let fill = fill_char(fillchar);
        let mut result = s;
        for _ in 0..(width - chars) {
            result.push(fill);
        }
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_ljust"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_ljust_abi(str_obj: Value, width: i64, fillchar: Value) -> Value {
    Value::from_ptr(rt_str_ljust(
        str_obj.unwrap_ptr(),
        width,
        fillchar.unwrap_ptr(),
    ))
}

/// Right justify string with fill character
/// Returns: new string
pub fn rt_str_rjust(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }
    unsafe {
        let s = str_obj_to_string(str_obj);
        let chars = s.chars().count() as i64;
        if chars >= width {
            return str_obj;
        }
        let fill = fill_char(fillchar);
        let mut result = String::with_capacity(s.len());
        for _ in 0..(width - chars) {
            result.push(fill);
        }
        result.push_str(&s);
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_rjust"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_rjust_abi(str_obj: Value, width: i64, fillchar: Value) -> Value {
    Value::from_ptr(rt_str_rjust(
        str_obj.unwrap_ptr(),
        width,
        fillchar.unwrap_ptr(),
    ))
}

/// Zero-fill string (left pad with zeros, preserving sign)
/// Returns: new string
pub fn rt_str_zfill(str_obj: *mut Obj, width: i64) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }
    unsafe {
        let s = str_obj_to_string(str_obj);
        let chars = s.chars().count() as i64;
        if chars >= width {
            return str_obj;
        }
        let padding = (width - chars) as usize;
        let mut result = String::with_capacity(s.len() + padding);
        let mut rest: &str = &s;
        if let Some(first) = s.chars().next() {
            if first == '+' || first == '-' {
                result.push(first);
                rest = &s[1..];
            }
        }
        for _ in 0..padding {
            result.push('0');
        }
        result.push_str(rest);
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_zfill"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_zfill_abi(str_obj: Value, width: i64) -> Value {
    Value::from_ptr(rt_str_zfill(str_obj.unwrap_ptr(), width))
}
