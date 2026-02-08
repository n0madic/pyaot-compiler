//! Shared min/max utilities for collections (list, tuple, set)

use crate::object::FloatObj;

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
