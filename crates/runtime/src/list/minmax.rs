//! List min/max operations

use super::load_value_as_raw;
use crate::minmax_utils::find_extremum_float;
use crate::object::{ListObj, Obj};
use crate::sorted::compare_key_values;

/// Generic list min/max for int and float elements.
/// is_min: 0=min, 1=max; elem_kind: 0=int, 1=float.
/// Returns i64 (for float, result is f64::to_bits()).
#[no_mangle]
pub extern "C" fn rt_list_minmax(list: *mut Obj, is_min: u8, elem_kind: u8) -> i64 {
    if list.is_null() {
        return 0;
    }
    unsafe {
        let list_obj = list as *mut ListObj;
        if (*list_obj).len == 0 {
            if is_min == 0 {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "min() arg is an empty sequence"
                );
            } else {
                raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "max() arg is an empty sequence"
                );
            }
        }
        let data = (*list_obj).data;
        let len = (*list_obj).len;
        let elem_tag = (*list_obj).elem_tag;
        let want_min = is_min == 0;
        if elem_kind == 1 {
            // Float: `data` points at a `[Value]` whose heap slots wrap
            // `*mut FloatObj`. Since `Value::from_ptr` preserves aligned
            // pointer bits verbatim, casting to `*const usize` still yields
            // valid FloatObj pointers, and `find_extremum_float` works
            // unchanged. (Raw-float lists don't exist — floats are always
            // heap-boxed per §2.2.)
            find_extremum_float(data as *const usize, len, want_min).to_bits() as i64
        } else {
            // Int: walk the `[Value]` slots directly, converting each slot
            // back to its ABI `i64` form via `load_value_as_raw` so the
            // logic handles both ELEM_RAW_INT (tagged immediate) and
            // ELEM_HEAP_OBJ (pointer to IntObj — though the typed caller
            // normally picks `rt_list_get_typed`, we handle both defensively).
            if len == 0 {
                return 0;
            }
            let mut extremum = load_value_as_raw(*data, elem_tag) as i64;
            for i in 1..len {
                let val = load_value_as_raw(*data.add(i), elem_tag) as i64;
                if want_min {
                    if val < extremum {
                        extremum = val;
                    }
                } else if val > extremum {
                    extremum = val;
                }
            }
            extremum
        }
    }
}

/// Generic list min/max with key function.
/// is_min: 0=min, 1=max
#[no_mangle]
pub extern "C" fn rt_list_minmax_with_key(
    list: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: i64,
    is_min: u8,
) -> *mut Obj {
    unsafe {
        find_extremum_with_key(
            list,
            key_fn,
            elem_tag,
            captures,
            capture_count as u8,
            is_min == 0,
        )
    }
}

/// Call key function with captures support
unsafe fn call_key_fn(
    key_fn: i64,
    captures: *mut Obj,
    capture_count: u8,
    elem: *mut Obj,
) -> *mut Obj {
    crate::iterator::call_map_with_captures(key_fn, captures, capture_count, elem)
}

/// Find extremum (min or max) element in a list using a key function
unsafe fn find_extremum_with_key(
    list: *mut Obj,
    key_fn: i64,
    elem_tag: i64,
    captures: *mut Obj,
    capture_count: u8,
    is_min: bool,
) -> *mut Obj {
    use crate::object::ELEM_RAW_INT;

    if list.is_null() {
        return std::ptr::null_mut();
    }

    let list_obj = list as *mut ListObj;
    let len = (*list_obj).len;

    if len == 0 {
        if is_min {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "min() arg is an empty sequence"
            );
        } else {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "max() arg is an empty sequence"
            );
        }
    }

    let data = (*list_obj).data;
    // Storage unwrapping always uses the list's own `elem_tag`. The
    // `elem_tag` parameter that codegen passes is a key-function hint
    // (whether to box before calling key_fn) and may disagree with the
    // real storage tag — for example, `min([int], key=identity)` passes
    // ELEM_HEAP_OBJ as a "don't box" hint while the list stores raw ints.
    let storage_tag = (*list_obj).elem_tag;

    // Apply key function to first element.
    let mut extremum_elem = load_value_as_raw(*data, storage_tag);
    let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
        crate::boxing::rt_box_int(extremum_elem as i64)
    } else {
        extremum_elem
    };
    let mut extremum_key = call_key_fn(key_fn, captures, capture_count, boxed_elem);

    // Compare remaining elements
    for i in 1..len {
        let elem = load_value_as_raw(*data.add(i), storage_tag);
        let boxed_elem = if elem_tag == ELEM_RAW_INT as i64 {
            crate::boxing::rt_box_int(elem as i64)
        } else {
            elem
        };
        let key = call_key_fn(key_fn, captures, capture_count, boxed_elem);

        let cmp = compare_key_values(key, extremum_key);
        let is_better = if is_min {
            cmp == std::cmp::Ordering::Less
        } else {
            cmp == std::cmp::Ordering::Greater
        };

        if is_better {
            extremum_elem = elem;
            extremum_key = key;
        }
    }

    extremum_elem // Return original element (in raw ABI form), not key!
}
