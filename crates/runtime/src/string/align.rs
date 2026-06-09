//! Alignment operations: center, ljust, rjust, zfill

use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

/// Center string with fill character
/// Returns: new string
pub fn rt_str_center(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
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
        // gc_alloc may trigger a collection, which would invalidate any raw pointer
        // derived from str_obj before this call. Re-derive src_data afterwards.
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Left padding
        for i in 0..left_pad {
            *dst_data.add(i) = fill;
        }

        // Content — re-derive src_data AFTER gc_alloc to avoid use-after-free.
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
        std::ptr::copy_nonoverlapping(src_data, dst_data.add(left_pad), src_len);

        // Right padding
        for i in 0..right_pad {
            *dst_data.add(left_pad + src_len + i) = fill;
        }

        obj
    }
}
#[export_name = "rt_str_center"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_center_abi(str_obj: Value, width: i64, fillchar: Value) -> Value {
    Value::from_ptr(rt_str_center(
        str_obj.unwrap_ptr(),
        width,
        fillchar.unwrap_ptr(),
    ))
}

/// Left justify string with fill character
/// Returns: new string
pub fn rt_str_ljust(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
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
        // gc_alloc may trigger a collection, which would invalidate any raw pointer
        // derived from str_obj before this call. Re-derive src_data afterwards.
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Content — re-derive src_data AFTER gc_alloc to avoid use-after-free.
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
        std::ptr::copy_nonoverlapping(src_data, dst_data, src_len);

        // Right padding
        for i in src_len..width {
            *dst_data.add(i) = fill;
        }

        obj
    }
}
#[export_name = "rt_str_ljust"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_ljust_abi(str_obj: Value, width: i64, fillchar: Value) -> Value {
    Value::from_ptr(rt_str_ljust(
        str_obj.unwrap_ptr(),
        width,
        fillchar.unwrap_ptr(),
    ))
}

/// Right justify string with fill character
/// Returns: new string
pub fn rt_str_rjust(str_obj: *mut Obj, width: i64, fillchar: *mut Obj) -> *mut Obj {
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
        // gc_alloc may trigger a collection, which would invalidate any raw pointer
        // derived from str_obj before this call. Re-derive src_data afterwards.
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Left padding
        for i in 0..padding {
            *dst_data.add(i) = fill;
        }

        // Content — re-derive src_data AFTER gc_alloc to avoid use-after-free.
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
        std::ptr::copy_nonoverlapping(src_data, dst_data.add(padding), src_len);

        obj
    }
}
#[export_name = "rt_str_rjust"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_rjust_abi(str_obj: Value, width: i64, fillchar: Value) -> Value {
    Value::from_ptr(rt_str_rjust(
        str_obj.unwrap_ptr(),
        width,
        fillchar.unwrap_ptr(),
    ))
}

/// Zero-fill string (left pad with zeros, preserving sign)
/// Returns: new string
pub fn rt_str_zfill(str_obj: *mut Obj, width: i64) -> *mut Obj {
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

        let padding = width - src_len;

        // Read has_sign from src_data before gc_alloc; no collection can occur yet.
        let has_sign = if src_len > 0 {
            let first = *(*src).data.as_ptr();
            first == b'+' || first == b'-'
        } else {
            false
        };

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + width;
        // gc_alloc may trigger a collection, which would invalidate any raw pointer
        // derived from str_obj before this call. Re-derive src_data afterwards.
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = width;

        let dst_data = (*result).data.as_mut_ptr();

        // Re-derive src_data AFTER gc_alloc to avoid use-after-free.
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();

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
#[export_name = "rt_str_zfill"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_zfill_abi(str_obj: Value, width: i64) -> Value {
    Value::from_ptr(rt_str_zfill(str_obj.unwrap_ptr(), width))
}
