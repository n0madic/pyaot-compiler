//! Class attribute storage for Python class-level variables
//!
//! This module provides typed key-value storage for class attributes.
//! Attributes are indexed by (class_id: u8, attr_idx: u32) and can store
//! any primitive type or heap object pointer.

use crate::object::Obj;
use std::cell::UnsafeCell;
use std::collections::HashMap;

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

/// Lock-free class attribute storage for single-threaded access
struct ClassAttrStorage(UnsafeCell<Option<HashMap<(u8, u32), ClassAttrEntry>>>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for ClassAttrStorage {}

static CLASS_ATTRS: ClassAttrStorage = ClassAttrStorage(UnsafeCell::new(None));

#[inline(always)]
unsafe fn attrs_map() -> &'static mut Option<HashMap<(u8, u32), ClassAttrEntry>> {
    &mut *CLASS_ATTRS.0.get()
}

/// Initialize class attribute storage (called by rt_init)
pub fn init_class_attrs() {
    unsafe {
        *CLASS_ATTRS.0.get() = Some(HashMap::new());
    }
}

/// Shutdown class attribute storage (called by rt_shutdown)
pub fn shutdown_class_attrs() {
    unsafe {
        *CLASS_ATTRS.0.get() = None;
    }
}

// ==================== Type-specific Integer API ====================

#[no_mangle]
pub extern "C" fn rt_class_attr_set_int(class_id: u8, attr_idx: u32, value: i64) {
    unsafe {
        if let Some(ref mut map) = *attrs_map() {
            map.insert(
                (class_id, attr_idx),
                ClassAttrEntry {
                    tag: ClassAttrTag::Int,
                    value,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_class_attr_get_int(class_id: u8, attr_idx: u32) -> i64 {
    unsafe {
        if let Some(ref map) = *attrs_map() {
            map.get(&(class_id, attr_idx)).map(|e| e.value).unwrap_or(0)
        } else {
            0
        }
    }
}

// ==================== Type-specific Float API ====================

#[no_mangle]
pub extern "C" fn rt_class_attr_set_float(class_id: u8, attr_idx: u32, value: f64) {
    unsafe {
        if let Some(ref mut map) = *attrs_map() {
            map.insert(
                (class_id, attr_idx),
                ClassAttrEntry {
                    tag: ClassAttrTag::Float,
                    value: value.to_bits() as i64,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_class_attr_get_float(class_id: u8, attr_idx: u32) -> f64 {
    unsafe {
        if let Some(ref map) = *attrs_map() {
            map.get(&(class_id, attr_idx))
                .map(|e| f64::from_bits(e.value as u64))
                .unwrap_or(0.0)
        } else {
            0.0
        }
    }
}

// ==================== Type-specific Bool API ====================

#[no_mangle]
pub extern "C" fn rt_class_attr_set_bool(class_id: u8, attr_idx: u32, value: i8) {
    unsafe {
        if let Some(ref mut map) = *attrs_map() {
            map.insert(
                (class_id, attr_idx),
                ClassAttrEntry {
                    tag: ClassAttrTag::Bool,
                    value: value as i64,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_class_attr_get_bool(class_id: u8, attr_idx: u32) -> i8 {
    unsafe {
        if let Some(ref map) = *attrs_map() {
            map.get(&(class_id, attr_idx))
                .map(|e| e.value as i8)
                .unwrap_or(0)
        } else {
            0
        }
    }
}

// ==================== Type-specific Pointer API (for heap objects) ====================

#[no_mangle]
pub extern "C" fn rt_class_attr_set_ptr(class_id: u8, attr_idx: u32, value: *mut Obj) {
    unsafe {
        if let Some(ref mut map) = *attrs_map() {
            map.insert(
                (class_id, attr_idx),
                ClassAttrEntry {
                    tag: ClassAttrTag::Ptr,
                    value: value as i64,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_class_attr_get_ptr(class_id: u8, attr_idx: u32) -> *mut Obj {
    unsafe {
        if let Some(ref map) = *attrs_map() {
            map.get(&(class_id, attr_idx))
                .map(|e| e.value as *mut Obj)
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    }
}

// ==================== GC Integration ====================

/// Get all class attribute pointer entries for GC marking
pub fn get_class_attr_pointers() -> Vec<*mut Obj> {
    unsafe {
        if let Some(ref map) = *attrs_map() {
            map.values()
                .filter(|e| e.tag == ClassAttrTag::Ptr && e.value != 0)
                .map(|e| e.value as *mut Obj)
                .collect()
        } else {
            Vec::new()
        }
    }
}
