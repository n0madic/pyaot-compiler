//! Tuple comparison operations: eq, lt, lte, gt, gte

use crate::object::{Obj, ELEM_HEAP_OBJ, ELEM_RAW_BOOL, ELEM_RAW_INT};

/// Box a raw value as a heap object based on its elem_tag.
/// If already a heap object (ELEM_HEAP_OBJ), returns as-is.
#[inline]
pub(super) unsafe fn box_if_raw(val: *mut Obj, elem_tag: u8) -> *mut Obj {
    match elem_tag {
        ELEM_RAW_INT => crate::boxing::rt_box_int(val as i64),
        ELEM_RAW_BOOL => crate::boxing::rt_box_bool(val as i8),
        _ => val, // ELEM_HEAP_OBJ or unknown — already a heap pointer
    }
}

/// Compare two heap objects for equality
/// Returns true if equal, false otherwise
/// Both arguments must be valid heap object pointers (or null).
/// Use `box_if_raw` to convert raw values before calling this.
pub(super) unsafe fn compare_heap_objects(a: *mut Obj, b: *mut Obj) -> bool {
    use crate::object::{BoolObj, FloatObj, IntObj, ObjHeader, TypeTagKind};
    use crate::string::rt_str_eq;

    // Both null => equal
    if a.is_null() && b.is_null() {
        return true;
    }
    // One null => not equal
    if a.is_null() || b.is_null() {
        return false;
    }

    // Both are heap objects - safe to dereference
    let header_a = a as *mut ObjHeader;
    let header_b = b as *mut ObjHeader;
    let type_a = (*header_a).type_tag;
    let type_b = (*header_b).type_tag;

    // Different types => check for numeric cross-type equality (CPython: 1 == 1.0 == True)
    if type_a != type_b {
        return crate::hash_table_utils::eq_hashable_obj(a, b);
    }

    match type_a {
        TypeTagKind::Int => {
            let int_a = a as *mut IntObj;
            let int_b = b as *mut IntObj;
            (*int_a).value == (*int_b).value
        }
        TypeTagKind::Float => {
            let float_a = a as *mut FloatObj;
            let float_b = b as *mut FloatObj;
            (*float_a).value == (*float_b).value
        }
        TypeTagKind::Bool => {
            let bool_a = a as *mut BoolObj;
            let bool_b = b as *mut BoolObj;
            (*bool_a).value == (*bool_b).value
        }
        TypeTagKind::Str => rt_str_eq(a, b) == 1,
        TypeTagKind::None => true, // All None values are equal
        TypeTagKind::Tuple => rt_tuple_eq(a, b) == 1, // Recursive for nested tuples
        _ => {
            // For other types, fall back to pointer comparison
            a == b
        }
    }
}

/// Compare two tuples for equality
/// Handles heterogeneous tuples by dispatching based on element type at runtime
/// Returns 1 if equal, 0 if not equal
#[no_mangle]
pub extern "C" fn rt_tuple_eq(a: *mut Obj, b: *mut Obj) -> i8 {
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
        let elem_tag_a = (*tuple_a).elem_tag;
        let elem_tag_b = (*tuple_b).elem_tag;

        // Compare each element
        for i in 0..len {
            let val_a = *data_a.add(i);
            let val_b = *data_b.add(i);

            // Determine element types for comparison
            // If both tuples have same element tag, use optimized path
            if elem_tag_a == elem_tag_b {
                match elem_tag_a {
                    ELEM_RAW_INT => {
                        // Raw integers
                        if val_a as i64 != val_b as i64 {
                            return 0;
                        }
                    }
                    ELEM_RAW_BOOL => {
                        // Raw bools
                        if val_a as i8 != val_b as i8 {
                            return 0;
                        }
                    }
                    ELEM_HEAP_OBJ => {
                        // Heap objects - need runtime type dispatch
                        if !compare_heap_objects(val_a, val_b) {
                            return 0;
                        }
                    }
                    _ => {
                        // Unknown tag, fall back to pointer comparison
                        if val_a != val_b {
                            return 0;
                        }
                    }
                }
            } else {
                // Mixed element tags - box raw values before comparing
                let boxed_a = box_if_raw(val_a, elem_tag_a);
                let boxed_b = box_if_raw(val_b, elem_tag_b);
                if !compare_heap_objects(boxed_a, boxed_b) {
                    return 0;
                }
            }
        }

        1
    }
}

/// Lexicographic ordering comparison for two tuples.
/// Uses elem_tag from each TupleObj to dispatch element comparison.
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
    let elem_tag_a = (*tuple_a).elem_tag;
    let elem_tag_b = (*tuple_b).elem_tag;

    let data_a = (*tuple_a).data.as_ptr();
    let data_b = (*tuple_b).data.as_ptr();

    for i in 0..min_len {
        let elem_a = *data_a.add(i);
        let elem_b = *data_b.add(i);

        let (cmp_a, cmp_b, cmp_tag) = if elem_tag_a == elem_tag_b {
            (elem_a, elem_b, elem_tag_a)
        } else {
            let boxed_a = box_if_raw(elem_a, elem_tag_a);
            let boxed_b = box_if_raw(elem_b, elem_tag_b);
            (boxed_a, boxed_b, crate::object::ELEM_HEAP_OBJ)
        };
        match crate::sorted::compare_list_elements(cmp_a, cmp_b, cmp_tag) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }

    len_a.cmp(&len_b)
}

/// Generic tuple ordering comparison with operation tag.
/// op_tag: 0=Lt, 1=Lte, 2=Gt, 3=Gte
#[no_mangle]
pub extern "C" fn rt_tuple_cmp(a: *mut Obj, b: *mut Obj, op_tag: u8) -> i8 {
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
