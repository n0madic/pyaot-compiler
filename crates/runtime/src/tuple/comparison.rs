//! Tuple comparison operations: eq, lt, lte, gt, gte

use crate::object::Obj;
use pyaot_core_defs::Value;

/// Compare two tuples for equality. After F.7c all slots are uniform tagged Values.
/// Returns 1 if equal, 0 if not equal
pub fn rt_tuple_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    use crate::object::TupleObj;

    // Both null => equal
    if a.is_null() && b.is_null() {
        return 1;
    }
    // One null => not equal
    if a.is_null() || b.is_null() {
        return 0;
    }

    unsafe {
        let tuple_a = a as *mut TupleObj;
        let tuple_b = b as *mut TupleObj;

        // Compare lengths
        if (*tuple_a).len != (*tuple_b).len {
            return 0;
        }

        let len = (*tuple_a).len;

        // Empty tuples are equal
        if len == 0 {
            return 1;
        }

        let data_a = (*tuple_a).data.as_ptr();
        let data_b = (*tuple_b).data.as_ptr();

        // After F.7c: all slots are uniform tagged Values.
        // eq_hashable_obj handles Value-tagged primitives and heap pointers.
        for i in 0..len {
            let va = *data_a.add(i);
            let vb = *data_b.add(i);
            if !crate::hash_table_utils::eq_hashable_obj(va.0 as *mut Obj, vb.0 as *mut Obj) {
                return 0;
            }
        }

        1
    }
}
#[export_name = "rt_tuple_eq"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_eq_abi(a: Value, b: Value) -> i8 {
    rt_tuple_eq(a.unwrap_ptr(), b.unwrap_ptr())
}


/// Lexicographic ordering comparison for two tuples.
/// After F.7c all slots are uniform tagged Values.
unsafe fn tuple_cmp_ordering(a: *mut Obj, b: *mut Obj) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    if a.is_null() && b.is_null() {
        return Ordering::Equal;
    }
    if a.is_null() {
        return Ordering::Less;
    }
    if b.is_null() {
        return Ordering::Greater;
    }

    let tuple_a = a as *mut crate::object::TupleObj;
    let tuple_b = b as *mut crate::object::TupleObj;
    let len_a = (*tuple_a).len;
    let len_b = (*tuple_b).len;
    let min_len = len_a.min(len_b);

    let data_a = (*tuple_a).data.as_ptr();
    let data_b = (*tuple_b).data.as_ptr();

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

/// Generic tuple ordering comparison with operation tag.
/// op_tag: 0=Lt, 1=Lte, 2=Gt, 3=Gte
pub fn rt_tuple_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8 {
    use std::cmp::Ordering;
    let ord = unsafe { tuple_cmp_ordering(a, b) };
    match op_tag {
        0 => (ord == Ordering::Less) as i8,
        1 => (ord != Ordering::Greater) as i8,
        2 => (ord == Ordering::Greater) as i8,
        3 => (ord != Ordering::Less) as i8,
        _ => unreachable!("invalid comparison op_tag: {op_tag}"),
    }
}
#[export_name = "rt_tuple_cmp"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_cmp_abi(a: Value, b: Value, op_tag: u8) -> i8 {
    rt_tuple_cmp(a.unwrap_ptr(), b.unwrap_ptr(), op_tag)
}

