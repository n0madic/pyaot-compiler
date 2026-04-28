//! First-class builtin function wrappers and function pointer table.
//!
//! This module provides wrapper functions for builtins that can be used as first-class values.
//! When `len`, `str`, `int`, etc. are passed to `map()`, `filter()`, `sorted()`, or assigned
//! to variables, they go through these wrappers which handle runtime type dispatch.
//!
//! All builtin wrappers take a single `*mut Obj` argument and return `*mut Obj` (boxed result),
//! matching the calling convention used by `map()` runtime implementation.
//! Functions that return int/float/bool box their result for uniform handling.

use crate::boxing;
use crate::object::{BytesObj, FloatObj, Obj, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

// =============================================================================
// BUILTIN WRAPPER FUNCTIONS
// =============================================================================
//
// These wrappers take a boxed object and dispatch to the appropriate implementation
// based on the object's type tag. They return boxed values so they can be used
// uniformly with map(), filter(), sorted(), etc.

/// Raise a TypeError with a static message. Never returns.
///
/// # Safety
/// `msg` must point to valid UTF-8 bytes with length `len`. Both constraints
/// are satisfied when passing a `b"..."` literal.
#[inline(always)]
unsafe fn raise_type_error(msg: &'static str) -> ! {
    raise_exc!(pyaot_core_defs::BuiltinExceptionKind::TypeError, "{}", msg)
}

/// len(obj) -> *mut Obj (boxed Int)
/// Returns the length of sequences (list, tuple, dict, set, str, bytes).
pub fn rt_builtin_len(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        // SAFETY: static byte string literal is valid for the duration of the call.
        unsafe { raise_type_error("len() argument is None") };
    }
    if !pyaot_core_defs::Value(obj as u64).is_ptr() {
        unsafe { raise_type_error("object of this type has no len()") };
    }
    let len = unsafe {
        match (*obj).type_tag() {
            TypeTagKind::List => crate::list::rt_list_len(obj),
            TypeTagKind::Str => crate::string::rt_str_len_int(obj),
            TypeTagKind::Dict => crate::dict::rt_dict_len(obj),
            TypeTagKind::Tuple => crate::tuple::rt_tuple_len(obj),
            TypeTagKind::Set => crate::set::rt_set_len(obj),
            TypeTagKind::Bytes => crate::bytes::rt_bytes_len(obj),
            _ => raise_type_error("object of this type has no len()"),
        }
    };
    // Box the result
    pyaot_core_defs::Value::from_int(len).0 as *mut crate::object::Obj
}
#[export_name = "rt_builtin_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_len_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_len(obj.unwrap_ptr()))
}

/// str(obj) -> *mut Obj (StrObj)
/// Converts any object to its string representation.
pub fn rt_builtin_str(obj: *mut Obj) -> *mut Obj {
    crate::conversions::rt_obj_to_str(obj)
}
#[export_name = "rt_builtin_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_str_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_str(obj.unwrap_ptr()))
}

/// int(obj) -> *mut Obj (boxed Int)
/// Converts string or float to integer.
pub fn rt_builtin_int(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        // SAFETY: static byte string literal.
        unsafe { raise_type_error("int() argument is None") };
    }
    // Check tagged primitives before heap dereference.
    let v = Value(obj as u64);
    let value = if v.is_int() {
        v.unwrap_int()
    } else if v.is_bool() {
        v.unwrap_bool() as i64
    } else {
        unsafe {
            match (*obj).type_tag() {
                TypeTagKind::Float => (*(obj as *mut FloatObj)).value as i64,
                TypeTagKind::Str => crate::conversions::rt_str_to_int(obj),
                _ => raise_type_error(
                    "int() argument must be a string, a bytes-like object or a real number",
                ),
            }
        }
    };
    // Box the result
    pyaot_core_defs::Value::from_int(value).0 as *mut crate::object::Obj
}
#[export_name = "rt_builtin_int"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_int_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_int(obj.unwrap_ptr()))
}

/// float(obj) -> *mut Obj (boxed Float)
/// Converts string or int to float.
pub fn rt_builtin_float(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        // SAFETY: static byte string literal.
        unsafe { raise_type_error("float() argument is None") };
    }
    // Check tagged primitives before heap dereference.
    let v = Value(obj as u64);
    let result: f64 = if v.is_int() {
        v.unwrap_int() as f64
    } else if v.is_bool() {
        if v.unwrap_bool() {
            1.0
        } else {
            0.0
        }
    } else {
        unsafe {
            match (*obj).type_tag() {
                TypeTagKind::Float => (*(obj as *mut FloatObj)).value,
                TypeTagKind::Str => crate::conversions::rt_str_to_float(obj),
                _ => raise_type_error("float() argument must be a string or a real number"),
            }
        }
    };
    // Box the result
    boxing::rt_box_float(result)
}
#[export_name = "rt_builtin_float"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_float_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_float(obj.unwrap_ptr()))
}

