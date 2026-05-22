//! Shared min/max utilities for collections (list, tuple, set)

use crate::object::{FloatObj, Obj};
use pyaot_core_defs::Value;

/// Generic integer min/max finder for array-like data.
/// `is_min` = true for min, false for max
///
/// # Safety
/// `data` must be valid for `len` reads.
pub unsafe fn find_extremum_int(data: *const usize, len: usize, is_min: bool) -> i64 {
    if data.is_null() || len == 0 {
        return 0;
    }

    let first = *data;
    let mut extremum = first as i64;

    for i in 1..len {
        let val = *data.add(i) as i64;
        if is_min {
            if val < extremum {
                extremum = val;
            }
        } else if val > extremum {
            extremum = val;
        }
    }

    extremum
}

/// Generic float min/max finder for array of FloatObj pointers.
/// `is_min` = true for min, false for max
///
/// # Safety
/// `data` must be valid for `len` reads, each element must be a valid FloatObj pointer.
pub unsafe fn find_extremum_float(data: *const usize, len: usize, is_min: bool) -> f64 {
    if data.is_null() || len == 0 {
        return 0.0;
    }

    let first_obj = *data as *mut FloatObj;
    let mut extremum = (*first_obj).value;

    for i in 1..len {
        let obj = *data.add(i) as *mut FloatObj;
        let val = (*obj).value;
        if is_min {
            if val < extremum {
                extremum = val;
            }
        } else if val > extremum {
            extremum = val;
        }
    }

    extremum
}

/// Generic tagged min/max finder for a contiguous array of `Value` slots
/// (list / fixed tuple over `Any`-typed elements).
///
/// Each slot is compared via the universal runtime comparator `rt_obj_cmp`,
/// which dispatches on the runtime tag (int / float / bool / str / mixed
/// numeric) — so `int` vs `float` identity is preserved in the returned
/// `Value`. Strict `Lt`/`Gt` keeps the first-seen extremum on ties,
/// matching CPython and the `_with_key` helpers.
///
/// # Safety
/// `data` must be valid for `len` reads; `len >= 1` (callers reject the
/// empty container first).
pub unsafe fn find_extremum_tagged(data: *const Value, len: usize, want_min: bool) -> Value {
    let op_tag: u8 = if want_min { 0 } else { 2 }; // 0 = Lt, 2 = Gt
    let mut extremum = *data;
    for i in 1..len {
        let cand = *data.add(i);
        if crate::ops::rt_obj_cmp(cand.0 as *mut Obj, extremum.0 as *mut Obj, op_tag) != 0 {
            extremum = cand;
        }
    }
    extremum
}
