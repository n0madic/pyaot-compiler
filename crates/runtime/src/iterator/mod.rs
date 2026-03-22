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
pub use composite::{rt_filter_new, rt_map_new, rt_zip3_new, rt_zip_new, rt_zip_next, rt_zipn_new};
pub use factory::{
    rt_iter_bytes, rt_iter_dict, rt_iter_enumerate, rt_iter_generator, rt_iter_list, rt_iter_range,
    rt_iter_reversed_bytes, rt_iter_reversed_dict, rt_iter_reversed_list, rt_iter_reversed_range,
    rt_iter_reversed_str, rt_iter_reversed_tuple, rt_iter_set, rt_iter_str, rt_iter_tuple,
};
pub(crate) use next::rt_iter_next_internal;
pub use next::{rt_iter_is_exhausted, rt_iter_next, rt_iter_next_no_exc};

use crate::object::{Obj, TypeTagKind};

/// Sentinel value indicating iterator exhaustion.
/// Using usize::MAX as sentinel because:
/// 1. It's not a valid pointer (addresses don't go that high)
/// 2. For raw integers, it's an extremely unlikely value in practice
/// 3. Even if a list contains MAX_INT, comparing pointers will use the actual storage value
pub(crate) const EXHAUSTED_SENTINEL: *mut Obj = usize::MAX as *mut Obj;

/// Helper to box an element if it came from a raw-value iterator (list[int], tuple[int,...], range, bytes)
/// Returns the element as-is if it's already a heap object, or boxes it if raw
pub(crate) unsafe fn box_if_raw_int_iterator(iter: *mut Obj, elem: *mut Obj) -> *mut Obj {
    use crate::boxing::rt_box_int;
    use crate::object::{IteratorKind, IteratorObj, ListObj, TupleObj, ELEM_RAW_INT};

    if iter.is_null() {
        return elem;
    }

    let type_tag = (*iter).header.type_tag;

    if type_tag == TypeTagKind::Iterator {
        let iter_obj = iter as *mut IteratorObj;
        let kind = IteratorKind::try_from((*iter_obj).kind)
            .expect("box_if_raw_int_iterator: invalid iterator kind");

        match kind {
            IteratorKind::Range | IteratorKind::Bytes => {
                // These always yield raw integers
                return rt_box_int(elem as i64);
            }
            IteratorKind::List => {
                // Check list's elem_tag
                let list = (*iter_obj).source as *mut ListObj;
                if (*list).elem_tag == ELEM_RAW_INT {
                    return rt_box_int(elem as i64);
                }
            }
            IteratorKind::Tuple => {
                // Check tuple's elem_tag
                let tuple = (*iter_obj).source as *mut TupleObj;
                if (*tuple).elem_tag == ELEM_RAW_INT {
                    return rt_box_int(elem as i64);
                }
            }
            _ => {
                // Other iterator types (Map, Filter, etc.) yield heap objects
            }
        }
    }

    elem
}
