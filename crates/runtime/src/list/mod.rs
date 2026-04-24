//! List operations for Python runtime
//!
//! This module provides list creation, manipulation, and conversion functions.
//!
//! # Phase 2 S2.3: Value-backed storage
//!
//! `ListObj.data` now points at `[Value]`. Every slot is a properly-tagged
//! `Value` (immediate for Int/Bool/None, pointer for heap). Internal ops work
//! directly in terms of `Value`; GC marks list elements via `Value::is_ptr()`
//! with no `elem_tag` branching. The extern ABI still speaks `*mut Obj`/raw
//! scalars to stay compatible with codegen — this module converts on both
//! sides of the boundary using [`store_raw_as_value`] / [`load_value_as_raw`].
//! S2.7 will collapse the boundary when codegen emits `Value` natively.

use pyaot_core_defs::Value;

use crate::object::{Obj, ELEM_RAW_BOOL, ELEM_RAW_INT};

/// Calculate new list capacity using CPython's growth formula.
/// This provides ~12.5% growth for large lists instead of 100% (doubling),
/// reducing memory waste by ~50% for large lists.
#[inline]
pub(crate) fn list_grow_capacity(capacity: usize) -> usize {
    if capacity == 0 {
        4
    } else if capacity < 9 {
        capacity + 3
    } else {
        // ~12.5% growth: capacity + capacity/8 + 6
        capacity + (capacity >> 3) + 6
    }
}

/// Convert an incoming ABI `*mut Obj` to the `Value` we store in the list
/// data array. `elem_tag` disambiguates the three physical layouts codegen
/// still emits today: raw `i64` encoded as `*mut Obj` for `ELEM_RAW_INT`,
/// raw `i8` for `ELEM_RAW_BOOL`, proper heap pointer for `ELEM_HEAP_OBJ`.
#[inline]
pub(crate) fn store_raw_as_value(raw: *mut Obj, elem_tag: u8) -> Value {
    match elem_tag {
        ELEM_RAW_INT => Value::from_int(raw as i64),
        ELEM_RAW_BOOL => Value::from_bool((raw as usize) != 0),
        // ELEM_HEAP_OBJ or unknown: treat as pointer (null is a valid pointer).
        _ => Value::from_ptr(raw),
    }
}

/// Inverse of [`store_raw_as_value`]: convert a stored `Value` back to the
/// `*mut Obj` the extern ABI expects, using `elem_tag` to choose the raw
/// scalar encoding for `ELEM_RAW_INT` / `ELEM_RAW_BOOL`.
#[inline]
pub fn load_value_as_raw(v: Value, elem_tag: u8) -> *mut Obj {
    match elem_tag {
        ELEM_RAW_INT => v.unwrap_int() as *mut Obj,
        ELEM_RAW_BOOL => i64::from(v.unwrap_bool()) as *mut Obj,
        _ => {
            // ELEM_HEAP_OBJ or unknown: return raw pointer bits (may be null).
            // We don't call `unwrap_ptr` here because its debug assertion
            // rejects non-pointer tags, and the caller is expected to treat
            // the result as opaque `*mut Obj` regardless of layout.
            v.0 as *mut Obj
        }
    }
}

/// Read a single slot from a list as the raw ABI `*mut Obj`.
///
/// Cross-module consumers that still expect the pre-S2.3 pointer layout
/// should call this instead of dereferencing `(*list).data.add(i)` directly.
/// Equivalent to `load_value_as_raw((*list).data[i], (*list).elem_tag)`.
///
/// # Safety
///
/// The caller must guarantee `list` points at a live `ListObj`, `i` is
/// within bounds (`i < (*list).len`), and the data array is non-null.
#[inline]
pub unsafe fn list_slot_raw(list: *mut crate::object::ListObj, i: usize) -> *mut Obj {
    load_value_as_raw(*(*list).data.add(i), (*list).elem_tag)
}

mod compare;
mod convert;
mod core;
mod minmax;
mod mutation;
mod query;
mod slice;
mod timsort;

// Re-export all public functions
pub use compare::{rt_list_cmp, rt_list_eq};
pub use convert::{
    rt_list_from_dict, rt_list_from_iter, rt_list_from_range, rt_list_from_set, rt_list_from_str,
    rt_list_from_tuple, rt_list_tail_to_tuple, rt_list_tail_to_tuple_bool,
    rt_list_tail_to_tuple_float,
};
pub use core::{
    list_finalize, rt_list_get, rt_list_get_typed, rt_list_len, rt_list_push, rt_list_set,
    rt_make_list,
};
pub use minmax::{rt_list_minmax, rt_list_minmax_with_key};
pub use mutation::{
    rt_list_append, rt_list_clear, rt_list_extend, rt_list_insert, rt_list_pop, rt_list_remove,
    rt_list_reverse, rt_list_sort,
};
pub use query::{rt_list_copy, rt_list_count, rt_list_index};
pub use slice::{rt_list_slice, rt_list_slice_step};
