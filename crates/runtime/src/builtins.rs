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
use crate::object::{BoolObj, BytesObj, FloatObj, IntObj, Obj, StrObj, TypeTagKind};

// =============================================================================
// BUILTIN WRAPPER FUNCTIONS
// =============================================================================
//
// These wrappers take a boxed object and dispatch to the appropriate implementation
// based on the object's type tag. They return boxed values so they can be used
// uniformly with map(), filter(), sorted(), etc.

/// len(obj) -> *mut Obj (boxed Int)
/// Returns the length of sequences (list, tuple, dict, set, str, bytes).
#[no_mangle]
pub extern "C" fn rt_builtin_len(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        panic!("len() argument is None");
    }
    let len = unsafe {
        match (*obj).type_tag() {
            TypeTagKind::List => crate::list::rt_list_len(obj),
            TypeTagKind::Str => crate::string::rt_str_len_int(obj),
            TypeTagKind::Dict => crate::dict::rt_dict_len(obj),
            TypeTagKind::Tuple => crate::tuple::rt_tuple_len(obj),
            TypeTagKind::Set => crate::set::rt_set_len(obj),
            TypeTagKind::Bytes => crate::bytes::rt_bytes_len(obj),
            _ => panic!("len() not supported for {}", (*obj).type_tag().type_name()),
        }
    };
    // Box the result
    boxing::rt_box_int(len)
}

/// str(obj) -> *mut Obj (StrObj)
/// Converts any object to its string representation.
#[no_mangle]
pub extern "C" fn rt_builtin_str(obj: *mut Obj) -> *mut Obj {
    crate::conversions::rt_obj_to_str(obj)
}

/// int(obj) -> *mut Obj (boxed Int)
/// Converts string or float to integer.
#[no_mangle]
pub extern "C" fn rt_builtin_int(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        panic!("int() argument is None");
    }
    let value = unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Int => (*(obj as *mut IntObj)).value,
            TypeTagKind::Float => (*(obj as *mut FloatObj)).value as i64,
            TypeTagKind::Bool => {
                if (*(obj as *mut BoolObj)).value {
                    1
                } else {
                    0
                }
            }
            TypeTagKind::Str => crate::conversions::rt_str_to_int(obj),
            _ => panic!("int() not supported for {}", (*obj).type_tag().type_name()),
        }
    };
    // Box the result
    boxing::rt_box_int(value)
}

/// float(obj) -> *mut Obj (boxed Float)
/// Converts string or int to float.
#[no_mangle]
pub extern "C" fn rt_builtin_float(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        panic!("float() argument is None");
    }
    let result: f64 = unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Float => (*(obj as *mut FloatObj)).value,
            TypeTagKind::Int => (*(obj as *mut IntObj)).value as f64,
            TypeTagKind::Bool => {
                if (*(obj as *mut BoolObj)).value {
                    1.0
                } else {
                    0.0
                }
            }
            TypeTagKind::Str => crate::conversions::rt_str_to_float(obj),
            _ => panic!(
                "float() not supported for {}",
                (*obj).type_tag().type_name()
            ),
        }
    };
    // Box the result
    boxing::rt_box_float(result)
}

