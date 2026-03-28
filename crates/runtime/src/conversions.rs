//! Type conversion operations for Python runtime

use crate::exceptions;
use crate::gc;
use crate::object::{
    BytesObj, DictObj, ListObj, Obj, ObjHeader, SetObj, StrObj, TupleObj, TypeTagKind,
};
use crate::string::rt_make_str;

/// Convert an integer to a string
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_int_to_str(value: i64) -> *mut Obj {
    use crate::object::{ObjHeader, StrObj, TypeTagKind};

    let s = value.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();

    let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

    unsafe {
        let str_obj = obj as *mut StrObj;
        (*str_obj).len = len;
        if len > 0 {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_obj).data.as_mut_ptr(), len);
        }
    }

    obj
}

/// Convert a float to a string
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_float_to_str(value: f64) -> *mut Obj {
    use crate::object::{ObjHeader, StrObj, TypeTagKind};

    let s = crate::utils::format_float_python(value);
    let bytes = s.as_bytes();
    let len = bytes.len();

    let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

    unsafe {
        let str_obj = obj as *mut StrObj;
        (*str_obj).len = len;
        if len > 0 {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_obj).data.as_mut_ptr(), len);
        }
    }

    obj
}

/// Convert a boolean to a string ("True" or "False")
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_bool_to_str(value: i8) -> *mut Obj {
    use crate::object::{ObjHeader, StrObj, TypeTagKind};

    let s = if value != 0 { "True" } else { "False" };
    let bytes = s.as_bytes();
    let len = bytes.len();

    let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

    unsafe {
        let str_obj = obj as *mut StrObj;
        (*str_obj).len = len;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_obj).data.as_mut_ptr(), len);
    }

    obj
}

/// Convert None to a string ("None")
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_none_to_str() -> *mut Obj {
    use crate::object::{ObjHeader, StrObj, TypeTagKind};

    let s = "None";
    let bytes = s.as_bytes();
    let len = bytes.len();

    let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

    unsafe {
        let str_obj = obj as *mut StrObj;
        (*str_obj).len = len;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_obj).data.as_mut_ptr(), len);
    }

    obj
}

/// Convert a string to an integer
/// Returns: i64 value
/// Raises: ValueError if string is not a valid integer
#[no_mangle]
pub extern "C" fn rt_str_to_int(str_obj: *mut Obj) -> i64 {
    use crate::object::StrObj;

    if str_obj.is_null() {
        let msg = b"int() argument must be a string, not None";
        unsafe {
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
            let msg = b"int() argument contains invalid UTF-8";
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
        let msg = b"float() argument must be a string, not None";
        unsafe {
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
            let msg = b"float() argument contains invalid UTF-8";
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }
}

/// Convert integer code point to character: chr(i) -> str
/// Returns: pointer to single-character string
/// Raises: ValueError if codepoint is out of range
#[no_mangle]
pub extern "C" fn rt_int_to_chr(codepoint: i64) -> *mut Obj {
    use crate::object::{ObjHeader, StrObj, TypeTagKind};

    if !(0..=0x10FFFF).contains(&codepoint) {
        let msg = b"chr() arg not in range(0x110000)";
        unsafe {
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }

    let ch = match char::from_u32(codepoint as u32) {
        Some(c) => c,
        None => {
            let msg = b"chr() arg not in valid Unicode range";
            unsafe {
                exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
            }
        }
    };

    let s = ch.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();

    let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

    unsafe {
        let str_obj = obj as *mut StrObj;
        (*str_obj).len = len;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_obj).data.as_mut_ptr(), len);
    }

    obj
}

/// Convert character to integer code point: ord(s) -> i64
/// Returns: Unicode code point as integer
/// Raises: ValueError if string is not exactly one character
#[no_mangle]
pub extern "C" fn rt_chr_to_int(str_obj: *mut Obj) -> i64 {
    use crate::object::StrObj;

    if str_obj.is_null() {
        let msg = b"ord() expected string, not None";
        unsafe {
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
                    let msg = b"ord() expected a character, but string is empty";
                    exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
            let msg = b"ord() argument contains invalid UTF-8";
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }
}

/// Convert any heap object to string with runtime type dispatch
/// Used for Union types where the actual type is determined at runtime
/// Returns: pointer to new allocated StrObj
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_to_str(obj: *mut Obj) -> *mut Obj {
    use crate::object::{BoolObj, FloatObj, IntObj, ObjHeader, StrObj, TypeTagKind};

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
                let len = bytes.len();

                let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
                let new_obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

                let str_obj = new_obj as *mut StrObj;
                (*str_obj).len = len;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*str_obj).data.as_mut_ptr(), len);

                new_obj
            }
        }
    }
}

