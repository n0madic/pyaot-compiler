//! defaultdict runtime support
//!
//! defaultdict uses the same `DictObj` struct as regular dicts (identical memory
//! layout and size). The factory tag is stored in a separate static registry
//! keyed by object pointer. This ensures defaultdict objects go through the same
//! slab allocation path as regular dicts, so all dict operations (keys, values,
//! items, resize, finalize) work correctly.

use crate::boxing::{rt_box_float, rt_box_int};
use crate::dict::{rt_dict_get, rt_dict_set};
use crate::object::{DictObj, Obj, TypeTagKind};
use std::cell::UnsafeCell;
use std::collections::HashMap;

/// Factory tags for default value creation
const FACTORY_INT: u8 = 0;
const FACTORY_FLOAT: u8 = 1;
const FACTORY_STR: u8 = 2;
const FACTORY_BOOL: u8 = 3;
const FACTORY_LIST: u8 = 4;
const FACTORY_DICT: u8 = 5;
const FACTORY_SET: u8 = 6;
const FACTORY_NONE: u8 = 255; // No factory (acts like regular dict)

/// Lock-free registry mapping defaultdict pointers to their factory tags.
struct FactoryRegistry(UnsafeCell<Option<HashMap<usize, u8>>>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for FactoryRegistry {}

static FACTORY_REGISTRY: FactoryRegistry = FactoryRegistry(UnsafeCell::new(None));

fn get_registry() -> &'static mut HashMap<usize, u8> {
    unsafe {
        let reg = &mut *FACTORY_REGISTRY.0.get();
        reg.get_or_insert_with(HashMap::new)
    }
}

/// Store factory_tag for a defaultdict object
fn set_factory_tag(obj: *mut Obj, tag: u8) {
    get_registry().insert(obj as usize, tag);
}

/// Get factory_tag for a defaultdict object (returns FACTORY_NONE if not found)
fn get_factory_tag(obj: *mut Obj) -> u8 {
    get_registry()
        .get(&(obj as usize))
        .copied()
        .unwrap_or(FACTORY_NONE)
}

/// Remove factory_tag entry when object is finalized
pub fn remove_factory_tag(obj: *mut Obj) {
    get_registry().remove(&(obj as usize));
}

/// Create a new defaultdict with the given capacity and factory tag.
/// Uses DictObj layout — identical to rt_make_dict but with DefaultDict type tag.
#[no_mangle]
pub extern "C" fn rt_make_defaultdict(capacity: i64, factory_tag: i64) -> *mut Obj {
    let obj = crate::dict::rt_make_dict(capacity);

    unsafe {
        // Change type tag from Dict to DefaultDict
        (*obj).header.type_tag = TypeTagKind::DefaultDict;
    }

    // Store factory tag in registry
    let tag = if factory_tag < 0 {
        FACTORY_NONE
    } else {
        factory_tag as u8
    };
    set_factory_tag(obj, tag);

    obj
}

/// Get a value from defaultdict. If key is missing, creates a default value
/// using the factory, inserts it, and returns it.
#[no_mangle]
pub extern "C" fn rt_defaultdict_get(dd: *mut Obj, key: *mut Obj) -> *mut Obj {
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
        let factory_tag = get_factory_tag(dd);

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
        if (*entry).hash == hash && eq_hashable_obj((*entry).key, key) {
            return (*entry).value;
        }
    }

    std::ptr::null_mut()
}

/// Create a default value based on factory_tag
unsafe fn create_default_value(factory_tag: u8) -> *mut Obj {
    match factory_tag {
        FACTORY_INT => rt_box_int(0),
        FACTORY_FLOAT => rt_box_float(0.0),
        FACTORY_STR => crate::string::rt_make_str(std::ptr::null(), 0),
        FACTORY_BOOL => crate::boxing::rt_box_bool(0),
        FACTORY_LIST => crate::list::rt_make_list(0, 0),
        FACTORY_DICT => crate::dict::rt_make_dict(0),
        FACTORY_SET => crate::set::rt_make_set(0),
        _ => crate::boxing::rt_box_none(),
    }
}
