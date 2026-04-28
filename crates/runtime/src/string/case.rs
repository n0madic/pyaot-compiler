//! Case conversion operations: upper, lower, title, capitalize, swapcase

use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

use super::core::rt_make_str;

/// Convert string to uppercase
/// Returns: pointer to new allocated StrObj
pub fn rt_str_upper(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Allocate new string first — gc_alloc may trigger collection
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = len;

        // Re-derive src_data AFTER gc_alloc to avoid use-after-free if src was moved
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
        let dst_data = (*new_str).data.as_mut_ptr();
        for i in 0..len {
            let c = *src_data.add(i);
            *dst_data.add(i) = if c.is_ascii_lowercase() { c - 32 } else { c };
        }

        obj
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
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Allocate new string first — gc_alloc may trigger collection
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = len;

        // Re-derive src_data AFTER gc_alloc to avoid use-after-free if src was moved
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
        let dst_data = (*new_str).data.as_mut_ptr();
        for i in 0..len {
            let c = *src_data.add(i);
            *dst_data.add(i) = if c.is_ascii_uppercase() { c + 32 } else { c };
        }

        obj
    }
}
#[export_name = "rt_str_lower"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_lower_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_lower(str_obj.unwrap_ptr()))
}


/// Title case: first letter of each word capitalized
/// Returns: new string
pub fn rt_str_title(str_obj: *mut Obj) -> *mut Obj {
    if str_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = len;

        // Re-derive src_data AFTER gc_alloc to avoid use-after-free
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
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
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = len;

        // Re-derive src_data AFTER gc_alloc to avoid use-after-free
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
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
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = len;

        // Re-derive src_data AFTER gc_alloc to avoid use-after-free
        let src_data = (*(str_obj as *mut StrObj)).data.as_ptr();
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
#[export_name = "rt_str_swapcase"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_swapcase_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_str_swapcase(str_obj.unwrap_ptr()))
}