/// Build a repr string for any object (used by str() for containers)
unsafe fn obj_to_repr_string(obj: *mut Obj) -> String {
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
            std::str::from_utf8(bytes)
                .unwrap_or("<invalid utf8>")
                .to_string()
        }
        TypeTagKind::None => "None".to_string(),
        TypeTagKind::List => {
            let list = obj as *mut ListObj;
            let len = (*list).len;
            let data = (*list).data;
            let elem_tag = (*list).elem_tag;
            let mut s = String::from("[");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let elem = *data.add(i);
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
unsafe fn maybe_raw_repr_string(s: &mut String, ptr: *mut Obj) {
    use std::fmt::Write;

    if crate::utils::is_heap_obj(ptr) {
        obj_repr_string(s, ptr);
    } else {
        let _ = write!(s, "{}", ptr as i64);
    }
}

/// Write repr of a single element to a string, based on elem_tag
unsafe fn elem_repr_string(s: &mut String, elem: *mut Obj, elem_tag: u8) {
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

/// Write repr of a boxed heap object to a string (strings get quotes)
unsafe fn obj_repr_string(s: &mut String, obj: *mut Obj) {
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
                repr_escape_into(s, text);
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

// ==================== Number formatting ====================

/// Convert integer to binary string (e.g., '0b1010')
#[no_mangle]
pub extern "C" fn rt_int_to_bin(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0b{:b}", n.unsigned_abs())
    } else {
        format!("0b{:b}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Convert integer to hexadecimal string (e.g., '0xff')
#[no_mangle]
pub extern "C" fn rt_int_to_hex(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0x{:x}", n.unsigned_abs())
    } else {
        format!("0x{:x}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Convert integer to octal string (e.g., '0o10')
#[no_mangle]
pub extern "C" fn rt_int_to_oct(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0o{:o}", n.unsigned_abs())
    } else {
        format!("0o{:o}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

// ==================== Format-specific number conversions ====================
// These functions produce strings WITHOUT prefixes (for format spec {:x}, {:o}, {:b})

/// Format integer as lowercase hex string without prefix (e.g., "ff")
#[no_mangle]
pub extern "C" fn rt_int_fmt_hex(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:x}", n)
    } else {
        format!("-{:x}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Format integer as uppercase hex string without prefix (e.g., "FF")
#[no_mangle]
pub extern "C" fn rt_int_fmt_hex_upper(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:X}", n)
    } else {
        format!("-{:X}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Format integer as octal string without prefix (e.g., "377")
#[no_mangle]
pub extern "C" fn rt_int_fmt_oct(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:o}", n)
    } else {
        format!("-{:o}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Format integer as binary string without prefix (e.g., "1010")
#[no_mangle]
pub extern "C" fn rt_int_fmt_bin(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:b}", n)
    } else {
        format!("-{:b}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

// ==================== Grouping format functions ====================

/// Insert grouping separators every 3 digits in the integer part of a number string.
fn insert_grouping(digits: &str, sep: char) -> String {
    let len = digits.len();
    if len <= 3 {
        return digits.to_string();
    }
    let mut result = String::with_capacity(len + len / 3);
    let first_group = len % 3;
    if first_group > 0 {
        result.push_str(&digits[..first_group]);
    }
    for (i, chunk) in digits.as_bytes()[first_group..].chunks(3).enumerate() {
        if i > 0 || first_group > 0 {
            result.push(sep);
        }
        for &b in chunk {
            result.push(b as char);
        }
    }
    result
}

/// Format integer with grouping separator: f"{1000000:,}" → "1,000,000"
#[no_mangle]
pub extern "C" fn rt_int_fmt_grouped(n: i64, sep: i64) -> *mut Obj {
    let sep_char = sep as u8 as char;
    let s = if n >= 0 {
        insert_grouping(&format!("{}", n), sep_char)
    } else {
        format!("-{}", insert_grouping(&format!("{}", -n), sep_char))
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Format float with precision and grouping separator: f"{1234.5:,.2f}" → "1,234.50"
#[no_mangle]
pub extern "C" fn rt_float_fmt_grouped(f: f64, precision: i64, sep: i64) -> *mut Obj {
    let sep_char = sep as u8 as char;
    let prec = precision as usize;
    let formatted = format!("{:.prec$}", f);
    let s = if let Some(dot_pos) = formatted.find('.') {
        let int_part = &formatted[..dot_pos];
        let frac_part = &formatted[dot_pos..];
        let (sign, digits) = if let Some(stripped) = int_part.strip_prefix('-') {
            ("-", stripped)
        } else {
            ("", int_part)
        };
        format!("{}{}{}", sign, insert_grouping(digits, sep_char), frac_part)
    } else {
        let (sign, digits) = if let Some(stripped) = formatted.strip_prefix('-') {
            ("-", stripped)
        } else {
            ("", formatted.as_str())
        };
        format!("{}{}", sign, insert_grouping(digits, sep_char))
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

// ==================== Repr functions ====================

/// repr(int) -> string
#[no_mangle]
pub extern "C" fn rt_repr_int(n: i64) -> *mut Obj {
    let s = format!("{}", n);
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(float) -> string
#[no_mangle]
pub extern "C" fn rt_repr_float(f: f64) -> *mut Obj {
    let s = crate::utils::format_float_python(f);
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(bool) -> string
#[no_mangle]
pub extern "C" fn rt_repr_bool(b: i8) -> *mut Obj {
    let s = if b != 0 { "True" } else { "False" };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(None) -> string
#[no_mangle]
pub extern "C" fn rt_repr_none() -> *mut Obj {
    let s = "None";
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(str) -> string (with quotes and proper escaping)
///
/// Matches CPython repr() behavior:
/// - ASCII printable (0x20-0x7E except \, '): literal
/// - `\n`, `\r`, `\t`, `\\`, `\'`: named escapes
/// - Control chars (0x00-0x1F, 0x7F-0x9F): `\xNN`
/// - Non-printable U+0100..U+FFFF: `\uXXXX`
/// - Non-printable U+10000+: `\UXXXXXXXX`
/// - All other printable Unicode: literal
#[no_mangle]
pub extern "C" fn rt_repr_str(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        let s = "''";
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;
        let data = (*src).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        let mut s = String::with_capacity(len + 2);
        s.push('\'');
        if let Ok(text) = std::str::from_utf8(bytes) {
            repr_escape_into(&mut s, text);
        }
        s.push('\'');

        let result_bytes = s.as_bytes();
        rt_make_str(result_bytes.as_ptr(), result_bytes.len())
    }
}

/// repr(list) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_list(list_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_repr_string(list_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(tuple) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_tuple(tuple_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_repr_string(tuple_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(dict) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_dict(dict_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_repr_string(dict_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(set) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_set(set_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_repr_string(set_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// repr(bytes) -> string
#[no_mangle]
pub extern "C" fn rt_repr_bytes(bytes_obj: *mut Obj) -> *mut Obj {
    if bytes_obj.is_null() {
        let s = "b''";
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }

    unsafe {
        let src = bytes_obj as *mut BytesObj;
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

        let result_bytes = s.as_bytes();
        rt_make_str(result_bytes.as_ptr(), result_bytes.len())
    }
}

/// repr(obj) - generic runtime dispatch
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_repr_obj(obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_repr_string(obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

// ==================== ASCII conversion functions ====================
// ascii() is like repr() but escapes non-ASCII characters using \xNN, \uNNNN, or \UNNNNNNNN

/// ascii(str) -> string (with quotes and escaped non-ASCII)
#[no_mangle]
pub extern "C" fn rt_ascii_str(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        let s = "''";
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;
        let data = (*src).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        let mut s = String::with_capacity(len + 2);
        s.push('\'');
        if let Ok(text) = std::str::from_utf8(bytes) {
            // Escape special characters and non-ASCII
            for c in text.chars() {
                match c {
                    '\n' => s.push_str("\\n"),
                    '\r' => s.push_str("\\r"),
                    '\t' => s.push_str("\\t"),
                    '\\' => s.push_str("\\\\"),
                    '\'' => s.push_str("\\'"),
                    _ => {
                        let cp = c as u32;
                        if cp < 128 {
                            s.push(c);
                        } else if cp <= 0xFF {
                            s.push_str(&format!("\\x{:02x}", cp));
                        } else if cp <= 0xFFFF {
                            s.push_str(&format!("\\u{:04x}", cp));
                        } else {
                            s.push_str(&format!("\\U{:08x}", cp));
                        }
                    }
                }
            }
        }
        s.push('\'');

        let result_bytes = s.as_bytes();
        rt_make_str(result_bytes.as_ptr(), result_bytes.len())
    }
}

/// Helper to convert an object to its ASCII representation string
unsafe fn obj_to_ascii_string(obj: *mut Obj) -> String {
    if obj.is_null() {
        return "None".to_string();
    }

    let header = obj as *mut ObjHeader;
    match (*header).type_tag {
        TypeTagKind::Str => {
            // Get the repr form with quotes
            let src = obj as *mut StrObj;
            let len = (*src).len;
            let data = (*src).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);

            let mut s = String::with_capacity(len + 2);
            s.push('\'');
            if let Ok(text) = std::str::from_utf8(bytes) {
                for c in text.chars() {
                    match c {
                        '\n' => s.push_str("\\n"),
                        '\r' => s.push_str("\\r"),
                        '\t' => s.push_str("\\t"),
                        '\\' => s.push_str("\\\\"),
                        '\'' => s.push_str("\\'"),
                        _ => {
                            let cp = c as u32;
                            if cp < 128 {
                                s.push(c);
                            } else if cp <= 0xFF {
                                s.push_str(&format!("\\x{:02x}", cp));
                            } else if cp <= 0xFFFF {
                                s.push_str(&format!("\\u{:04x}", cp));
                            } else {
                                s.push_str(&format!("\\U{:08x}", cp));
                            }
                        }
                    }
                }
            }
            s.push('\'');
            s
        }
        TypeTagKind::List => {
            let list = obj as *mut ListObj;
            let len = (*list).len;
            let data = (*list).data;

            let mut s = String::from("[");
            for i in 0..len {
                if i > 0 {
                    s.push_str(", ");
                }
                let elem = *data.add(i);
                s.push_str(&obj_to_ascii_string(elem));
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
                let elem = *data.add(i);
                s.push_str(&obj_to_ascii_string(elem));
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
                    s.push_str(&obj_to_ascii_string(key));
                    s.push_str(": ");
                    s.push_str(&obj_to_ascii_string((*entry).value));
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
            const SET_TOMBSTONE: *mut Obj = std::ptr::dangling_mut::<Obj>();

            let mut s = String::from("{");
            let mut first = true;
            for i in 0..capacity {
                let entry = entries.add(i);
                let elem = (*entry).elem;
                if !elem.is_null() && elem != SET_TOMBSTONE {
                    if !first {
                        s.push_str(", ");
                    }
                    first = false;
                    s.push_str(&obj_to_ascii_string(elem));
                }
            }
            s.push('}');
            s
        }
        // For non-string primitive types, delegate to repr (they don't contain non-ASCII)
        _ => obj_to_repr_string(obj),
    }
}

/// ascii(list) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_ascii_list(list_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_ascii_string(list_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// ascii(tuple) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_ascii_tuple(tuple_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_ascii_string(tuple_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// ascii(dict) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_ascii_dict(dict_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_ascii_string(dict_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// ascii(set) -> string
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_ascii_set(set_obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_ascii_string(set_obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// ascii(obj) - generic runtime dispatch
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_ascii_obj(obj: *mut Obj) -> *mut Obj {
    let s = unsafe { obj_to_ascii_string(obj) };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Get type name for type() builtin
/// Uses type_class() from core-defs as the single source of truth.
#[no_mangle]
pub extern "C" fn rt_type_name(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        // Null pointer represents None
        let s = TypeTagKind::None.type_class();
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }

    unsafe {
        // Get type_class directly from the type tag - single source of truth in core-defs
        let type_class_str = (*obj).header.type_tag.type_class();
        let bytes = type_class_str.as_bytes();
        rt_make_str(bytes.as_ptr(), bytes.len())
    }
}

/// Extract type name from type string for __name__ attribute access
/// Extracts "int" from "<class 'int'>" for type(x).__name__
/// Input: string object like "<class 'int'>"
/// Output: string object like "int"
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_type_name_extract(type_str: *mut Obj) -> *mut Obj {
    let full_str = unsafe { crate::utils::str_obj_to_rust_string(type_str) };

    // Parse "<class 'typename'>" format to extract typename
    // Also handles multi-part names like "time.struct_time"
    if let Some(start) = full_str.find("'") {
        if let Some(end) = full_str.rfind("'") {
            if start < end {
                let name = &full_str[start + 1..end];
                let bytes = name.as_bytes();
                return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
            }
        }
    }

    // Fallback: return the full string if parsing fails
    let bytes = full_str.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}

/// Default repr for objects without __str__ or __repr__
/// Returns: pointer to string like "<object at 0x...>"
#[no_mangle]
pub extern "C" fn rt_obj_default_repr(obj: *mut Obj) -> *mut Obj {
    unsafe {
        let s = format!("<object at {:p}>", obj);
        let bytes = s.as_bytes();
        rt_make_str(bytes.as_ptr(), bytes.len())
    }
}

/// Convert string to integer with given base
/// s: pointer to StrObj
/// base: numeric base (2, 8, 10, or 16)
/// Returns: integer value
#[no_mangle]
pub extern "C" fn rt_str_to_int_with_base(s: *mut Obj, base: i64) -> i64 {
    use crate::object::StrObj;

    if s.is_null() {
        unsafe {
            let msg = b"invalid literal for int()";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        if let Ok(s_str) = std::str::from_utf8(bytes) {
            let trimmed = s_str.trim();

            // Handle prefixes for auto-detection when base is 0
            let (actual_base, trimmed_str) = if base == 0 {
                if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                    (16, &trimmed[2..])
                } else if trimmed.starts_with("0b") || trimmed.starts_with("0B") {
                    (2, &trimmed[2..])
                } else if trimmed.starts_with("0o") || trimmed.starts_with("0O") {
                    (8, &trimmed[2..])
                } else {
                    (10, trimmed)
                }
            } else {
                // Only strip a prefix when it matches the requested base.
                // CPython raises ValueError for mismatches such as int("0b10", 16).
                let trimmed_str = match base {
                    16 if trimmed.starts_with("0x") || trimmed.starts_with("0X") => &trimmed[2..],
                    2 if trimmed.starts_with("0b") || trimmed.starts_with("0B") => &trimmed[2..],
                    8 if trimmed.starts_with("0o") || trimmed.starts_with("0O") => &trimmed[2..],
                    _ => trimmed,
                };
                (base as u32, trimmed_str)
            };

            if !(2..=36).contains(&actual_base) {
                let msg = b"int() base must be >= 2 and <= 36, or 0";
                crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
            }

            match i64::from_str_radix(trimmed_str, actual_base) {
                Ok(val) => val,
                Err(_) => {
                    let msg = b"invalid literal for int() with base";
                    crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
                }
            }
        } else {
            let msg = b"invalid literal for int()";
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }
}
