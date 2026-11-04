//! Trimming operations: strip, lstrip, rstrip
//!
//! Character-wise, like CPython: the default strips every character for which
//! `str.isspace()` is true (full Unicode whitespace), and a `chars` argument is
//! a SET OF CODE POINTS (never bytes — stripping UTF-8 bytes could split a
//! character).

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{Obj, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

use super::core::rt_make_str;

/// The kept byte range `(start, end)` and the number of code points stripped,
/// for a strip of `text` from the left and/or right. `chars = None` strips
/// Unicode whitespace.
fn strip_range(text: &str, chars: Option<&str>, left: bool, right: bool) -> (usize, usize, usize) {
    let strip = |c: char| match chars {
        None => super::predicates::is_py_space(c),
        Some(set) => set.contains(c),
    };
    let mut start = 0;
    let mut stripped = 0;
    if left {
        for (i, c) in text.char_indices() {
            if !strip(c) {
                start = i;
                break;
            }
            start = i + c.len_utf8();
            stripped += 1;
        }
    }
    let mut end = text.len();
    if right {
        for (i, c) in text[start..].char_indices().rev() {
            if !strip(c) {
                break;
            }
            end = start + i;
            stripped += 1;
        }
    }
    (start, end, stripped)
}

/// Shared strip driver: compute the kept range (no allocation), then copy it
/// into a fresh string, re-deriving the source data after `gc_alloc`.
///
/// # Safety
/// `str_obj` must be a valid, caller-rooted `StrObj`; `chars` null or a valid
/// `StrObj`.
unsafe fn strip_impl(str_obj: *mut Obj, chars: *mut Obj, left: bool, right: bool) -> *mut Obj {
    let src = str_obj as *mut StrObj;
    let src_len = (*src).len;
    let src_char_len = (*src).char_len;
    let (start, end, stripped) = {
        let text = std::str::from_utf8_unchecked(std::slice::from_raw_parts(
            (*src).data.as_ptr(),
            src_len,
        ));
        if chars.is_null() || Value(chars as u64).is_none() {
            strip_range(text, None, left, right)
        } else {
            let c = chars as *mut StrObj;
            let set = std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                (*c).data.as_ptr(),
                (*c).len,
            ));
            strip_range(text, Some(set), left, right)
        }
    };
    let result_len = end - start;
    let size = crate::string::core::str_alloc_size(result_len);
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
    let new_str = obj as *mut StrObj;
    (*new_str).len = result_len;
    (*new_str).char_len = src_char_len - stripped;
    if result_len > 0 {
        // Re-derive the source pointer AFTER gc_alloc (str_obj is a live root).
        let fresh = (*(str_obj as *mut StrObj)).data.as_ptr();
        std::ptr::copy_nonoverlapping(fresh.add(start), (*new_str).data.as_mut_ptr(), result_len);
    }
    obj
}

/// Strip whitespace from both ends of string
/// Returns: pointer to new allocated StrObj
pub fn rt_str_strip(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_strip");
        strip_impl(str_obj, std::ptr::null_mut(), true, true)
    }
}
#[export_name = "rt_str_strip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_strip_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_strip(str_obj.unwrap_ptr()))
}

/// Strip whitespace (or the code points in `chars`) from the left side
/// Returns: new string
pub fn rt_str_lstrip(str_obj: *mut Obj, chars: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_lstrip");
        strip_impl(str_obj, chars, true, false)
    }
}
#[export_name = "rt_str_lstrip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_lstrip_abi(str_obj: Value, chars: Value) -> Value {
    Value::from_ptr(rt_str_lstrip(str_obj.unwrap_ptr(), chars.unwrap_ptr()))
}

/// Strip whitespace (or the code points in `chars`) from the right side
/// Returns: new string
pub fn rt_str_rstrip(str_obj: *mut Obj, chars: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_rstrip");
        strip_impl(str_obj, chars, false, true)
    }
}
#[export_name = "rt_str_rstrip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_rstrip_abi(str_obj: Value, chars: Value) -> Value {
    Value::from_ptr(rt_str_rstrip(str_obj.unwrap_ptr(), chars.unwrap_ptr()))
}
