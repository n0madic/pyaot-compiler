//! Character predicate operations: isdigit, isalpha, isalnum, isspace, isupper, islower

use crate::object::{Obj, StrObj};

/// Check if all characters are digits
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_str_isdigit(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        if len == 0 {
            return 0;
        }

        let data = (*src).data.as_ptr();
        for i in 0..len {
            if !(*data.add(i)).is_ascii_digit() {
                return 0;
            }
        }
        1
    }
}

/// Check if all characters are alphabetic
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_str_isalpha(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        if len == 0 {
            return 0;
        }

        let data = (*src).data.as_ptr();
        for i in 0..len {
            if !(*data.add(i)).is_ascii_alphabetic() {
                return 0;
            }
        }
        1
    }
}

/// Check if all characters are alphanumeric
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_str_isalnum(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        if len == 0 {
            return 0;
        }

        let data = (*src).data.as_ptr();
        for i in 0..len {
            if !(*data.add(i)).is_ascii_alphanumeric() {
                return 0;
            }
        }
        1
    }
}

/// Check if all characters are whitespace
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_str_isspace(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        if len == 0 {
            return 0;
        }

        let data = (*src).data.as_ptr();
        for i in 0..len {
            let c = *data.add(i);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' && c != b'\x0c' && c != b'\x0b' {
                return 0;
            }
        }
        1
    }
}

/// Check if all cased characters are uppercase
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_str_isupper(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        if len == 0 {
            return 0;
        }

        let data = (*src).data.as_ptr();
        let mut has_cased = false;
        for i in 0..len {
            let c = *data.add(i);
            if c.is_ascii_lowercase() {
                return 0;
            }
            if c.is_ascii_uppercase() {
                has_cased = true;
            }
        }
        if has_cased {
            1
        } else {
            0
        }
    }
}

/// Check if all cased characters are lowercase
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_str_islower(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        if len == 0 {
            return 0;
        }

        let data = (*src).data.as_ptr();
        let mut has_cased = false;
        for i in 0..len {
            let c = *data.add(i);
            if c.is_ascii_uppercase() {
                return 0;
            }
            if c.is_ascii_lowercase() {
                has_cased = true;
            }
        }
        if has_cased {
            1
        } else {
            0
        }
    }
}

/// Check if all characters are ASCII (code points < 128)
/// Returns: 1 (true) or 0 (false)
/// Empty string returns 1 (Python behavior)
#[no_mangle]
pub extern "C" fn rt_str_isascii(str_obj: *mut Obj) -> i8 {
    if str_obj.is_null() {
        return 0;
    }

    unsafe {
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
