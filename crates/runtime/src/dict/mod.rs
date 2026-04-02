//! Dictionary operations for Python runtime
//!
//! Uses CPython 3.6+ compact dict design to preserve insertion order:
//! - `indices`: hash index table mapping hash slots to entry indices
//! - `entries`: dense array of DictEntry stored in insertion order

mod core;
mod iteration;
mod ops;

// Re-export all public functions
pub use core::{dict_finalize, rt_dict_len, rt_make_dict};
pub(crate) use core::{
    real_entries_capacity, set_real_entries_capacity, CAPACITY_MASK, FACTORY_TAG_SHIFT,
};

pub use iteration::{rt_dict_items, rt_dict_keys, rt_dict_values};

pub use ops::{
    rt_dict_clear, rt_dict_contains, rt_dict_copy, rt_dict_from_pairs, rt_dict_fromkeys,
    rt_dict_get, rt_dict_get_default, rt_dict_merge, rt_dict_pop, rt_dict_popitem,
    rt_dict_set, rt_dict_setdefault, rt_dict_update,
};
