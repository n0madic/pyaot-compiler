//! Conversions to string: rt_*_to_str variants and related helpers

use crate::object::{FloatObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Convert an integer to a string
/// Returns: pointer to new allocated StrObj
pub fn rt_int_to_str(value: i64) -> *mut Obj {
    let s = value.to_string();
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_str_abi(value: i64) -> Value {
    Value::from_ptr(rt_int_to_str(value))
}


/// Convert a float to a string
/// Returns: pointer to new allocated StrObj
pub fn rt_float_to_str(value: f64) -> *mut Obj {
    let s = crate::utils::format_float_python(value);
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_float_to_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_float_to_str_abi(value: f64) -> Value {
    Value::from_ptr(rt_float_to_str(value))
}


/// Convert a boolean to a string ("True" or "False")
/// Returns: pointer to new allocated StrObj
pub fn rt_bool_to_str(value: i8) -> *mut Obj {
    let s = if value != 0 { "True" } else { "False" };
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_bool_to_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bool_to_str_abi(value: i8) -> Value {
    Value::from_ptr(rt_bool_to_str(value))
}


/// Convert None to a string ("None")
/// Returns: pointer to new allocated StrObj
pub fn rt_none_to_str() -> *mut Obj {
    let s = "None";
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_none_to_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_none_to_str_abi() -> Value {
    Value::from_ptr(rt_none_to_str())
}


/// Convert a string to an integer
/// Returns: i64 value
/// Raises: ValueError if string is not a valid integer
pub fn rt_str_to_int(str_obj: *mut Obj) -> i64 {
    use crate::object::StrObj;

    if str_obj.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "int() argument must be a string, not None"
            );
        }
    }

    unsafe {
        let str_obj = str_obj as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        if let Ok(s) = std::str::from_utf8(bytes) {
            let trimmed = s.trim_matches(|c: char| c.is_whitespace());
            match trimmed.parse::<i64>() {
                Ok(val) => val,
                Err(_) => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "invalid literal for int() with base 10: '{}'",
                        s.trim()
                    );
                }
            }
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "int() argument contains invalid UTF-8"
            );
        }
    }
}
#[export_name = "rt_str_to_int"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_to_int_abi(str_obj: Value) -> i64 {
    rt_str_to_int(str_obj.unwrap_ptr())
}


/// Convert a string to a float
/// Returns: f64 value
/// Raises: ValueError if string is not a valid float
pub fn rt_str_to_float(str_obj: *mut Obj) -> f64 {
    use crate::object::StrObj;

    if str_obj.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "float() argument must be a string, not None"
            );
        }
    }

    unsafe {
        let str_obj = str_obj as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        if let Ok(s) = std::str::from_utf8(bytes) {
            let trimmed = s.trim_matches(|c: char| c.is_whitespace());
            match trimmed.parse::<f64>() {
                Ok(val) => val,
                Err(_) => {
                    crate::raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "could not convert string to float: '{}'",
                        s.trim()
                    );
                }
            }
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "float() argument contains invalid UTF-8"
            );
        }
    }
}
#[export_name = "rt_str_to_float"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_to_float_abi(str_obj: Value) -> f64 {
    rt_str_to_float(str_obj.unwrap_ptr())
}


/// Convert integer code point to character: chr(i) -> str
/// Returns: pointer to single-character string
/// Raises: ValueError if codepoint is out of range
pub fn rt_int_to_chr(codepoint: i64) -> *mut Obj {
    if !(0..=0x10FFFF).contains(&codepoint) {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "chr() arg not in range(0x110000)"
            );
        }
    }

    let ch = match char::from_u32(codepoint as u32) {
        Some(c) => c,
        None => unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "chr() arg not in valid Unicode range"
            );
        },
    };

    let s = ch.to_string();
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_chr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_chr_abi(codepoint: i64) -> Value {
    Value::from_ptr(rt_int_to_chr(codepoint))
}


/// Convert character to integer code point: ord(s) -> i64
/// Returns: Unicode code point as integer
/// Raises: ValueError if string is not exactly one character
pub fn rt_chr_to_int(str_obj: *mut Obj) -> i64 {
    use crate::object::StrObj;

    if str_obj.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "ord() expected string, not None"
            );
        }
    }

    unsafe {
        let str_obj = str_obj as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        if let Ok(s) = std::str::from_utf8(bytes) {
            let mut chars = s.chars();
            let ch = match chars.next() {
                Some(c) => c,
                None => {
                    raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "ord() expected a character, but string is empty"
                    );
                }
            };

            if chars.next().is_some() {
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "ord() expected a character, but string of length {} found",
                    s.chars().count()
                );
            }

            ch as i64
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "ord() argument contains invalid UTF-8"
            );
        }
    }
}
#[export_name = "rt_chr_to_int"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_chr_to_int_abi(str_obj: Value) -> i64 {
    rt_chr_to_int(str_obj.unwrap_ptr())
}


/// Convert any heap object to string with runtime type dispatch
/// Used for Union types where the actual type is determined at runtime
/// Returns: pointer to new allocated StrObj
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_obj_to_str(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return rt_none_to_str();
    }

    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        return rt_int_to_str(v.unwrap_int());
    }
    if v.is_bool() {
        return rt_bool_to_str(if v.unwrap_bool() { 1 } else { 0 });
    }
    if v.is_none() {
        return rt_none_to_str();
    }

    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Float => {
                let value = (*(obj as *mut FloatObj)).value;
                rt_float_to_str(value)
            }
            TypeTagKind::Str => obj, // already a string
            TypeTagKind::None => rt_none_to_str(),
            _ => {
                // For containers and other types, build repr string
                let s = obj_to_repr_string(obj);
                let bytes = s.as_bytes();
                crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len())
            }
        }
    }
}
#[export_name = "rt_obj_to_str"]
pub extern "C" fn rt_obj_to_str_abi(obj: Value) -> Value {
    Value::from_ptr(rt_obj_to_str(obj.unwrap_ptr()))
}


