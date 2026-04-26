//! Iterator operations for Python runtime
//!
//! This module provides iterator creation and iteration functions for:
//! - Lists, tuples, dicts, strings, ranges, sets, bytes
//! - Reversed iterators
//! - Composite iterators: zip, map, filter, enumerate

mod composite;
mod factory;
mod next;

// Re-export all public functions
pub(crate) use composite::call_map_with_captures;
pub use composite::{rt_filter_new, rt_map_new, rt_zip3_new, rt_zip_new, rt_zip_next, rt_zipn_new};
pub use factory::{
    rt_iter_bytes, rt_iter_dict, rt_iter_enumerate, rt_iter_generator, rt_iter_list, rt_iter_range,
    rt_iter_reversed_bytes, rt_iter_reversed_dict, rt_iter_reversed_list, rt_iter_reversed_range,
    rt_iter_reversed_str, rt_iter_reversed_tuple, rt_iter_set, rt_iter_str, rt_iter_tuple,
};
pub(crate) use next::rt_iter_next_internal;
pub use next::{rt_iter_is_exhausted, rt_iter_next, rt_iter_next_no_exc};

use crate::object::Obj;

/// Sentinel value indicating iterator exhaustion.
/// Using usize::MAX as sentinel because:
/// 1. It's not a valid pointer (addresses don't go that high)
/// 2. For raw integers, it's an extremely unlikely value in practice
/// 3. Even if a list contains MAX_INT, comparing pointers will use the actual storage value
pub(crate) const EXHAUSTED_SENTINEL: *mut Obj = usize::MAX as *mut Obj;