/// bool(obj) -> *mut Obj (boxed Bool)
/// Returns truthiness of any object.
pub fn rt_builtin_bool(obj: *mut Obj) -> *mut Obj {
    // Check tagged primitives first (null = None = falsy).
    let v = Value(obj as u64);
    let value = if obj.is_null() || v.is_none() {
        false // None is falsy
    } else if v.is_int() {
        v.unwrap_int() != 0
    } else if v.is_bool() {
        v.unwrap_bool()
    } else {
        unsafe {
            match (*obj).type_tag() {
                TypeTagKind::Float => (*(obj as *mut FloatObj)).value != 0.0,
                TypeTagKind::Str => {
                    let str_obj = obj as *mut StrObj;
                    (*str_obj).len > 0
                }
                TypeTagKind::List => crate::list::rt_list_len(obj) > 0,
                TypeTagKind::Tuple => crate::tuple::rt_tuple_len(obj) > 0,
                TypeTagKind::Dict => crate::dict::rt_dict_len(obj) > 0,
                TypeTagKind::Set => crate::set::rt_set_len(obj) > 0,
                TypeTagKind::Bytes => {
                    let bytes_obj = obj as *mut BytesObj;
                    (*bytes_obj).len > 0
                }
                TypeTagKind::None => false,
                // All other types (instances, iterators, etc.) are truthy
                _ => true,
            }
        }
    };
    // Box the result
    pyaot_core_defs::Value::from_bool(value).0 as *mut crate::object::Obj
}
#[export_name = "rt_builtin_bool"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_bool_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_bool(obj.unwrap_ptr()))
}

/// abs(obj) -> *mut Obj (boxed Int or Float)
/// Returns absolute value of number.
pub fn rt_builtin_abs(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        // SAFETY: static byte string literal.
        unsafe { raise_type_error("abs() argument is None") };
    }
    // Check tagged primitives before heap dereference.
    let v = Value(obj as u64);
    if v.is_int() {
        return pyaot_core_defs::Value::from_int(v.unwrap_int().abs()).0 as *mut crate::object::Obj;
    }
    if v.is_bool() {
        // abs(True) = 1, abs(False) = 0
        return pyaot_core_defs::Value::from_int(v.unwrap_bool() as i64).0
            as *mut crate::object::Obj;
    }
    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Float => {
                let value = (*(obj as *mut FloatObj)).value;
                boxing::rt_box_float(value.abs())
            }
            _ => raise_type_error("bad operand type for abs()"),
        }
    }
}
#[export_name = "rt_builtin_abs"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_abs_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_abs(obj.unwrap_ptr()))
}

/// hash(obj) -> *mut Obj (boxed Int)
/// Returns hash value of hashable object.
pub fn rt_builtin_hash(obj: *mut Obj) -> *mut Obj {
    // Check tagged primitives before heap dereference.
    let v = Value(obj as u64);
    let value = if obj.is_null() || v.is_none() {
        0 // hash(None) == 0
    } else if v.is_int() {
        crate::hash::rt_hash_int(v.unwrap_int())
    } else if v.is_bool() {
        crate::hash::rt_hash_bool(if v.unwrap_bool() { 1 } else { 0 })
    } else {
        unsafe {
            match (*obj).type_tag() {
                TypeTagKind::Float => {
                    let float_obj = obj as *mut FloatObj;
                    let fv = (*float_obj).value;
                    if fv == 0.0 {
                        0 // hash(-0.0) == hash(0.0) == 0
                    } else if fv.fract() == 0.0 && fv.is_finite() {
                        // Integer-valued float: same hash as the equivalent integer
                        crate::hash::rt_hash_int(fv as i64)
                    } else {
                        // Non-integer float: use bit representation as input to the scramble
                        crate::hash::rt_hash_int(fv.to_bits() as i64)
                    }
                }
                TypeTagKind::Str => crate::hash::rt_hash_str(obj),
                TypeTagKind::Tuple => crate::hash::rt_hash_tuple(obj),
                TypeTagKind::None => 0,
                _ => raise_type_error("unhashable type"),
            }
        }
    };
    // Box the result
    pyaot_core_defs::Value::from_int(value).0 as *mut crate::object::Obj
}
#[export_name = "rt_builtin_hash"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_hash_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_hash(obj.unwrap_ptr()))
}

/// ord(obj) -> *mut Obj (boxed Int)
/// Returns Unicode code point of single-character string.
pub fn rt_builtin_ord(obj: *mut Obj) -> *mut Obj {
    let value = crate::conversions::rt_chr_to_int(obj);
    pyaot_core_defs::Value::from_int(value).0 as *mut crate::object::Obj
}
#[export_name = "rt_builtin_ord"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_ord_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_ord(obj.unwrap_ptr()))
}

/// chr(obj) -> *mut Obj (StrObj)
/// Returns single-character string from Unicode code point.
/// Note: obj must be a boxed IntObj containing the code point.
pub fn rt_builtin_chr(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        // SAFETY: static byte string literal.
        unsafe { raise_type_error("chr() argument is None") };
    }
    let v = Value(obj as u64);
    if v.is_int() {
        return crate::conversions::rt_int_to_chr(v.unwrap_int());
    }
    // SAFETY: static byte string literal is valid for the duration of the call.
    unsafe { raise_type_error("an integer is required for chr()") }
}
#[export_name = "rt_builtin_chr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_chr_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_chr(obj.unwrap_ptr()))
}

