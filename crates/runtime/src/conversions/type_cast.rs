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
        // Class instances: use the registered qualified name so `type(w)`
        // gives `<class '__main__.Widget'>` (CPython compatible) instead of
        // the generic `<class 'object'>` from the `Instance` type tag.
        if (*obj).header.type_tag == TypeTagKind::Instance {
            let class_id = (*(obj as *const crate::object::InstanceObj)).class_id;
            if let Some(qn) = crate::instance::lookup_class_qualname(class_id) {
                let s = format!("<class '{}'>", qn);
                let bytes = s.as_bytes();
                return rt_make_str(bytes.as_ptr(), bytes.len());
            }
        }
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
///
/// `__name__` is the bare class name, NOT module-qualified: CPython's
/// `type(w).__name__` is `"Widget"` (not `"__main__.Widget"`) and
/// `time.struct_time.__name__` is `"struct_time"`. So the last dotted
/// segment of the quoted name is taken.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_type_name_extract(type_str: *mut Obj) -> *mut Obj {
    let full_str = unsafe { crate::utils::str_obj_to_rust_string(type_str) };

    // Parse "<class 'module.typename'>" → bare "typename" (last `.` segment).
    if let Some(start) = full_str.find("'") {
        if let Some(end) = full_str.rfind("'") {
            if start < end {
                let qualified = &full_str[start + 1..end];
                let name = qualified.rsplit('.').next().unwrap_or(qualified);
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

/// Default repr for objects without __str__ or __repr__.
/// Returns: pointer to a string like "<__main__.Cls object at 0x...>"
/// (CPython compatible; falls back to "<object at 0x...>" when the class
/// registered no qualified name).
pub fn rt_obj_default_repr(obj: *mut Obj) -> *mut Obj {
    unsafe {
        let s = crate::instance::instance_default_repr(obj);
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
