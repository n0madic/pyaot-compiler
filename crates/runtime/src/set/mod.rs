//! Set operations for Python runtime

mod algebra;
mod comparison;
mod convert;
mod core;
mod ops;

// Re-export all public functions
pub use algebra::{
    rt_set_difference, rt_set_intersection, rt_set_symmetric_difference, rt_set_union,
};

pub use comparison::{rt_set_isdisjoint, rt_set_issubset, rt_set_issuperset};

pub use convert::rt_set_to_list;

pub use core::{rt_make_set, rt_set_len, rt_set_minmax, rt_set_minmax_with_key, set_finalize};

pub use ops::{
    rt_set_add, rt_set_clear, rt_set_contains, rt_set_copy, rt_set_difference_update,
    rt_set_discard, rt_set_intersection_update, rt_set_pop, rt_set_remove,
    rt_set_symmetric_difference_update, rt_set_update,
};
