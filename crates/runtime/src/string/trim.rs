//! Trimming operations: strip, lstrip, rstrip

use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

use super::core::rt_make_str;

/// Strip whitespace from both ends of string
/// Returns: pointer to new allocated StrObj
pub fn rt_str_strip(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Compute start/end offsets using a temporary data pointer — this is safe
        // because we have not called gc_alloc yet, so no collection can occur here.
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

        // Allocate new string — gc_alloc may trigger a collection, which would
        // invalidate any raw pointer derived from str_obj before this call.
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = result_len;

        // Re-derive src data pointer AFTER gc_alloc to avoid use-after-free.
        // str_obj is a GC root held by the caller, so the object is still live;
        // we just need a fresh pointer into it.
        if result_len > 0 {
            let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
            std::ptr::copy_nonoverlapping(
                src_data.add(start),
                (*new_str).data.as_mut_ptr(),
                result_len,
            );
        }

        obj
    }
}
#[export_name = "rt_str_strip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_strip_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_strip(str_obj.unwrap_ptr()))
}


/// Strip whitespace from left side
/// Returns: new string
pub fn rt_str_lstrip(str_obj: *mut Obj, chars: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;

        // Compute the start offset before gc_alloc; no collection can occur here.
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

        let result_len = src_len - start;

        // Allocate result string — gc_alloc may trigger a collection, which would
        // invalidate src_data. Re-derive it from str_obj (a live GC root) afterwards.
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = result_len;

        if result_len > 0 {
            // Re-derive src_data AFTER gc_alloc to avoid use-after-free.
            let fresh_src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
            std::ptr::copy_nonoverlapping(
                fresh_src_data.add(start),
                (*new_str).data.as_mut_ptr(),
                result_len,
            );
        }

        obj
    }
}
#[export_name = "rt_str_lstrip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_lstrip_abi(str_obj: Value, chars: Value) -> Value {
    Value::from_ptr(rt_str_lstrip(str_obj.unwrap_ptr(), chars.unwrap_ptr()))
}


/// Strip whitespace from right side
/// Returns: new string
pub fn rt_str_rstrip(str_obj: *mut Obj, chars: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;

        // Compute the end offset before gc_alloc; no collection can occur here.
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

        let result_len = end;

        // Allocate result string — gc_alloc may trigger a collection, which would
        // invalidate src_data. Re-derive it from str_obj (a live GC root) afterwards.
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = result_len;

        if result_len > 0 {
            // Re-derive src_data AFTER gc_alloc to avoid use-after-free.
            let fresh_src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
            std::ptr::copy_nonoverlapping(fresh_src_data, (*new_str).data.as_mut_ptr(), result_len);
        }

        obj
    }
}
#[export_name = "rt_str_rstrip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_rstrip_abi(str_obj: Value, chars: Value) -> Value {
    Value::from_ptr(rt_str_rstrip(str_obj.unwrap_ptr(), chars.unwrap_ptr()))
}

