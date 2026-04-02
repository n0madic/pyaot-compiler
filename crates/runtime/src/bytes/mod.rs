//! Bytes operations for Python runtime

mod check;
mod convert;
mod core;
mod search;
mod transform;

// Re-export all public functions
pub use check::{rt_bytes_endswith, rt_bytes_startswith};

pub use convert::{rt_bytes_decode, rt_bytes_fromhex};

pub use core::{
    rt_bytes_concat, rt_bytes_eq, rt_bytes_get, rt_bytes_len, rt_bytes_repeat, rt_bytes_slice,
    rt_bytes_slice_step, rt_make_bytes, rt_make_bytes_from_list, rt_make_bytes_from_str,
    rt_make_bytes_zero,
};

pub use search::{
    rt_bytes_contains, rt_bytes_count, rt_bytes_find, rt_bytes_join, rt_bytes_rfind,
    rt_bytes_rsplit, rt_bytes_search, rt_bytes_split,
};

pub use transform::{
    rt_bytes_lower, rt_bytes_lstrip, rt_bytes_replace, rt_bytes_rstrip, rt_bytes_strip,
    rt_bytes_upper,
};
