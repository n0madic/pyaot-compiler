//! List comparison operations (equality and ordering)

use crate::hash_table_utils::eq_hashable_obj;
use crate::object::{
    BoolObj, FloatObj, IntObj, ListObj, Obj, TypeTagKind, ELEM_HEAP_OBJ, ELEM_RAW_BOOL,
    ELEM_RAW_INT,
};
use std::cmp::Ordering;

/// Semantic element equality across potentially different `elem_tag` storages.
/// Mirrors the tuple mixed-storage handler in `hash_table_utils.rs::eq_hashable_obj`
/// (TypeTagKind::Tuple branch). No allocation — extracts raw values and checks the
/// heap side is a matching `IntObj` / `BoolObj`.
///
/// Fast path `a == b` catches two cases:
///   (1) genuine heap-pointer equality (interned strings, pooled ints, None/True/False).
///   (2) raw-bit equality under a buggy elem_tag — some compiler paths (notably
///       `list(map(closure, iter))`) emit `ELEM_HEAP_OBJ` on the declared list but
///       store raw i64 bits verbatim. Without this fast path, the mixed-tag branch
///       would dereference a raw integer as a pointer and segfault. Keeping the
///       compare robust is preferable to asserting — the upstream mismatch is
///       tracked separately.
#[inline]
unsafe fn list_elem_eq(a: *mut Obj, tag_a: u8, b: *mut Obj, tag_b: u8) -> bool {
    if a == b {
        return true;
    }
    if tag_a == tag_b {
        return match tag_a {
            ELEM_RAW_INT | ELEM_RAW_BOOL => false, // a == b already checked above
            _ => eq_hashable_obj(a, b),
        };
    }
    // Mixed storage. The raw side is always ELEM_RAW_INT or ELEM_RAW_BOOL;
    // the other side is ELEM_HEAP_OBJ (the only non-raw tag).
    let (raw_val, heap_ptr) = match (tag_a, tag_b) {
        (ELEM_RAW_INT, ELEM_HEAP_OBJ) | (ELEM_RAW_BOOL, ELEM_HEAP_OBJ) => (a as i64, b),
        (ELEM_HEAP_OBJ, ELEM_RAW_INT) | (ELEM_HEAP_OBJ, ELEM_RAW_BOOL) => (b as i64, a),
        // Two distinct raw tags (ELEM_RAW_INT vs ELEM_RAW_BOOL) — compare numerically.
        (ELEM_RAW_INT, ELEM_RAW_BOOL) | (ELEM_RAW_BOOL, ELEM_RAW_INT) => {
            return (a as i64) == (b as i64);
        }
        _ => return false,
    };
    if heap_ptr.is_null() {
        return false;
    }
    match (*heap_ptr).type_tag() {
        TypeTagKind::Int => (*(heap_ptr as *mut IntObj)).value == raw_val,
        TypeTagKind::Bool => {
            let bv = if (*(heap_ptr as *mut BoolObj)).value {
                1i64
            } else {
                0
            };
            bv == raw_val
        }
        TypeTagKind::Float => {
            let float_val = (*(heap_ptr as *mut FloatObj)).value;
            float_val.fract() == 0.0 && float_val.is_finite() && float_val as i64 == raw_val
        }
        _ => false,
    }
}

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

        let tag_a = (*(a as *mut ListObj)).elem_tag;
        let tag_b = (*(b as *mut ListObj)).elem_tag;

        if tag_a == tag_b {
            // Fast paths for matched storage.
            if tag_a == ELEM_RAW_INT || tag_a == ELEM_RAW_BOOL {
                for i in 0..len {
                    if (*data_a.add(i) as i64) != (*data_b.add(i) as i64) {
                        return 0;
                    }
                }
            } else {
                for i in 0..len {
                    if !eq_hashable_obj(*data_a.add(i), *data_b.add(i)) {
                        return 0;
                    }
                }
            }
        } else {
            // Mixed storage — dispatch per element.
            for i in 0..len {
                if !list_elem_eq(*data_a.add(i), tag_a, *data_b.add(i), tag_b) {
                    return 0;
                }
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
