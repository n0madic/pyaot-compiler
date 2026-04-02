//! Tuple operations for Python runtime

mod comparison;
mod core;
mod query;

// Re-export all public functions
pub use comparison::{rt_tuple_eq, rt_tuple_gt, rt_tuple_gte, rt_tuple_lt, rt_tuple_lte};

pub use core::{
    rt_call_with_tuple_args, rt_make_tuple, rt_tuple_concat, rt_tuple_from_dict,
    rt_tuple_from_iter, rt_tuple_from_list, rt_tuple_from_range, rt_tuple_from_set,
    rt_tuple_from_str, rt_tuple_get, rt_tuple_get_bool, rt_tuple_get_float, rt_tuple_get_int,
    rt_tuple_len, rt_tuple_set, rt_tuple_set_heap_mask, rt_tuple_slice, rt_tuple_slice_step,
    rt_tuple_slice_to_list,
};

pub use query::{
    rt_tuple_count, rt_tuple_index, rt_tuple_max_float, rt_tuple_max_int,
    rt_tuple_max_with_key, rt_tuple_min_float, rt_tuple_min_int, rt_tuple_min_with_key,
};
