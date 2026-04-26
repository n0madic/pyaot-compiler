//! Tuple operations for Python runtime

mod comparison;
mod core;
mod query;

// Re-export all public functions
pub use comparison::{rt_tuple_cmp, rt_tuple_eq};

pub use core::{
    rt_call_with_tuple_args, rt_make_tuple, rt_tuple_concat, rt_tuple_from_dict,
    rt_tuple_from_iter, rt_tuple_from_list, rt_tuple_from_range, rt_tuple_from_set,
    rt_tuple_from_str, rt_tuple_get, rt_tuple_len, rt_tuple_set, rt_tuple_slice,
    rt_tuple_slice_step, rt_tuple_slice_to_list,
};

pub use query::{rt_tuple_count, rt_tuple_index, rt_tuple_minmax, rt_tuple_minmax_with_key};