/// bool(obj) -> *mut Obj (boxed Bool)
/// Returns truthiness of any object.
#[no_mangle]
pub extern "C" fn rt_builtin_bool(obj: *mut Obj) -> *mut Obj {
    let value = if obj.is_null() {
        false // None is falsy
    } else {
        unsafe {
            match (*obj).type_tag() {
                TypeTagKind::Bool => (*(obj as *mut BoolObj)).value,
                TypeTagKind::Int => (*(obj as *mut IntObj)).value != 0,
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
    boxing::rt_box_bool(if value { 1 } else { 0 })
}

/// abs(obj) -> *mut Obj (boxed Int or Float)
/// Returns absolute value of number.
#[no_mangle]
pub extern "C" fn rt_builtin_abs(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        panic!("abs() argument is None");
    }
    unsafe {
        match (*obj).type_tag() {
            TypeTagKind::Int => {
                let value = (*(obj as *mut IntObj)).value;
                boxing::rt_box_int(value.abs())
            }
            TypeTagKind::Float => {
                let value = (*(obj as *mut FloatObj)).value;
                boxing::rt_box_float(value.abs())
            }
            TypeTagKind::Bool => {
                // abs(True) = 1, abs(False) = 0
                let value = if (*(obj as *mut BoolObj)).value { 1 } else { 0 };
                boxing::rt_box_int(value)
            }
            _ => panic!("abs() not supported for {}", (*obj).type_tag().type_name()),
        }
    }
}

/// hash(obj) -> *mut Obj (boxed Int)
/// Returns hash value of hashable object.
#[no_mangle]
pub extern "C" fn rt_builtin_hash(obj: *mut Obj) -> *mut Obj {
    let value = if obj.is_null() {
        0 // hash(None) == 0
    } else {
        unsafe {
            match (*obj).type_tag() {
                TypeTagKind::Int => {
                    let int_obj = obj as *mut IntObj;
                    crate::hash::rt_hash_int((*int_obj).value)
                }
                TypeTagKind::Bool => {
                    let bool_obj = obj as *mut BoolObj;
                    crate::hash::rt_hash_bool(if (*bool_obj).value { 1 } else { 0 })
                }
                TypeTagKind::Float => {
                    let float_obj = obj as *mut FloatObj;
                    (*float_obj).value.to_bits() as i64
                }
                TypeTagKind::Str => crate::hash::rt_hash_str(obj),
                TypeTagKind::Tuple => crate::hash::rt_hash_tuple(obj),
                TypeTagKind::None => 0,
                _ => panic!("unhashable type: '{}'", (*obj).type_tag().type_name()),
            }
        }
    };
    // Box the result
    boxing::rt_box_int(value)
}

/// ord(obj) -> *mut Obj (boxed Int)
/// Returns Unicode code point of single-character string.
#[no_mangle]
pub extern "C" fn rt_builtin_ord(obj: *mut Obj) -> *mut Obj {
    let value = crate::conversions::rt_chr_to_int(obj);
    boxing::rt_box_int(value)
}

/// chr(obj) -> *mut Obj (StrObj)
/// Returns single-character string from Unicode code point.
/// Note: obj must be a boxed IntObj containing the code point.
#[no_mangle]
pub extern "C" fn rt_builtin_chr(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        panic!("chr() argument is None");
    }
    unsafe {
        let codepoint = match (*obj).type_tag() {
            TypeTagKind::Int => (*(obj as *mut IntObj)).value,
            _ => panic!("chr() requires int, got {}", (*obj).type_tag().type_name()),
        };
        crate::conversions::rt_int_to_chr(codepoint)
    }
}

/// repr(obj) -> *mut Obj (StrObj)
/// Returns repr string of object.
#[no_mangle]
pub extern "C" fn rt_builtin_repr(obj: *mut Obj) -> *mut Obj {
    crate::conversions::rt_repr_obj(obj)
}

/// type(obj) -> *mut Obj (StrObj)
/// Returns type name string of object.
#[no_mangle]
pub extern "C" fn rt_builtin_type(obj: *mut Obj) -> *mut Obj {
    crate::conversions::rt_type_name(obj)
}

// =============================================================================
// FUNCTION POINTER TABLE
// =============================================================================
//
// Instead of a static array (which can't hold function pointers as const in Rust),
// we use a match-based dispatch. The IDs MUST match the order in core-defs/src/builtins.rs.

/// Internal implementation for getting builtin function pointer.
/// This non-FFI version can be tested with `#[should_panic]`.
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
///
/// # Panics
/// Panics if builtin_id is out of range.
#[no_mangle]
pub extern "C" fn rt_get_builtin_func_ptr(builtin_id: i64) -> i64 {
    get_builtin_func_ptr_impl(builtin_id)
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
            let ptr = rt_get_builtin_func_ptr(id);
            assert_ne!(
                ptr, 0,
                "Builtin {:?} (id={}) has null function pointer",
                kind, id
            );
        }
    }

    #[test]
    fn test_get_builtin_func_ptr_valid() {
        // Test all valid IDs using internal impl (not extern "C" which aborts on panic)
        for i in 0..pyaot_core_defs::BUILTIN_FUNCTION_COUNT {
            let ptr = get_builtin_func_ptr_impl(i as i64);
            assert_ne!(ptr, 0, "Builtin ID {} returned null pointer", i);
        }
    }

    #[test]
    #[should_panic(expected = "Invalid builtin ID")]
    fn test_get_builtin_func_ptr_invalid() {
        // Use internal impl because extern "C" functions abort on panic instead of unwinding
        get_builtin_func_ptr_impl(100);
    }
}
