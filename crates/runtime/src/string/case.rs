//! Case conversion operations: upper, lower, title, capitalize, swapcase

use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};

use super::core::rt_make_str;

/// Convert string to uppercase
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_upper(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Allocate new string
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = len;

        // Copy and convert to uppercase (ASCII only for simplicity)
        let src_data = (*src).data.as_ptr();
        let dst_data = (*new_str).data.as_mut_ptr();
        for i in 0..len {
            let c = *src_data.add(i);
            *dst_data.add(i) = if c.is_ascii_lowercase() { c - 32 } else { c };
        }

        obj
    }
}

/// Convert string to lowercase
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_lower(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Allocate new string
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = len;

        // Copy and convert to lowercase (ASCII only for simplicity)
        let src_data = (*src).data.as_ptr();
        let dst_data = (*new_str).data.as_mut_ptr();
        for i in 0..len {
            let c = *src_data.add(i);
            *dst_data.add(i) = if c.is_ascii_uppercase() { c + 32 } else { c };
        }

        obj
    }
}

/// Title case: first letter of each word capitalized
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_title(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;
        let src_data = (*src).data.as_ptr();

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = len;

        let dst_data = (*result).data.as_mut_ptr();
        let mut word_start = true;

        for i in 0..len {
            let c = *src_data.add(i);
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                *dst_data.add(i) = c;
                word_start = true;
            } else if word_start {
                *dst_data.add(i) = c.to_ascii_uppercase();
                word_start = false;
            } else {
                *dst_data.add(i) = c.to_ascii_lowercase();
            }
        }

        obj
    }
}

/// Capitalize: first character uppercase, rest lowercase
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_capitalize(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;
        let src_data = (*src).data.as_ptr();

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = len;

        let dst_data = (*result).data.as_mut_ptr();

        for i in 0..len {
            let c = *src_data.add(i);
            if i == 0 {
                *dst_data.add(i) = c.to_ascii_uppercase();
            } else {
                *dst_data.add(i) = c.to_ascii_lowercase();
            }
        }

        obj
    }
}

/// Swapcase: swap upper and lower case
/// Returns: new string
#[no_mangle]
pub extern "C" fn rt_str_swapcase(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;
        let src_data = (*src).data.as_ptr();

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = len;

        let dst_data = (*result).data.as_mut_ptr();

        for i in 0..len {
            let c = *src_data.add(i);
            if c.is_ascii_uppercase() {
                *dst_data.add(i) = c.to_ascii_lowercase();
            } else if c.is_ascii_lowercase() {
                *dst_data.add(i) = c.to_ascii_uppercase();
            } else {
                *dst_data.add(i) = c;
            }
        }

        obj
    }
}
