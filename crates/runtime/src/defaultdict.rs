//! defaultdict runtime support
//!
//! defaultdict uses the same `DictObj` struct as regular dicts (identical memory
//! layout and size). The factory tag is packed into the high byte of
//! `entries_capacity` so that no external registry is needed and the tag
//! survives slab-allocator address reuse.
//!
//! Layout:
//!   entries_capacity = (factory_tag as usize) << 56 | real_capacity
//!
//! Because real capacities are always power-of-two values ≤ 2^56 this is safe
//! on any 64-bit platform.  The constants and low-level helpers live in
//! `dict.rs` so that `dict_resize` and `dict_finalize` can also use them.

use crate::boxing::rt_box_float;
use crate::dict::{
    real_entries_capacity, rt_dict_get, rt_dict_set, CAPACITY_MASK, FACTORY_TAG_SHIFT,
};
use crate::object::{DictObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Factory tags for default value creation
const FACTORY_INT: u8 = 0;
const FACTORY_FLOAT: u8 = 1;
const FACTORY_STR: u8 = 2;
const FACTORY_BOOL: u8 = 3;
const FACTORY_LIST: u8 = 4;
const FACTORY_DICT: u8 = 5;
const FACTORY_SET: u8 = 6;
const FACTORY_NONE: u8 = 255; // No factory (acts like regular dict)

/// Extract the factory_tag from the high byte of a DictObj's entries_capacity.
#[inline]
fn get_factory_tag(dict: *mut DictObj) -> u8 {
    unsafe { ((*dict).entries_capacity >> FACTORY_TAG_SHIFT) as u8 }
}

/// Create a new defaultdict with the given capacity and factory tag.
/// Uses DictObj layout — identical to rt_make_dict but with DefaultDict type tag
/// and the factory_tag packed into the high byte of entries_capacity.
pub fn rt_make_defaultdict(capacity: i64, factory_tag: i64) -> *mut Obj {
    let obj = crate::dict::rt_make_dict(capacity);

    unsafe {
        // Change type tag from Dict to DefaultDict
        (*obj).header.type_tag = TypeTagKind::DefaultDict;

        // Pack factory_tag into the high byte of entries_capacity.
        // rt_make_dict already set entries_capacity to the real capacity value.
        let dict = obj as *mut DictObj;
        let real_cap = real_entries_capacity(dict); // identical to raw value before packing
        let tag = if factory_tag < 0 {
            FACTORY_NONE
        } else {
            factory_tag as u8
        };
        (*dict).entries_capacity =
            (real_cap & CAPACITY_MASK) | ((tag as usize) << FACTORY_TAG_SHIFT);
    }

    obj
}
#[export_name = "rt_make_defaultdict"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_defaultdict_abi(capacity: i64, factory_tag: i64) -> Value {
    Value::from_ptr(rt_make_defaultdict(capacity, factory_tag))
}


/// Get a value from defaultdict. If key is missing, creates a default value
/// using the factory, inserts it, and returns it.
pub fn rt_defaultdict_get(dd: *mut Obj, key: *mut Obj) -> *mut Obj {
    if dd.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // First try regular dict get — returns null if not found
        let result = rt_dict_get_or_null(dd as *mut DictObj, key);
        if !result.is_null() {
            return result;
        }

        // Key not found — create default value using factory
        let factory_tag = get_factory_tag(dd as *mut DictObj);

        if factory_tag == FACTORY_NONE {
            // No factory — raise KeyError (same as regular dict)
            return rt_dict_get(dd, key);
        }

        let default_value = create_default_value(factory_tag);

        // Insert default into the dict
        rt_dict_set(dd, key, default_value);

        default_value
    }
}
#[export_name = "rt_defaultdict_get"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_defaultdict_get_abi(dd: Value, key: Value) -> Value {
    Value::from_ptr(rt_defaultdict_get(dd.unwrap_ptr(), key.unwrap_ptr()))
}


/// Try to get a value from dict, returning null if not found (no KeyError).
unsafe fn rt_dict_get_or_null(dict: *mut DictObj, key: *mut Obj) -> *mut Obj {
    use crate::hash_table_utils::{eq_hashable_obj, hash_hashable_obj};

    let cap = (*dict).indices_capacity;
    if cap == 0 || (*dict).len == 0 {
        return std::ptr::null_mut();
    }

    let hash = hash_hashable_obj(key);
    let mask = cap - 1;
    let base = hash as usize;

    for probe in 0..cap {
        let offset = (probe * (probe + 1)) >> 1;
        let slot = (base + offset) & mask;
        let entry_idx = *(*dict).indices.add(slot);

        if entry_idx == -1 {
            return std::ptr::null_mut();
        }
        if entry_idx == -2 {
            continue;
        }
        let entry = (*dict).entries.add(entry_idx as usize);
        if (*entry).hash == hash && eq_hashable_obj((*entry).key.0 as *mut Obj, key) {
            return (*entry).value.0 as *mut Obj;
        }
    }

    std::ptr::null_mut()
}

/// Create a default value based on factory_tag
unsafe fn create_default_value(factory_tag: u8) -> *mut Obj {
    match factory_tag {
        FACTORY_INT => Value::from_int(0).0 as *mut crate::object::Obj,
        FACTORY_FLOAT => rt_box_float(0.0),
        FACTORY_STR => crate::string::rt_make_str(std::ptr::null(), 0),
        FACTORY_BOOL => Value::from_bool(false).0 as *mut crate::object::Obj,
        FACTORY_LIST => crate::list::rt_make_list(0),
        FACTORY_DICT => crate::dict::rt_make_dict(0),
        FACTORY_SET => crate::set::rt_make_set(0),
        _ => crate::boxing::rt_box_none(),
    }
}
