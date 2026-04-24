//! Conversions to string: rt_*_to_str variants and related helpers

use crate::object::{BoolObj, FloatObj, IntObj, Obj, TypeTagKind};

/// Convert an integer to a string
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_int_to_str(value: i64) -> *mut Obj {
    let s = value.to_string();
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}

/// Convert a float to a string
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_float_to_str(value: f64) -> *mut Obj {
    let s = crate::utils::format_float_python(value);
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}

/// Convert a boolean to a string ("True" or "False")
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_bool_to_str(value: i8) -> *mut Obj {
    let s = if value != 0 { "True" } else { "False" };
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}

/// Convert None to a string ("None")
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_none_to_str() -> *mut Obj {
    let s = "None";
    let bytes = s.as_bytes();
    unsafe { crate::string::rt_make_str_impl(bytes.as_ptr(), bytes.len()) }
}

/// Convert a string to an integer
/// Returns: i64 value
/// Raises: ValueError if string is not a valid integer
#[no_mangle]
pub extern "C" fn rt_str_to_int(str_obj: *mut Obj) -> i64 {
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

/// Convert a string to a float
/// Returns: f64 value
/// Raises: ValueError if string is not a valid float
#[no_mangle]
pub extern "C" fn rt_str_to_float(str_obj: *mut Obj) -> f64 {
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

/// Convert integer code point to character: chr(i) -> str
/// Returns: pointer to single-character string
/// Raises: ValueError if codepoint is out of range
#[no_mangle]
pub extern "C" fn rt_int_to_chr(codepoint: i64) -> *mut Obj {
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

/// Convert character to integer code point: ord(s) -> i64
/// Returns: Unicode code point as integer
/// Raises: ValueError if string is not exactly one character
#[no_mangle]
pub extern "C" fn rt_chr_to_int(str_obj: *mut Obj) -> i64 {
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

/// Convert any heap object to string with runtime type dispatch
/// Used for Union types where the actual type is determined at runtime
/// Returns: pointer to new allocated StrObj
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_to_str(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return rt_none_to_str();
    }

    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Int => {
                let value = (*(obj as *mut IntObj)).value;
                rt_int_to_str(value)
            }
            TypeTagKind::Float => {
                let value = (*(obj as *mut FloatObj)).value;
                rt_float_to_str(value)
            }
            TypeTagKind::Bool => {
                let value = (*(obj as *mut BoolObj)).value;
                rt_bool_to_str(if value { 1 } else { 0 })
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

/// Build a repr string for any object (used by str() for containers)
pub(super) unsafe fn obj_to_repr_string(obj: *mut Obj) -> String {
    use crate::object::*;

    if obj.is_null() {
        return "None".to_string();
    }

    match (*obj).type_tag() {
        TypeTagKind::Int => format!("{}", (*(obj as *mut IntObj)).value),
        TypeTagKind::Float => crate::utils::format_float_python((*(obj as *mut FloatObj)).value),
        TypeTagKind::Bool => {
            if (*(obj as *mut BoolObj)).value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
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
            let elem_tag = (*list).elem_tag;
            let mut s = String::from("[");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let elem = crate::list::list_slot_raw(list, i);
                elem_repr_string(&mut s, elem, elem_tag);
            }
            s.push(']');
            s
        }
        TypeTagKind::Tuple => {
            let tuple = obj as *mut TupleObj;
            let len = (*tuple).len;
            let data = (*tuple).data.as_ptr();
            let elem_tag = (*tuple).elem_tag;
            let mut s = String::from("(");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let elem = *data.add(i);
                elem_repr_string(&mut s, elem, elem_tag);
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
                if !key.is_null() {
                    if !first {
                        s.push_str(", ");
                    }
                    first = false;
                    obj_repr_string(&mut s, key);
                    s.push_str(": ");
                    maybe_raw_repr_string(&mut s, (*entry).value);
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
            const TOMBSTONE: *mut Obj = std::ptr::dangling_mut::<Obj>();
            let mut s = String::from("{");
            let mut first = true;
            for i in 0..capacity {
                let entry = entries.add(i);
                let elem = (*entry).elem;
                if !elem.is_null() && elem != TOMBSTONE {
                    if !first {
                        s.push_str(", ");
                    }
                    first = false;
                    obj_repr_string(&mut s, elem);
                }
            }
            s.push('}');
            s
        }
        _ => format!("<object at {:p}>", obj),
    }
}

/// Write repr of a value that may be a heap object or raw primitive
pub(super) unsafe fn maybe_raw_repr_string(s: &mut String, ptr: *mut Obj) {
    use std::fmt::Write;

    if crate::utils::is_heap_obj(ptr) {
        obj_repr_string(s, ptr);
    } else {
        let _ = write!(s, "{}", ptr as i64);
    }
}

/// Write repr of a single element to a string, based on elem_tag
pub(super) unsafe fn elem_repr_string(s: &mut String, elem: *mut Obj, elem_tag: u8) {
    use crate::object::*;
    use std::fmt::Write;

    match elem_tag {
        ELEM_RAW_INT => {
            let _ = write!(s, "{}", elem as i64);
        }
        ELEM_RAW_BOOL => {
            let val = elem as i64;
            s.push_str(if val != 0 { "True" } else { "False" });
        }
        _ => {
            obj_repr_string(s, elem);
        }
    }
}

/// Write repr of a boxed heap object to a string (strings get quotes)
pub(super) unsafe fn obj_repr_string(s: &mut String, obj: *mut Obj) {
    use crate::object::*;
    use std::fmt::Write;

    if obj.is_null() {
        s.push_str("None");
        return;
    }
    match (*obj).type_tag() {
        TypeTagKind::Int => {
            let _ = write!(s, "{}", (*(obj as *mut IntObj)).value);
        }
        TypeTagKind::Float => {
            s.push_str(&crate::utils::format_float_python(
                (*(obj as *mut FloatObj)).value,
            ));
        }
        TypeTagKind::Bool => {
            s.push_str(if (*(obj as *mut BoolObj)).value {
                "True"
            } else {
                "False"
            });
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
