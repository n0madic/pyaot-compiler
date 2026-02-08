//! Class attribute storage for Python class-level variables
//!
//! This module provides typed key-value storage for class attributes.
//! Attributes are indexed by (class_id: u8, attr_idx: u32) and can store
//! any primitive type or heap object pointer.

use crate::object::Obj;
use std::collections::HashMap;
use std::sync::Mutex;

/// Type tag for class attribute entries
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassAttrTag {
    Int = 0,
    Float = 1,
    Bool = 2,
    Ptr = 3, // Pointer to heap object (str, list, dict, etc.)
}

/// Entry for a class attribute with type information
#[derive(Debug, Clone, Copy)]
pub struct ClassAttrEntry {
    pub tag: ClassAttrTag,
    pub value: i64, // Stores int, float bits, bool, or pointer
}

/// Class attribute storage - maps (class_id, attr_idx) to typed ClassAttrEntry
static CLASS_ATTRS: Mutex<Option<HashMap<(u8, u32), ClassAttrEntry>>> = Mutex::new(None);

/// Initialize class attribute storage (called by rt_init)
pub fn init_class_attrs() {
    let mut attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    *attrs = Some(HashMap::new());
}

/// Shutdown class attribute storage (called by rt_shutdown)
pub fn shutdown_class_attrs() {
    let mut attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    *attrs = None;
}

// ==================== Type-specific Integer API ====================

/// Set a class attribute integer
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// value: the integer value to store (i64)
#[no_mangle]
pub extern "C" fn rt_class_attr_set_int(class_id: u8, attr_idx: u32, value: i64) {
    let mut attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *attrs {
        map.insert(
            (class_id, attr_idx),
            ClassAttrEntry {
                tag: ClassAttrTag::Int,
                value,
            },
        );
    }
}

/// Get a class attribute integer
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// Returns: the stored integer value, or 0 if not found
#[no_mangle]
pub extern "C" fn rt_class_attr_get_int(class_id: u8, attr_idx: u32) -> i64 {
    let attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref map) = *attrs {
        map.get(&(class_id, attr_idx)).map(|e| e.value).unwrap_or(0)
    } else {
        0
    }
}

// ==================== Type-specific Float API ====================

/// Set a class attribute float
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// value: the float value to store (f64)
#[no_mangle]
pub extern "C" fn rt_class_attr_set_float(class_id: u8, attr_idx: u32, value: f64) {
    let mut attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *attrs {
        map.insert(
            (class_id, attr_idx),
            ClassAttrEntry {
                tag: ClassAttrTag::Float,
                value: value.to_bits() as i64,
            },
        );
    }
}

/// Get a class attribute float
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// Returns: the stored float value, or 0.0 if not found
#[no_mangle]
pub extern "C" fn rt_class_attr_get_float(class_id: u8, attr_idx: u32) -> f64 {
    let attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref map) = *attrs {
        map.get(&(class_id, attr_idx))
            .map(|e| f64::from_bits(e.value as u64))
            .unwrap_or(0.0)
    } else {
        0.0
    }
}

// ==================== Type-specific Bool API ====================

/// Set a class attribute boolean
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// value: the boolean value to store (i8: 0 = false, non-zero = true)
#[no_mangle]
pub extern "C" fn rt_class_attr_set_bool(class_id: u8, attr_idx: u32, value: i8) {
    let mut attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *attrs {
        map.insert(
            (class_id, attr_idx),
            ClassAttrEntry {
                tag: ClassAttrTag::Bool,
                value: value as i64,
            },
        );
    }
}

/// Get a class attribute boolean
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// Returns: the stored boolean value (i8: 0 = false, 1 = true), or 0 if not found
#[no_mangle]
pub extern "C" fn rt_class_attr_get_bool(class_id: u8, attr_idx: u32) -> i8 {
    let attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref map) = *attrs {
        map.get(&(class_id, attr_idx))
            .map(|e| e.value as i8)
            .unwrap_or(0)
    } else {
        0
    }
}

// ==================== Type-specific Pointer API (for heap objects) ====================

/// Set a class attribute pointer (for str, list, dict, tuple, etc.)
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// value: the pointer to a heap object (*mut Obj)
#[no_mangle]
pub extern "C" fn rt_class_attr_set_ptr(class_id: u8, attr_idx: u32, value: *mut Obj) {
    let mut attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *attrs {
        map.insert(
            (class_id, attr_idx),
            ClassAttrEntry {
                tag: ClassAttrTag::Ptr,
                value: value as i64,
            },
        );
    }
}

/// Get a class attribute pointer (for str, list, dict, tuple, etc.)
/// class_id: the ClassId as u8
/// attr_idx: the attribute index within the class
/// Returns: the stored pointer, or null if not found
#[no_mangle]
pub extern "C" fn rt_class_attr_get_ptr(class_id: u8, attr_idx: u32) -> *mut Obj {
    let attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref map) = *attrs {
        map.get(&(class_id, attr_idx))
            .map(|e| e.value as *mut Obj)
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

// ==================== GC Integration ====================

/// Get all class attribute pointer entries for GC marking
/// Returns a vector of pointers to heap objects stored in class attributes
pub fn get_class_attr_pointers() -> Vec<*mut Obj> {
    let attrs = CLASS_ATTRS
        .lock()
        .expect("CLASS_ATTRS mutex poisoned - another thread panicked");
    if let Some(ref map) = *attrs {
        map.values()
            .filter(|e| e.tag == ClassAttrTag::Ptr && e.value != 0)
            .map(|e| e.value as *mut Obj)
            .collect()
    } else {
        Vec::new()
    }
}
