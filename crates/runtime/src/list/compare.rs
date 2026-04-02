//! List comparison operations (equality and ordering)

use crate::object::{FloatObj, ListObj, Obj};
use std::cmp::Ordering;

/// Shared null-check and length comparison for list equality.
/// Returns Some(result) if a quick answer can be given (both null, one null,
/// different lengths, or both empty). Returns None if element-by-element
/// comparison is needed, along with (data_a, data_b, len).
unsafe fn list_eq_precheck(
    a: *mut Obj,
    b: *mut Obj,
) -> Result<i8, (*mut *mut Obj, *mut *mut Obj, usize)> {
    if a.is_null() && b.is_null() {
        return Ok(1);
    }
    if a.is_null() || b.is_null() {
        return Ok(0);
    }

    let list_a = a as *mut ListObj;
    let list_b = b as *mut ListObj;

    if (*list_a).len != (*list_b).len {
        return Ok(0);
    }

    let len = (*list_a).len;
    if len == 0 {
        return Ok(1);
    }

    let data_a = (*list_a).data;
    let data_b = (*list_b).data;

    if data_a.is_null() && data_b.is_null() {
        return Ok(1);
    }
    if data_a.is_null() || data_b.is_null() {
        return Ok(0);
    }

    Err((data_a, data_b, len))
}

/// Compare two lists for equality (integer elements)
/// Returns 1 if equal, 0 if not equal
#[no_mangle]
pub extern "C" fn rt_list_eq_int(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        let (data_a, data_b, len) = match list_eq_precheck(a, b) {
            Ok(result) => return result,
            Err(data) => data,
        };

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
    unsafe {
        let (data_a, data_b, len) = match list_eq_precheck(a, b) {
            Ok(result) => return result,
            Err(data) => data,
        };

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
    unsafe {
        let (data_a, data_b, len) = match list_eq_precheck(a, b) {
            Ok(result) => return result,
            Err(data) => data,
        };

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

/// Lexicographic ordering comparison for two lists.
/// Uses elem_tag from the ListObj to dispatch element comparison.
unsafe fn list_cmp_ordering(a: *mut Obj, b: *mut Obj) -> Ordering {
    if a.is_null() && b.is_null() {
        return Ordering::Equal;
    }
    if a.is_null() {
        return Ordering::Less;
    }
    if b.is_null() {
        return Ordering::Greater;
    }

    let list_a = a as *mut ListObj;
    let list_b = b as *mut ListObj;
    let len_a = (*list_a).len;
    let len_b = (*list_b).len;
    let min_len = len_a.min(len_b);
    let elem_tag = (*list_a).elem_tag;

    let data_a = (*list_a).data;
    let data_b = (*list_b).data;

    for i in 0..min_len {
        let elem_a = *data_a.add(i);
        let elem_b = *data_b.add(i);
        match crate::sorted::compare_list_elements(elem_a, elem_b, elem_tag) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }

    len_a.cmp(&len_b)
}

/// Generic list ordering comparison with operation tag.
/// op_tag: 0=Lt, 1=Lte, 2=Gt, 3=Gte
#[no_mangle]
pub extern "C" fn rt_list_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8 {
    let ord = unsafe { list_cmp_ordering(a, b) };
    match op_tag {
        0 => (ord == Ordering::Less) as i8,
        1 => (ord != Ordering::Greater) as i8,
        2 => (ord == Ordering::Greater) as i8,
        3 => (ord != Ordering::Less) as i8,
        _ => unreachable!("invalid comparison op_tag: {op_tag}"),
    }
}
