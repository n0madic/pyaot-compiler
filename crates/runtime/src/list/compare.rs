//! List comparison operations (equality and ordering)

use crate::hash_table_utils::eq_hashable_obj;
use crate::object::{ListObj, Obj, ELEM_RAW_BOOL, ELEM_RAW_INT};
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

/// Compare two lists for equality using the list's elem_tag to dispatch
/// element comparison. Replaces rt_list_eq_int/float/str.
/// Returns 1 if equal, 0 if not equal.
#[no_mangle]
pub extern "C" fn rt_list_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        let (data_a, data_b, len) = match list_eq_precheck(a, b) {
            Ok(result) => return result,
            Err(data) => data,
        };

        let elem_tag = (*(a as *mut ListObj)).elem_tag;

        if elem_tag == ELEM_RAW_INT || elem_tag == ELEM_RAW_BOOL {
            // Raw integer/bool elements — compare as i64
            for i in 0..len {
                let val_a = *data_a.add(i) as i64;
                let val_b = *data_b.add(i) as i64;
                if val_a != val_b {
                    return 0;
                }
            }
        } else {
            // Heap objects — use eq_hashable_obj for proper value equality
            // Handles floats (FloatObj), strings (StrObj), and any other heap type
            for i in 0..len {
                let obj_a = *data_a.add(i);
                let obj_b = *data_b.add(i);
                if !eq_hashable_obj(obj_a, obj_b) {
                    return 0;
                }
            }
        }

        1
    }
}

/// Compare two lists for equality (integer elements) — thin wrapper for backward compat
#[no_mangle]
pub extern "C" fn rt_list_eq_int(a: *mut Obj, b: *mut Obj) -> i8 {
    rt_list_eq(a, b)
}

/// Compare two lists for equality (float elements) — thin wrapper for backward compat
#[no_mangle]
pub extern "C" fn rt_list_eq_float(a: *mut Obj, b: *mut Obj) -> i8 {
    rt_list_eq(a, b)
}

/// Compare two lists for equality (string elements) — thin wrapper for backward compat
#[no_mangle]
pub extern "C" fn rt_list_eq_str(a: *mut Obj, b: *mut Obj) -> i8 {
    rt_list_eq(a, b)
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