/// repr(obj) -> *mut Obj (StrObj)
/// Returns repr string of object.
pub fn rt_builtin_repr(obj: *mut Obj) -> *mut Obj {
    crate::conversions::rt_repr_collection(obj)
}
#[export_name = "rt_builtin_repr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_repr_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_repr(obj.unwrap_ptr()))
}

/// type(obj) -> *mut Obj (StrObj)
/// Returns type name string of object.
pub fn rt_builtin_type(obj: *mut Obj) -> *mut Obj {
    crate::conversions::rt_type_name(obj)
}
#[export_name = "rt_builtin_type"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_builtin_type_abi(obj: Value) -> Value {
    Value::from_ptr(rt_builtin_type(obj.unwrap_ptr()))
}

// =============================================================================
// FUNCTION POINTER TABLE
// =============================================================================
//
// Instead of a static array (which can't hold function pointers as const in Rust),
// we use a match-based dispatch. The IDs MUST match the order in core-defs/src/builtins.rs.

/// Internal implementation for getting builtin function pointer.
/// This non-FFI version can be tested with `#[should_panic]`.
#[cfg_attr(not(test), allow(dead_code))]
fn get_builtin_func_ptr_impl(builtin_id: i64) -> i64 {
    match builtin_id {
        0 => rt_builtin_len as *const () as usize as i64,
        1 => rt_builtin_str as *const () as usize as i64,
        2 => rt_builtin_int as *const () as usize as i64,
        3 => rt_builtin_float as *const () as usize as i64,
        4 => rt_builtin_bool as *const () as usize as i64,
        5 => rt_builtin_abs as *const () as usize as i64,
        6 => rt_builtin_hash as *const () as usize as i64,
        7 => rt_builtin_ord as *const () as usize as i64,
        8 => rt_builtin_chr as *const () as usize as i64,
        9 => rt_builtin_repr as *const () as usize as i64,
        10 => rt_builtin_type as *const () as usize as i64,
        _ => panic!(
            "Invalid builtin ID: {} (max: {})",
            builtin_id,
            pyaot_core_defs::BUILTIN_FUNCTION_COUNT - 1
        ),
    }
}

/// Get function pointer for a builtin by its ID.
/// Called from codegen via BuiltinAddr instruction.
/// Raises a RuntimeError (via longjmp) if `builtin_id` is out of range.
#[no_mangle]
pub extern "C" fn rt_get_builtin_func_ptr(builtin_id: i64) -> i64 {
    match builtin_id {
        0 => rt_builtin_len as *const () as usize as i64,
        1 => rt_builtin_str as *const () as usize as i64,
        2 => rt_builtin_int as *const () as usize as i64,
        3 => rt_builtin_float as *const () as usize as i64,
        4 => rt_builtin_bool as *const () as usize as i64,
        5 => rt_builtin_abs as *const () as usize as i64,
        6 => rt_builtin_hash as *const () as usize as i64,
        7 => rt_builtin_ord as *const () as usize as i64,
        8 => rt_builtin_chr as *const () as usize as i64,
        9 => rt_builtin_repr as *const () as usize as i64,
        10 => rt_builtin_type as *const () as usize as i64,
        _ => unsafe {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::RuntimeError,
                "invalid builtin function ID"
            )
        },
    }
}

// =============================================================================
// COMPILE-TIME VERIFICATION
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_core_defs::BuiltinFunctionKind;

    #[test]
    fn test_builtin_count_matches() {
        // Verify that we have entries for all builtins
        assert_eq!(
            pyaot_core_defs::BUILTIN_FUNCTION_COUNT,
            11,
            "BUILTIN_FUNCTION_COUNT should be 11"
        );
    }

    #[test]
    fn test_all_builtins_have_entries() {
        // Verify every builtin kind has a valid function pointer
        for kind in BuiltinFunctionKind::ALL {
            let id = kind.id() as i64;
            let ptr = get_builtin_func_ptr_impl(id);
            assert_ne!(
                ptr, 0,
                "Builtin {:?} (id={}) has null function pointer",
                kind, id
            );
        }
    }

    #[test]
    fn test_get_builtin_func_ptr_valid() {
        // Test all valid IDs using internal impl (not extern "C" which raises exception on invalid)
        for i in 0..pyaot_core_defs::BUILTIN_FUNCTION_COUNT {
            let ptr = get_builtin_func_ptr_impl(i as i64);
            assert_ne!(ptr, 0, "Builtin ID {} returned null pointer", i);
        }
    }

    #[test]
    #[should_panic(expected = "Invalid builtin ID")]
    fn test_get_builtin_func_ptr_invalid() {
        // Use internal impl because the extern "C" version raises a Python exception
        // (via longjmp) instead of panicking.
        get_builtin_func_ptr_impl(100);
    }
}
