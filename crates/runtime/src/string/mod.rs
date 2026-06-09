//! String operations for Python runtime
//!
//! This module provides string manipulation functions for the Python AOT compiler runtime.
//! Functions are organized into submodules by functionality.

mod align;
pub mod builder;
mod case;
mod core;
pub mod interning;
mod modify;
mod predicates;
mod search;
pub mod slice;
mod split_join;
mod trim;

// Re-export all public functions
pub use align::{rt_str_center, rt_str_ljust, rt_str_rjust, rt_str_zfill};
pub use case::{rt_str_capitalize, rt_str_lower, rt_str_swapcase, rt_str_title, rt_str_upper};
pub use core::{
    rt_make_str, rt_make_str_impl, rt_str_concat, rt_str_data, rt_str_encode, rt_str_len,
    rt_str_len_int,
};
pub use modify::{rt_str_mul, rt_str_replace};
pub use predicates::{
    rt_str_isalnum, rt_str_isalpha, rt_str_isascii, rt_str_isdigit, rt_str_islower, rt_str_isspace,
    rt_str_isupper,
};
pub use search::{
    rt_str_contains, rt_str_count, rt_str_endswith, rt_str_eq, rt_str_find, rt_str_rfind,
    rt_str_search, rt_str_startswith,
};
pub(crate) use slice::utf8_char_width;
pub use slice::{rt_str_getchar, rt_str_slice, rt_str_slice_step};
pub use split_join::{rt_str_join, rt_str_rsplit, rt_str_split};
pub use trim::{rt_str_lstrip, rt_str_rstrip, rt_str_strip};

// Re-export interning functions
pub use interning::{
    init_string_pool, prune_string_pool, rt_make_str_interned, shutdown_string_pool,
};

// Re-export builder functions
pub use builder::{
    rt_make_string_builder, rt_string_builder_append, rt_string_builder_to_str,
    string_builder_finalize,
};
