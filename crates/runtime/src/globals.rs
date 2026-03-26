//! Global variable storage for Python global statements
//!
//! This module provides typed key-value storage for global variables.
//! Variables are indexed by their VarId (u32) and can store any primitive type
//! or heap object pointer.

use crate::object::Obj;
use std::cell::UnsafeCell;
use std::collections::HashMap;

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

/// Lock-free global variable storage for single-threaded access
struct GlobalStorage(UnsafeCell<Option<HashMap<u32, GlobalEntry>>>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for GlobalStorage {}

static GLOBALS: GlobalStorage = GlobalStorage(UnsafeCell::new(None));

#[inline(always)]
unsafe fn globals_map() -> &'static mut Option<HashMap<u32, GlobalEntry>> {
    &mut *GLOBALS.0.get()
}

/// Initialize global storage (called by rt_init)
pub fn init_globals() {
    unsafe {
        *GLOBALS.0.get() = Some(HashMap::new());
    }
}

/// Shutdown global storage (called by rt_shutdown)
pub fn shutdown_globals() {
    unsafe {
        *GLOBALS.0.get() = None;
    }
}

// ==================== Legacy API (kept for backward compatibility) ====================

/// Set a global variable value (legacy API - assumes integer)
#[no_mangle]
pub extern "C" fn rt_global_set(var_id: u32, value: i64) {
    unsafe {
        if let Some(ref mut map) = *globals_map() {
            map.insert(
                var_id,
                GlobalEntry {
                    tag: GlobalTag::Int,
                    value,
                },
            );
        }
    }
}

/// Get a global variable value (legacy API - assumes integer).
///
/// Returns 0 when the variable has not been set yet.  This does **not** raise
/// `NameError` because the AOT compiler statically guarantees that every
/// `GlobalGet` is dominated by a `GlobalSet` in the same compilation unit.
/// The only way to reach this function with an unknown `var_id` is via a
/// programming error in the compiler itself, not in user Python code.
/// The type-specific `rt_global_get_*` functions share the same invariant.
#[no_mangle]
pub extern "C" fn rt_global_get(var_id: u32) -> i64 {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.get(&var_id).map(|e| e.value).unwrap_or(0)
        } else {
            0
        }
    }
}

// ==================== Type-specific Integer API ====================

#[no_mangle]
pub extern "C" fn rt_global_set_int(var_id: u32, value: i64) {
    unsafe {
        if let Some(ref mut map) = *globals_map() {
            map.insert(
                var_id,
                GlobalEntry {
                    tag: GlobalTag::Int,
                    value,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_global_get_int(var_id: u32) -> i64 {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.get(&var_id).map(|e| e.value).unwrap_or(0)
        } else {
            0
        }
    }
}

// ==================== Type-specific Float API ====================

#[no_mangle]
pub extern "C" fn rt_global_set_float(var_id: u32, value: f64) {
    unsafe {
        if let Some(ref mut map) = *globals_map() {
            map.insert(
                var_id,
                GlobalEntry {
                    tag: GlobalTag::Float,
                    value: value.to_bits() as i64,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_global_get_float(var_id: u32) -> f64 {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.get(&var_id)
                .map(|e| f64::from_bits(e.value as u64))
                .unwrap_or(0.0)
        } else {
            0.0
        }
    }
}

// ==================== Type-specific Bool API ====================

#[no_mangle]
pub extern "C" fn rt_global_set_bool(var_id: u32, value: i8) {
    unsafe {
        if let Some(ref mut map) = *globals_map() {
            map.insert(
                var_id,
                GlobalEntry {
                    tag: GlobalTag::Bool,
                    value: value as i64,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_global_get_bool(var_id: u32) -> i8 {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.get(&var_id).map(|e| e.value as i8).unwrap_or(0)
        } else {
            0
        }
    }
}

// ==================== Type-specific Pointer API (for heap objects) ====================

#[no_mangle]
pub extern "C" fn rt_global_set_ptr(var_id: u32, value: *mut Obj) {
    unsafe {
        if let Some(ref mut map) = *globals_map() {
            map.insert(
                var_id,
                GlobalEntry {
                    tag: GlobalTag::Ptr,
                    value: value as i64,
                },
            );
        }
    }
}

#[no_mangle]
pub extern "C" fn rt_global_get_ptr(var_id: u32) -> *mut Obj {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.get(&var_id)
                .map(|e| e.value as *mut Obj)
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    }
}

// ==================== GC Integration ====================

/// Mark all global pointer variables as GC roots
pub fn mark_global_pointers(mark_fn: impl Fn(*mut Obj)) {
    unsafe {
        if let Some(ref map) = *globals_map() {
            for entry in map.values() {
                if entry.tag == GlobalTag::Ptr && entry.value != 0 {
                    mark_fn(entry.value as *mut Obj);
                }
            }
        }
    }
}

/// Get all global pointer entries for GC marking
pub fn get_global_pointers() -> Vec<*mut Obj> {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.values()
                .filter(|e| e.tag == GlobalTag::Ptr && e.value != 0)
                .map(|e| e.value as *mut Obj)
                .collect()
        } else {
            Vec::new()
        }
    }
}
