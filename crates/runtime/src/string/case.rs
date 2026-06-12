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

/// True iff the string is pure ASCII, read from the cached `char_len` (item #5's
/// `char_len == len ⟺ ASCII` invariant — no separate is-ASCII bit, no re-walk).
/// ASCII case mapping is byte-local and identical to the Unicode mapping, so a
/// byte-wise pass over `data` is semantically equivalent to the `chars()` path
/// while avoiding the UTF-8 decode + char iteration + intermediate `String`.
unsafe fn is_ascii_str(str_obj: *mut Obj) -> bool {
    let src = str_obj as *mut StrObj;
    (*src).char_len == (*src).len
}

/// Borrow a StrObj's raw bytes (the same slice `str_obj_to_string` decodes).
unsafe fn str_bytes<'a>(str_obj: *mut Obj) -> &'a [u8] {
    let src = str_obj as *mut StrObj;
    std::slice::from_raw_parts((*src).data.as_ptr(), (*src).len)
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
        if is_ascii_str(str_obj) {
            let bytes = str_bytes(str_obj).to_ascii_uppercase();
            return rt_make_str(bytes.as_ptr(), bytes.len());
        }
        let result: String = str_obj_to_string(str_obj)
            .chars()
            .flat_map(|c| c.to_uppercase())
            .collect();
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
        if is_ascii_str(str_obj) {
            let bytes = str_bytes(str_obj).to_ascii_lowercase();
            return rt_make_str(bytes.as_ptr(), bytes.len());
        }
        let result: String = str_obj_to_string(str_obj)
            .chars()
            .flat_map(|c| c.to_lowercase())
            .collect();
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
        if is_ascii_str(str_obj) {
            // Byte-wise title-casing: a letter starting a word goes upper, the
            // rest of the word lower. `is_ascii_alphabetic` is the ASCII analogue
            // of `is_cased` (case-converting a letter keeps it a letter).
            let mut out = str_bytes(str_obj).to_vec();
            let mut prev_cased = false;
            for b in out.iter_mut() {
                if prev_cased {
                    b.make_ascii_lowercase();
                } else {
                    b.make_ascii_uppercase();
                }
                prev_cased = b.is_ascii_alphabetic();
            }
            return rt_make_str(out.as_ptr(), out.len());
        }
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
        if is_ascii_str(str_obj) {
            let mut out = str_bytes(str_obj).to_vec();
            if let Some(first) = out.first_mut() {
                first.make_ascii_uppercase();
            }
            for b in out.iter_mut().skip(1) {
                b.make_ascii_lowercase();
            }
            return rt_make_str(out.as_ptr(), out.len());
        }
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
        if is_ascii_str(str_obj) {
            let mut out = str_bytes(str_obj).to_vec();
            for b in out.iter_mut() {
                if b.is_ascii_uppercase() {
                    b.make_ascii_lowercase();
                } else if b.is_ascii_lowercase() {
                    b.make_ascii_uppercase();
                }
            }
            return rt_make_str(out.as_ptr(), out.len());
        }
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
