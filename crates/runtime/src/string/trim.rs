//! Trimming operations: strip, lstrip, rstrip

use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};

use super::core::rt_make_str;

/// Strip whitespace from both ends of string
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_strip(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;
        let data = (*src).data.as_ptr();

        // Find start (skip leading whitespace)
        let mut start = 0;
        while start < len {
            let c = *data.add(start);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                break;
            }
            start += 1;
        }

        // Find end (skip trailing whitespace)
        let mut end = len;
        while end > start {
            let c = *data.add(end - 1);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                break;
            }
            end -= 1;
        }

        let result_len = end - start;

        // Allocate new string
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = result_len;

        // Copy trimmed data
        if result_len > 0 {
            std::ptr::copy_nonoverlapping(
                data.add(start),
                (*new_str).data.as_mut_ptr(),
                result_len,
            );
        }

        obj
    }
}

/// Strip whitespace from left side
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_lstrip(str_obj: *mut Obj, chars: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let src_data = (*src).data.as_ptr();

        let mut start = 0;

        if chars.is_null() {
            // Strip whitespace
            while start < src_len {
                let c = *src_data.add(start);
                if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                    break;
                }
                start += 1;
            }
        } else {
            // Strip specified characters
            let chars_str = chars as *mut StrObj;
            let chars_len = (*chars_str).len;
            let chars_data = (*chars_str).data.as_ptr();

            while start < src_len {
                let c = *src_data.add(start);
                let mut found = false;
                for j in 0..chars_len {
                    if c == *chars_data.add(j) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    break;
                }
                start += 1;
            }
        }

        rt_make_str(src_data.add(start), src_len - start)
    }
}

/// Strip whitespace from right side
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_rstrip(str_obj: *mut Obj, chars: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let src_data = (*src).data.as_ptr();

        let mut end = src_len;

        if chars.is_null() {
            // Strip whitespace
            while end > 0 {
                let c = *src_data.add(end - 1);
                if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                    break;
                }
                end -= 1;
            }
        } else {
            // Strip specified characters
            let chars_str = chars as *mut StrObj;
            let chars_len = (*chars_str).len;
            let chars_data = (*chars_str).data.as_ptr();

            while end > 0 {
                let c = *src_data.add(end - 1);
                let mut found = false;
                for j in 0..chars_len {
                    if c == *chars_data.add(j) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    break;
                }
                end -= 1;
            }
        }

        rt_make_str(src_data, end)
    }
}
