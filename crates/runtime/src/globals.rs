//! Global variable storage for Python global statements
//!
//! This module provides typed key-value storage for global variables.
//! Variables are indexed by their VarId (u32) and can store any primitive type
//! or heap object pointer.

use crate::object::Obj;
use std::collections::HashMap;
use std::sync::Mutex;

/// Type tag for global variable entries
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalTag {
    Int = 0,
    Float = 1,
    Bool = 2,
    Ptr = 3, // Pointer to heap object (str, list, dict, etc.)
}

/// Entry for a global variable with type information
#[derive(Debug, Clone, Copy)]
pub struct GlobalEntry {
    pub tag: GlobalTag,
    pub value: i64, // Stores int, float bits, bool, or pointer
}

/// Global variable storage - maps VarId to typed GlobalEntry
static GLOBALS: Mutex<Option<HashMap<u32, GlobalEntry>>> = Mutex::new(None);

/// Initialize global storage (called by rt_init)
pub fn init_globals() {
    let mut globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    *globals = Some(HashMap::new());
}

/// Shutdown global storage (called by rt_shutdown)
pub fn shutdown_globals() {
    let mut globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    *globals = None;
}

// ==================== Legacy API (kept for backward compatibility) ====================

/// Set a global variable value (legacy API - assumes integer)
/// var_id: the VarId as u32
/// value: the value to store (i64)
#[no_mangle]
pub extern "C" fn rt_global_set(var_id: u32, value: i64) {
    let mut globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *globals {
        map.insert(
            var_id,
            GlobalEntry {
                tag: GlobalTag::Int,
                value,
            },
        );
    }
}

/// Get a global variable value (legacy API - assumes integer)
/// var_id: the VarId as u32
/// Returns: the stored value, or 0 if not found
#[no_mangle]
pub extern "C" fn rt_global_get(var_id: u32) -> i64 {
    let globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref map) = *globals {
        map.get(&var_id).map(|e| e.value).unwrap_or(0)
    } else {
        0
    }
}

// ==================== Type-specific Integer API ====================

/// Set a global integer variable
/// var_id: the VarId as u32
/// value: the integer value to store (i64)
#[no_mangle]
pub extern "C" fn rt_global_set_int(var_id: u32, value: i64) {
    let mut globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *globals {
        map.insert(
            var_id,
            GlobalEntry {
                tag: GlobalTag::Int,
                value,
            },
        );
    }
}

/// Get a global integer variable
/// var_id: the VarId as u32
/// Returns: the stored integer value, or 0 if not found
#[no_mangle]
pub extern "C" fn rt_global_get_int(var_id: u32) -> i64 {
    let globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref map) = *globals {
        map.get(&var_id).map(|e| e.value).unwrap_or(0)
    } else {
        0
    }
}

// ==================== Type-specific Float API ====================

/// Set a global float variable
/// var_id: the VarId as u32
/// value: the float value to store (f64)
#[no_mangle]
pub extern "C" fn rt_global_set_float(var_id: u32, value: f64) {
    let mut globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *globals {
        map.insert(
            var_id,
            GlobalEntry {
                tag: GlobalTag::Float,
                value: value.to_bits() as i64,
            },
        );
    }
}

/// Get a global float variable
/// var_id: the VarId as u32
/// Returns: the stored float value, or 0.0 if not found
#[no_mangle]
pub extern "C" fn rt_global_get_float(var_id: u32) -> f64 {
    let globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref map) = *globals {
        map.get(&var_id)
            .map(|e| f64::from_bits(e.value as u64))
            .unwrap_or(0.0)
    } else {
        0.0
    }
}

// ==================== Type-specific Bool API ====================

/// Set a global boolean variable
/// var_id: the VarId as u32
/// value: the boolean value to store (i8: 0 = false, non-zero = true)
#[no_mangle]
pub extern "C" fn rt_global_set_bool(var_id: u32, value: i8) {
    let mut globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *globals {
        map.insert(
            var_id,
            GlobalEntry {
                tag: GlobalTag::Bool,
                value: value as i64,
            },
        );
    }
}

/// Get a global boolean variable
/// var_id: the VarId as u32
/// Returns: the stored boolean value (i8: 0 = false, 1 = true), or 0 if not found
#[no_mangle]
pub extern "C" fn rt_global_get_bool(var_id: u32) -> i8 {
    let globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref map) = *globals {
        map.get(&var_id).map(|e| e.value as i8).unwrap_or(0)
    } else {
        0
    }
}

// ==================== Type-specific Pointer API (for heap objects) ====================

/// Set a global pointer variable (for str, list, dict, tuple, etc.)
/// var_id: the VarId as u32
/// value: the pointer to a heap object (*mut Obj)
#[no_mangle]
pub extern "C" fn rt_global_set_ptr(var_id: u32, value: *mut Obj) {
    let mut globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref mut map) = *globals {
        map.insert(
            var_id,
            GlobalEntry {
                tag: GlobalTag::Ptr,
                value: value as i64,
            },
        );
    }
}

/// Get a global pointer variable (for str, list, dict, tuple, etc.)
/// var_id: the VarId as u32
/// Returns: the stored pointer, or null if not found
#[no_mangle]
pub extern "C" fn rt_global_get_ptr(var_id: u32) -> *mut Obj {
    let globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref map) = *globals {
        map.get(&var_id)
            .map(|e| e.value as *mut Obj)
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

// ==================== GC Integration ====================

/// Mark all global pointer variables as GC roots
/// Called during GC mark phase to ensure heap objects stored in globals survive collection
pub fn mark_global_pointers(mark_fn: impl Fn(*mut Obj)) {
    let globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref map) = *globals {
        for entry in map.values() {
            if entry.tag == GlobalTag::Ptr && entry.value != 0 {
                mark_fn(entry.value as *mut Obj);
            }
        }
    }
}

/// Get all global pointer entries for GC marking
/// Returns a vector of pointers to heap objects stored in globals
pub fn get_global_pointers() -> Vec<*mut Obj> {
    let globals = GLOBALS
        .lock()
        .expect("GLOBALS mutex poisoned - another thread panicked");
    if let Some(ref map) = *globals {
        map.values()
            .filter(|e| e.tag == GlobalTag::Ptr && e.value != 0)
            .map(|e| e.value as *mut Obj)
            .collect()
    } else {
        Vec::new()
    }
}
