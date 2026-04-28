//! repr() conversion functions for Python runtime

use crate::object::Obj;
use crate::string::rt_make_str;
use pyaot_core_defs::Value;

/// Escape a string for repr() output, matching CPython behavior.
///
/// - `\n`, `\r`, `\t`, `\\`, `\'`: named escapes
/// - Control chars (U+0000-U+001F, U+007F-U+009F): `\xNN`
/// - Non-printable U+0100..U+FFFF: `\uXXXX`
/// - Non-printable U+10000+: `\UXXXXXXXX`
/// - All other printable Unicode: literal
pub(crate) fn repr_escape_into(s: &mut String, text: &str) {
    use std::fmt::Write;
    for ch in text.chars() {
        match ch {
            '\\' => s.push_str("\\\\"),
            '\'' => s.push_str("\\'"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c => {
                let cp = c as u32;
                if cp < 0x20 || (0x7F..=0x9F).contains(&cp) {
                    // ASCII/Latin-1 control characters: \xNN
                    let _ = write!(s, "\\x{:02x}", cp);
                } else if !c.is_control() && (cp < 0x80 || is_printable_unicode(c)) {
                    // Printable character: literal
                    s.push(c);
                } else if cp <= 0xFF {
                    let _ = write!(s, "\\x{:02x}", cp);
                } else if cp <= 0xFFFF {
                    let _ = write!(s, "\\u{:04x}", cp);
                } else {
                    let _ = write!(s, "\\U{:08x}", cp);
                }
            }
        }
    }
}

/// Check if a Unicode character is printable (matches CPython's Py_UNICODE_ISPRINTABLE).
/// Characters in categories Cc, Cf, Cs, Co, Cn, Zl, Zp are non-printable.
/// Space (U+0020) is printable; other Zs are non-printable.
fn is_printable_unicode(c: char) -> bool {
    // Fast path for common ASCII
    let cp = c as u32;
    if cp < 0x80 {
        return (0x20..0x7F).contains(&cp);
    }
    // Non-printable ranges from Unicode (simplified — covers the common cases)
    // Cf (format) characters
    if cp == 0xAD {
        return false;
    } // soft hyphen
    if (0x600..=0x605).contains(&cp) {
        return false;
    }
    if cp == 0x61C || cp == 0x6DD || cp == 0x70F {
        return false;
    }
    if (0x200B..=0x200F).contains(&cp) {
        return false;
    } // zero-width spaces, LRM, RLM
    if (0x202A..=0x202E).contains(&cp) {
        return false;
    } // bidi embeddings
    if (0x2060..=0x2064).contains(&cp) {
        return false;
    } // word joiner etc.
    if (0x2066..=0x2069).contains(&cp) {
        return false;
    } // bidi isolates
    if cp == 0xFEFF {
        return false;
    } // BOM
    if (0xFFF9..=0xFFFB).contains(&cp) {
        return false;
    }
    // Zl, Zp (line/paragraph separator)
    if cp == 0x2028 || cp == 0x2029 {
        return false;
    }
    // Cs (surrogates) — shouldn't appear in valid Rust chars
    if (0xD800..=0xDFFF).contains(&cp) {
        return false;
    }
    // Cn: unassigned (check some major unassigned blocks)
    // Co: private use
    if (0xE000..=0xF8FF).contains(&cp) {
        return true;
    } // PUA is "printable" in CPython
    if (0xF0000..=0xFFFFD).contains(&cp) {
        return true;
    } // Supplementary PUA-A
    if (0x100000..=0x10FFFD).contains(&cp) {
        return true;
    } // Supplementary PUA-B
      // Default: assume printable for assigned characters
    true
}

/// repr(int) -> string
pub fn rt_repr_int(n: i64) -> *mut Obj {
    let s = format!("{}", n);
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_repr_int"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_int_abi(n: i64) -> Value {
    Value::from_ptr(rt_repr_int(n))
}


/// repr(float) -> string
pub fn rt_repr_float(f: f64) -> *mut Obj {
    let s = crate::utils::format_float_python(f);
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_repr_float"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_float_abi(f: f64) -> Value {
    Value::from_ptr(rt_repr_float(f))
}


/// repr(bool) -> string
pub fn rt_repr_bool(b: i8) -> *mut Obj {
    let s = if b != 0 { "True" } else { "False" };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_repr_bool"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_bool_abi(b: i8) -> Value {
    Value::from_ptr(rt_repr_bool(b))
}


/// repr(None) -> string
pub fn rt_repr_none() -> *mut Obj {
    let s = "None";
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_repr_none"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_none_abi() -> Value {
    Value::from_ptr(rt_repr_none())
}


/// repr() for collections (list, tuple, dict, set), str, bytes, and generic objects — runtime type-dispatched
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_repr_collection(obj: *mut Obj) -> *mut Obj {
    let s = unsafe { super::to_str::obj_to_repr_string(obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_repr_collection"]
pub extern "C" fn rt_repr_collection_abi(obj: Value) -> Value {
    Value::from_ptr(rt_repr_collection(obj.unwrap_ptr()))
}

