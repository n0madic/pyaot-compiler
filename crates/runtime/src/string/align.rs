//! Alignment operations: center, ljust, rjust, zfill

use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};

/// Center string with fill character
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_center(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let width = width as usize;

        if src_len >= width {
            return str_obj;
        }

        let fill = if fillchar.is_null() {
            b' '
        } else {
            let fill_str = fillchar as *mut StrObj;
            if (*fill_str).len > 0 {
                *(*fill_str).data.as_ptr()
            } else {
                b' '
            }
        };

        let padding = width - src_len;
        let left_pad = padding / 2;
        let right_pad = padding - left_pad;

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + width;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Left padding
        for i in 0..left_pad {
            *dst_data.add(i) = fill;
        }

        // Content
        std::ptr::copy_nonoverlapping((*src).data.as_ptr(), dst_data.add(left_pad), src_len);

        // Right padding
        for i in 0..right_pad {
            *dst_data.add(left_pad + src_len + i) = fill;
        }

        obj
    }
}

/// Left justify string with fill character
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_ljust(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let width = width as usize;

        if src_len >= width {
            return str_obj;
        }

        let fill = if fillchar.is_null() {
            b' '
        } else {
            let fill_str = fillchar as *mut StrObj;
            if (*fill_str).len > 0 {
                *(*fill_str).data.as_ptr()
            } else {
                b' '
            }
        };

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + width;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Content
        std::ptr::copy_nonoverlapping((*src).data.as_ptr(), dst_data, src_len);

        // Right padding
        for i in src_len..width {
            *dst_data.add(i) = fill;
        }

        obj
    }
}

/// Right justify string with fill character
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_rjust(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let width = width as usize;

        if src_len >= width {
            return str_obj;
        }

        let fill = if fillchar.is_null() {
            b' '
        } else {
            let fill_str = fillchar as *mut StrObj;
            if (*fill_str).len > 0 {
                *(*fill_str).data.as_ptr()
            } else {
                b' '
            }
        };

        let padding = width - src_len;

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + width;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Left padding
        for i in 0..padding {
            *dst_data.add(i) = fill;
        }

        // Content
        std::ptr::copy_nonoverlapping((*src).data.as_ptr(), dst_data.add(padding), src_len);

        obj
    }
}

/// Zero-fill string (left pad with zeros, preserving sign)
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_zfill(str_obj: *mut Obj, width: i64) -> *mut Obj {
    if str_obj.is_null() || width <= 0 {
        return str_obj;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let width = width as usize;

        if src_len >= width {
            return str_obj;
        }

        let src_data = (*src).data.as_ptr();
        let padding = width - src_len;

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + width;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Check for sign prefix
        let has_sign = if src_len > 0 {
            let first = *src_data;
            first == b'+' || first == b'-'
        } else {
            false
        };

        if has_sign {
            // Copy sign first
            *dst_data = *src_data;
            // Zero padding after sign
            for i in 0..padding {
                *dst_data.add(1 + i) = b'0';
            }
            // Copy rest of string
            std::ptr::copy_nonoverlapping(src_data.add(1), dst_data.add(1 + padding), src_len - 1);
        } else {
            // Zero padding at start
            for i in 0..padding {
                *dst_data.add(i) = b'0';
            }
            // Copy string
            std::ptr::copy_nonoverlapping(src_data, dst_data.add(padding), src_len);
        }

        obj
    }
}
