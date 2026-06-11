//! Case conversion operations: upper, lower, title, capitalize, swapcase
//!
//! All conversions are Unicode-aware (Rust `char` full case mappings), which
//! matches CPython for the common cases (Cyrillic, Greek, Latin-1, 'ß' → "SS").
//! The source bytes are copied into a Rust `String` BEFORE any allocation, so
//! no GC rooting is needed.

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
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

/// True iff `c` is a cased character (approximates CPython's Lu/Ll/Lt check).
fn is_cased(c: char) -> bool {
    c.is_lowercase() || c.is_uppercase()
}

/// Convert string to uppercase
/// Returns: pointer to new allocated StrObj
pub fn rt_str_upper(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_upper");
        let result: String = str_obj_to_string(str_obj).chars().flat_map(|c| c.to_uppercase()).collect();
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_upper"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_upper_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_upper(str_obj.unwrap_ptr()))
}

/// Convert string to lowercase
/// Returns: pointer to new allocated StrObj
pub fn rt_str_lower(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_lower");
        let result: String = str_obj_to_string(str_obj).chars().flat_map(|c| c.to_lowercase()).collect();
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_lower"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_lower_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_lower(str_obj.unwrap_ptr()))
}

/// Title case: uppercase each character that follows a non-cased character,
/// lowercase the rest (CPython's word-boundary rule — "hello-world" →
/// "Hello-World", not whitespace-only boundaries).
/// Returns: new string
pub fn rt_str_title(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_title");
        let mut result = String::new();
        let mut prev_cased = false;
        for c in str_obj_to_string(str_obj).chars() {
            if prev_cased {
                result.extend(c.to_lowercase());
            } else {
                result.extend(c.to_uppercase());
            }
            prev_cased = is_cased(c);
        }
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_title"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_title_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_title(str_obj.unwrap_ptr()))
}

/// Capitalize: first character uppercase, rest lowercase
/// Returns: new string
pub fn rt_str_capitalize(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_capitalize");
        let mut result = String::new();
        for (i, c) in str_obj_to_string(str_obj).chars().enumerate() {
            if i == 0 {
                result.extend(c.to_uppercase());
            } else {
                result.extend(c.to_lowercase());
            }
        }
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_capitalize"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_capitalize_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_capitalize(str_obj.unwrap_ptr()))
}

/// Swapcase: swap upper and lower case
/// Returns: new string
pub fn rt_str_swapcase(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_swapcase");
        let mut result = String::new();
        for c in str_obj_to_string(str_obj).chars() {
            if c.is_uppercase() {
                result.extend(c.to_lowercase());
            } else if c.is_lowercase() {
                result.extend(c.to_uppercase());
            } else {
                result.push(c);
            }
        }
        rt_make_str(result.as_ptr(), result.len())
    }
}
#[export_name = "rt_str_swapcase"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_swapcase_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_swapcase(str_obj.unwrap_ptr()))
}
