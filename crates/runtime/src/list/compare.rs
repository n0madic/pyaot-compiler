//! List comparison operations (equality and ordering)

use crate::hash_table_utils::eq_hashable_obj;
use crate::object::{ListObj, Obj};
use pyaot_core_defs::Value;
use std::cmp::Ordering;

/// Shared null-check and length comparison for list equality.
/// Returns Some(result) if a quick answer can be given (both null, one null,
/// different lengths, or both empty). Returns None if element-by-element
/// comparison is needed, along with (data_a, data_b, len).
unsafe fn list_eq_precheck(
    a: *mut Obj,
    b: *mut Obj,
) -> Result<i8, (*mut Value, *mut Value, usize)> {
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

/// Compare two lists for equality. After F.7c all slots are uniform tagged Values.
/// Returns 1 if equal, 0 if not equal.
pub fn rt_list_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    unsafe {
        let (data_a, data_b, len) = match list_eq_precheck(a, b) {
            Ok(result) => return result,
            Err(data) => data,
        };

        for i in 0..len {
            let a_raw = (*data_a.add(i)).0 as *mut Obj;
            let b_raw = (*data_b.add(i)).0 as *mut Obj;
            if !eq_hashable_obj(a_raw, b_raw) {
                return 0;
            }
        }

        1
    }
}
#[export_name = "rt_list_eq"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_eq_abi(a: Value, b: Value) -> i8 {
    rt_list_eq(a.unwrap_ptr(), b.unwrap_ptr())
}

/// Lexicographic ordering comparison for two lists.
/// After F.7c all slots are uniform tagged Values.
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

    let data_a = (*list_a).data;
    let data_b = (*list_b).data;

    for i in 0..min_len {
        let elem_a = (*data_a.add(i)).0 as *mut Obj;
        let elem_b = (*data_b.add(i)).0 as *mut Obj;
        match crate::sorted::compare_list_elements(elem_a, elem_b) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }

    len_a.cmp(&len_b)
}

/// Generic list ordering comparison with operation tag.
/// op_tag: 0=Lt, 1=Lte, 2=Gt, 3=Gte
pub fn rt_list_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8 {
    let ord = unsafe { list_cmp_ordering(a, b) };
    match op_tag {
        0 => (ord == Ordering::Less) as i8,
        1 => (ord != Ordering::Greater) as i8,
        2 => (ord == Ordering::Greater) as i8,
        3 => (ord != Ordering::Less) as i8,
        _ => unreachable!("invalid comparison op_tag: {op_tag}"),
    }
}
#[export_name = "rt_list_cmp"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_cmp_abi(a: Value, b: Value, op_tag: u8) -> i8 {
    rt_list_cmp(a.unwrap_ptr(), b.unwrap_ptr(), op_tag)
}
