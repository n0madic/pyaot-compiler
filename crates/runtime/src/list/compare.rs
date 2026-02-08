//! List equality comparison operations

use crate::object::{FloatObj, ListObj, Obj};

/// Compare two lists for equality (integer elements)
/// Returns 1 if equal, 0 if not equal
#[no_mangle]
pub extern "C" fn rt_list_eq_int(a: *mut Obj, b: *mut Obj) -> i8 {
    if a.is_null() && b.is_null() {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }

    unsafe {
        let list_a = a as *mut ListObj;
        let list_b = b as *mut ListObj;

        // Compare lengths
        if (*list_a).len != (*list_b).len {
            return 0;
        }

        let len = (*list_a).len;

        // If both lists are empty, they are equal
        if len == 0 {
            return 1;
        }

        let data_a = (*list_a).data;
        let data_b = (*list_b).data;

        if data_a.is_null() && data_b.is_null() {
            return 1;
        }
        if data_a.is_null() || data_b.is_null() {
            return 0;
        }

        // Compare elements
        for i in 0..len {
            let val_a = *data_a.add(i) as i64;
            let val_b = *data_b.add(i) as i64;
            if val_a != val_b {
                return 0;
            }
        }

        1
    }
}

/// Compare two lists for equality (float elements)
/// Returns 1 if equal, 0 if not equal
#[no_mangle]
pub extern "C" fn rt_list_eq_float(a: *mut Obj, b: *mut Obj) -> i8 {
    if a.is_null() && b.is_null() {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }

    unsafe {
        let list_a = a as *mut ListObj;
        let list_b = b as *mut ListObj;

        // Compare lengths
        if (*list_a).len != (*list_b).len {
            return 0;
        }

        let len = (*list_a).len;

        // If both lists are empty, they are equal
        if len == 0 {
            return 1;
        }

        let data_a = (*list_a).data;
        let data_b = (*list_b).data;

        if data_a.is_null() && data_b.is_null() {
            return 1;
        }
        if data_a.is_null() || data_b.is_null() {
            return 0;
        }

        // Compare elements (float elements are boxed FloatObj pointers)
        for i in 0..len {
            let obj_a = *data_a.add(i) as *mut FloatObj;
            let obj_b = *data_b.add(i) as *mut FloatObj;
            let val_a = (*obj_a).value;
            let val_b = (*obj_b).value;
            if val_a != val_b {
                return 0;
            }
        }

        1
    }
}

/// Compare two lists for equality (string elements)
/// Returns 1 if equal, 0 if not equal
#[no_mangle]
pub extern "C" fn rt_list_eq_str(a: *mut Obj, b: *mut Obj) -> i8 {
    if a.is_null() && b.is_null() {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }

    unsafe {
        let list_a = a as *mut ListObj;
        let list_b = b as *mut ListObj;

        // Compare lengths
        if (*list_a).len != (*list_b).len {
            return 0;
        }

        let len = (*list_a).len;

        // If both lists are empty, they are equal
        if len == 0 {
            return 1;
        }

        let data_a = (*list_a).data;
        let data_b = (*list_b).data;

        if data_a.is_null() && data_b.is_null() {
            return 1;
        }
        if data_a.is_null() || data_b.is_null() {
            return 0;
        }

        // Compare elements (string elements are StrObj pointers)
        for i in 0..len {
            let str_a = *data_a.add(i);
            let str_b = *data_b.add(i);
            if crate::string::rt_str_eq(str_a, str_b) == 0 {
                return 0;
            }
        }

        1
    }
}
