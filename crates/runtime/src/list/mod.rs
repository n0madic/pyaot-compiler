//! List operations for Python runtime
//!
//! `ListObj.data` points at `[Value]`. Every slot is a properly-tagged
//! `Value` (immediate for Int/Bool/None, pointer for heap objects).

mod compare;
mod convert;
mod core;
mod minmax;
mod mutation;
mod query;
mod slice;
mod timsort;

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
