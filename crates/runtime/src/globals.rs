//! Global variable storage for Python global statements
//!
//! Promoted module-globals are stored uniformly as tagged heap-pointer slots
//! (`Value` bits), indexed by their VarId (u32). The compiler routes every
//! promoted-global read/write through `rt_global_get_ptr` / `rt_global_set_ptr`
//! (invariant 2: one uniform tagged substrate); the GC walks the live entries
//! as roots.

use crate::object::Obj;
use pyaot_core_defs::Value;
use std::cell::UnsafeCell;
use std::collections::HashMap;

/// Lock-free global variable storage for single-threaded access
struct GlobalStorage(UnsafeCell<Option<HashMap<u32, *mut Obj>>>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for GlobalStorage {}

static GLOBALS: GlobalStorage = GlobalStorage(UnsafeCell::new(None));

#[inline(always)]
unsafe fn globals_map() -> &'static mut Option<HashMap<u32, *mut Obj>> {
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

// ==================== Pointer API (uniform tagged slots) ====================
//
// A slot stores the raw `Value` bits of a promoted global. The value may be a
// tagged immediate (int/bool/None) rather than a real pointer, so the bits are
// stored verbatim — `rt_global_set_ptr_abi` deliberately bypasses `unwrap_ptr`.

pub fn rt_global_set_ptr(var_id: u32, value: *mut Obj) {
    unsafe {
        if let Some(ref mut map) = *globals_map() {
            map.insert(var_id, value);
        }
    }
}
#[export_name = "rt_global_set_ptr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_global_set_ptr_abi(var_id: u32, value: Value) {
    // `value` is a stored slot that may be a tagged immediate (int/bool/None);
    // pass raw bits so the tag survives instead of tripping `unwrap_ptr`'s
    // debug `is_ptr` assertion.
    rt_global_set_ptr(var_id, value.0 as *mut Obj)
}

pub fn rt_global_get_ptr(var_id: u32) -> *mut Obj {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.get(&var_id).copied().unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    }
}
#[export_name = "rt_global_get_ptr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_global_get_ptr_abi(var_id: u32) -> Value {
    Value::from_ptr(rt_global_get_ptr(var_id))
}

// ==================== GC Integration ====================

/// Get all global slots for GC marking.
///
/// A slot may hold a tagged immediate (int/bool/None); the GC's marker filters
/// non-pointer bits (`TAG_MASK`), so every non-null slot is reported here
/// verbatim.
pub fn get_global_pointers() -> Vec<*mut Obj> {
    unsafe {
        if let Some(ref map) = *globals_map() {
            map.values().filter(|&&p| !p.is_null()).copied().collect()
        } else {
            Vec::new()
        }
    }
}