/// Build a repr string for any object (used by str() for containers)
pub(super) unsafe fn obj_to_repr_string(obj: *mut Obj) -> String {
    use crate::object::*;

    if obj.is_null() {
        return "None".to_string();
    }

    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        return format!("{}", v.unwrap_int());
    }
    if v.is_bool() {
        return if v.unwrap_bool() {
            "True".to_string()
        } else {
            "False".to_string()
        };
    }
    if v.is_none() {
        return "None".to_string();
    }

    match (*obj).type_tag() {
        TypeTagKind::Float => crate::utils::format_float_python((*(obj as *mut FloatObj)).value),
        TypeTagKind::Str => {
            let str_obj = obj as *mut StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);
            let mut s = String::with_capacity(len + 2);
            s.push('\'');
            if let Ok(text) = std::str::from_utf8(bytes) {
                super::repr::repr_escape_into(&mut s, text);
            }
            s.push('\'');
            s
        }
        TypeTagKind::Bytes => {
            let src = obj as *mut BytesObj;
            let len = (*src).len;
            let data = (*src).data.as_ptr();
            let mut s = String::with_capacity(len + 3);
            s.push_str("b'");
            for i in 0..len {
                let b = *data.add(i);
                if (0x20..0x7f).contains(&b) && b != b'\'' && b != b'\\' {
                    s.push(b as char);
                } else {
                    s.push_str(&format!("\\x{:02x}", b));
                }
            }
            s.push('\'');
            s
        }
        TypeTagKind::None => "None".to_string(),
        TypeTagKind::List => {
            let list = obj as *mut ListObj;
            let len = (*list).len;
            let mut s = String::from("[");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let val = *(*list).data.add(i);
                elem_repr_string(&mut s, val.0 as *mut Obj);
            }
            s.push(']');
            s
        }
        TypeTagKind::Tuple => {
            let tuple = obj as *mut TupleObj;
            let len = (*tuple).len;
            let data = (*tuple).data.as_ptr();
            let mut s = String::from("(");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let val = *data.add(i);
                elem_repr_string(&mut s, val.0 as *mut Obj);
            }
            if len == 1 {
                s.push(',');
            }
            s.push(')');
            s
        }
        TypeTagKind::Dict => {
            let dict = obj as *mut DictObj;
            let entries_len = (*dict).entries_len;
            let entries = (*dict).entries;
            let mut s = String::from("{");
            let mut first = true;
            for i in 0..entries_len {
                let entry = entries.add(i);
                let key = (*entry).key;
                if key.0 != 0 {
                    if !first {
                        s.push_str(", ");
                    }
                    first = false;
                    obj_repr_string(&mut s, key.0 as *mut Obj);
                    s.push_str(": ");
                    maybe_raw_repr_string(&mut s, (*entry).value.0 as *mut Obj);
                }
            }
            s.push('}');
            s
        }
        TypeTagKind::Set => {
            let set = obj as *mut SetObj;
            let len = (*set).len;
            if len == 0 {
                return "set()".to_string();
            }
            let capacity = (*set).capacity;
            let entries = (*set).entries;
            let mut s = String::from("{");
            let mut first = true;
            for i in 0..capacity {
                let entry = entries.add(i);
                let elem = (*entry).elem;
                if elem.0 != 0 && elem != crate::object::TOMBSTONE {
                    if !first {
                        s.push_str(", ");
                    }
                    first = false;
                    obj_repr_string(&mut s, elem.0 as *mut Obj);
                }
            }
            s.push('}');
            s
        }
        _ => format!("<object at {:p}>", obj),
    }
}

/// Write repr of a value that may be a heap object or Value-tagged primitive
pub(super) unsafe fn maybe_raw_repr_string(s: &mut String, ptr: *mut Obj) {
    obj_repr_string(s, ptr);
}

/// Write repr of a single element to a string, dispatching on Value tag
pub(super) unsafe fn elem_repr_string(s: &mut String, elem: *mut Obj) {
    obj_repr_string(s, elem);
}

/// Write repr of a boxed heap object to a string (strings get quotes)
pub(super) unsafe fn obj_repr_string(s: &mut String, obj: *mut Obj) {
    use crate::object::*;
    use std::fmt::Write;

    if obj.is_null() {
        s.push_str("None");
        return;
    }
    // Check Value-tagged primitives before heap pointer dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        let _ = write!(s, "{}", v.unwrap_int());
        return;
    }
    if v.is_bool() {
        s.push_str(if v.unwrap_bool() { "True" } else { "False" });
        return;
    }
    if v.is_none() {
        s.push_str("None");
        return;
    }
    match (*obj).type_tag() {
        TypeTagKind::Float => {
            s.push_str(&crate::utils::format_float_python(
                (*(obj as *mut FloatObj)).value,
            ));
        }
        TypeTagKind::Str => {
            let str_obj = obj as *mut StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);
            s.push('\'');
            if let Ok(text) = std::str::from_utf8(bytes) {
                super::repr::repr_escape_into(s, text);
            }
            s.push('\'');
        }
        TypeTagKind::None => s.push_str("None"),
        _ => {
            // Recurse into containers
            let inner = obj_to_repr_string(obj);
            s.push_str(&inner);
        }
    }
}
