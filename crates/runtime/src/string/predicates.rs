//! Character predicate operations: isdigit, isalpha, isalnum, isspace, isupper,
//! islower, isascii.
//!
//! §9 — Unicode-aware: the buffer is decoded to codepoints and classified with
//! the Rust `char::is_*` family, matching CPython's category definitions for the
//! common cases (Latin accented, Cyrillic, Greek, ASCII). For ASCII inputs
//! `char::is_*` agrees with the old `is_ascii_*` byte tests, so existing ASCII
//! behavior is unchanged. Residual divergence (the narrower documented limit):
//! `isdigit` uses `char::is_numeric` (Nd+Nl+No) where exact CPython parity needs
//! Unicode Numeric_Type data the std lacks (e.g. `½`, `Ⅷ`, superscripts); a
//! wrong predicate returns a bool, never crashes (PITFALLS A2). A non-codepoint
//! (invalid UTF-8) or empty string is `False` for every predicate.

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::object::{Obj, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

/// Decode a `StrObj` to `&str`, or `None` for a null pointer / invalid UTF-8.
/// `who` is the caller name for the B18 type-tag seam guard.
///
/// # Safety
/// `str_obj` must be null or a valid `StrObj` pointer.
#[inline]
unsafe fn str_chars<'a>(str_obj: *mut Obj, who: &str) -> Option<&'a str> {
    if str_obj.is_null() {
        return None;
    }
    debug_assert_type_tag!(str_obj, TypeTagKind::Str, who);
    let src = str_obj as *mut StrObj;
    let len = (*src).len;
    let bytes = std::slice::from_raw_parts((*src).data.as_ptr(), len);
    std::str::from_utf8(bytes).ok()
}

/// `True` iff the string is non-empty and every codepoint satisfies `pred`.
#[inline]
unsafe fn all_chars(str_obj: *mut Obj, who: &str, pred: fn(char) -> bool) -> i8 {
    match str_chars(str_obj, who) {
        Some(s) if !s.is_empty() => s.chars().all(pred) as i8,
        _ => 0,
    }
}

/// Check if all characters are digits (codepoint `is_numeric`, §9)
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isdigit(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isdigit", |c| c.is_numeric()) }
}
#[export_name = "rt_str_isdigit"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isdigit_abi(str_obj: Value) -> i8 {
    rt_str_isdigit(str_obj.unwrap_ptr())
}

/// Check if all characters are alphabetic (codepoint `is_alphabetic`, §9)
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isalpha(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isalpha", |c| c.is_alphabetic()) }
}
#[export_name = "rt_str_isalpha"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isalpha_abi(str_obj: Value) -> i8 {
    rt_str_isalpha(str_obj.unwrap_ptr())
}

/// Check if all characters are alphanumeric (codepoint `is_alphanumeric`, §9)
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isalnum(str_obj: *mut Obj) -> i8 {
    unsafe { all_chars(str_obj, "rt_str_isalnum", |c| c.is_alphanumeric()) }
}
#[export_name = "rt_str_isalnum"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isalnum_abi(str_obj: Value) -> i8 {
    rt_str_isalnum(str_obj.unwrap_ptr())
}

/// Check if all characters are whitespace (§9). CPython treats the Unicode
/// White_Space property AS WELL AS the file/group/record/unit separators
/// (U+001C..U+001F) as space; `char::is_whitespace` omits the latter, so add the
/// range explicitly.
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isspace(str_obj: *mut Obj) -> i8 {
    unsafe {
        all_chars(str_obj, "rt_str_isspace", |c| {
            c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
        })
    }
}
#[export_name = "rt_str_isspace"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isspace_abi(str_obj: Value) -> i8 {
    rt_str_isspace(str_obj.unwrap_ptr())
}

/// Check if all cased characters are uppercase and at least one is (§9 — using
/// the Unicode case properties). A lowercase codepoint fails; a titlecase-only
/// codepoint (neither upper nor lower in Rust) leaves `has_cased` false.
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_isupper(str_obj: *mut Obj) -> i8 {
    unsafe {
        let Some(s) = str_chars(str_obj, "rt_str_isupper") else {
            return 0;
        };
        let mut has_cased = false;
        for c in s.chars() {
            if c.is_lowercase() {
                return 0;
            }
            if c.is_uppercase() {
                has_cased = true;
            }
        }
        has_cased as i8
    }
}
#[export_name = "rt_str_isupper"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isupper_abi(str_obj: Value) -> i8 {
    rt_str_isupper(str_obj.unwrap_ptr())
}

/// Check if all cased characters are lowercase and at least one is (§9).
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_islower(str_obj: *mut Obj) -> i8 {
    unsafe {
        let Some(s) = str_chars(str_obj, "rt_str_islower") else {
            return 0;
        };
        let mut has_cased = false;
        for c in s.chars() {
            if c.is_uppercase() {
                return 0;
            }
            if c.is_lowercase() {
                has_cased = true;
            }
        }
        has_cased as i8
    }
}
#[export_name = "rt_str_islower"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_islower_abi(str_obj: Value) -> i8 {
    rt_str_islower(str_obj.unwrap_ptr())
}

/// Check if all characters are ASCII (code points < 128)
/// Returns: 1 (true) or 0 (false)
/// Empty string returns 1 (Python behavior)
pub fn rt_str_isascii(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_isascii");
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Empty string is ASCII
        if len == 0 {
            return 1;
        }

        let data = (*src).data.as_ptr();
        for i in 0..len {
            if !(*data.add(i)).is_ascii() {
                return 0;
            }
        }
        1
    }
}
#[export_name = "rt_str_isascii"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_isascii_abi(str_obj: Value) -> i8 {
    rt_str_isascii(str_obj.unwrap_ptr())
}
