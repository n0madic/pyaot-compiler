//! Type cast and numeric formatting conversions for Python runtime

use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};
use crate::string::rt_make_str;
use pyaot_core_defs::Value;

// ==================== Number formatting ====================

/// Convert integer to binary string (e.g., '0b1010')
pub fn rt_int_to_bin(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0b{:b}", n.unsigned_abs())
    } else {
        format!("0b{:b}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_bin"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_bin_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_to_bin(n))
}

/// Convert integer to hexadecimal string (e.g., '0xff')
pub fn rt_int_to_hex(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0x{:x}", n.unsigned_abs())
    } else {
        format!("0x{:x}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_hex"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_hex_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_to_hex(n))
}

/// Convert integer to octal string (e.g., '0o10')
pub fn rt_int_to_oct(n: i64) -> *mut Obj {
    let s = if n < 0 {
        format!("-0o{:o}", n.unsigned_abs())
    } else {
        format!("0o{:o}", n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_to_oct"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_to_oct_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_to_oct(n))
}

// ==================== Format-specific number conversions ====================
// These functions produce strings WITHOUT prefixes (for format spec {:x}, {:o}, {:b})

/// Format integer as lowercase hex string without prefix (e.g., "ff")
pub fn rt_int_fmt_hex(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:x}", n)
    } else {
        format!("-{:x}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_fmt_hex"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_fmt_hex_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_fmt_hex(n))
}

/// Format integer as uppercase hex string without prefix (e.g., "FF")
pub fn rt_int_fmt_hex_upper(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:X}", n)
    } else {
        format!("-{:X}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_fmt_hex_upper"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_fmt_hex_upper_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_fmt_hex_upper(n))
}

/// Format integer as octal string without prefix (e.g., "377")
pub fn rt_int_fmt_oct(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:o}", n)
    } else {
        format!("-{:o}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_fmt_oct"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_fmt_oct_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_fmt_oct(n))
}

/// Format integer as binary string without prefix (e.g., "1010")
pub fn rt_int_fmt_bin(n: i64) -> *mut Obj {
    let s = if n >= 0 {
        format!("{:b}", n)
    } else {
        format!("-{:b}", -n)
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_fmt_bin"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_fmt_bin_abi(n: i64) -> Value {
    Value::from_ptr(rt_int_fmt_bin(n))
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
pub fn rt_int_fmt_grouped(n: i64, sep: i64) -> *mut Obj {
    let sep_char = sep as u8 as char;
    let s = if n >= 0 {
        insert_grouping(&format!("{}", n), sep_char)
    } else {
        format!("-{}", insert_grouping(&format!("{}", -n), sep_char))
    };
    let bytes = s.as_bytes();
    unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) }
}
#[export_name = "rt_int_fmt_grouped"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_int_fmt_grouped_abi(n: i64, sep: i64) -> Value {
    Value::from_ptr(rt_int_fmt_grouped(n, sep))
}

/// Format float with precision and grouping separator: f"{1234.5:,.2f}" → "1,234.50"
pub fn rt_float_fmt_grouped(f: f64, precision: i64, sep: i64) -> *mut Obj {
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
#[export_name = "rt_float_fmt_grouped"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_float_fmt_grouped_abi(f: f64, precision: i64, sep: i64) -> Value {
    Value::from_ptr(rt_float_fmt_grouped(f, precision, sep))
}

// ==================== Type name functions ====================

/// Get type name for type() builtin
/// Uses type_class() from core-defs as the single source of truth.
pub fn rt_type_name(obj: *mut Obj) -> *mut Obj {
    let v = pyaot_core_defs::Value(obj as u64);
    if obj.is_null() || v.is_none() {
        let s = TypeTagKind::None.type_class();
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }
    if v.is_int() {
        let s = TypeTagKind::Int.type_class();
        let bytes = s.as_bytes();
        return unsafe { rt_make_str(bytes.as_ptr(), bytes.len()) };
    }
    if v.is_bool() {
        let s = TypeTagKind::Bool.type_class();
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
#[export_name = "rt_type_name"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_type_name_abi(obj: Value) -> Value {
    Value::from_ptr(rt_type_name(obj.unwrap_ptr()))
}

/// Extract type name from type string for __name__ attribute access
/// Extracts "int" from "<class 'int'>" for type(x).__name__
/// Input: string object like "<class 'int'>"
/// Output: string object like "int"
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_type_name_extract(type_str: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_type_name_extract"]
pub extern "C" fn rt_type_name_extract_abi(type_str: Value) -> Value {
    Value::from_ptr(rt_type_name_extract(type_str.unwrap_ptr()))
}

/// Default repr for objects without __str__ or __repr__
/// Returns: pointer to string like "<object at 0x...>"
pub fn rt_obj_default_repr(obj: *mut Obj) -> *mut Obj {
    unsafe {
        let s = format!("<object at {:p}>", obj);
        let bytes = s.as_bytes();
        rt_make_str(bytes.as_ptr(), bytes.len())
    }
}
#[export_name = "rt_obj_default_repr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_obj_default_repr_abi(obj: Value) -> Value {
    Value::from_ptr(rt_obj_default_repr(obj.unwrap_ptr()))
}

/// Convert string to integer with given base
/// s: pointer to StrObj
/// base: numeric base (2, 8, 10, or 16)
/// Returns: integer value
pub fn rt_str_to_int_with_base(s: *mut Obj, base: i64) -> i64 {
    if s.is_null() {
        unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "invalid literal for int()"
            );
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
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "int() base must be >= 2 and <= 36, or 0"
                );
            }

            match i64::from_str_radix(trimmed_str, actual_base) {
                Ok(val) => val,
                Err(_) => {
                    raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "invalid literal for int() with base"
                    );
                }
            }
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "invalid literal for int()"
            );
        }
    }
}
#[export_name = "rt_str_to_int_with_base"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_to_int_with_base_abi(s: Value, base: i64) -> i64 {
    rt_str_to_int_with_base(s.unwrap_ptr(), base)
}

// Suppress unused import warning
#[allow(unused_imports)]
use ObjHeader as _;
