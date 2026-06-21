//! Class attribute storage for Python class-level variables
//!
//! Class-level attributes are stored uniformly as tagged heap-pointer slots
//! (`Value` bits), indexed by (class_id: u8, attr_idx: u32). The compiler routes
//! every class-attribute read/write through `rt_class_attr_get_ptr` /
//! `rt_class_attr_set_ptr` (invariant 2: one uniform tagged substrate); the GC
//! walks the live entries as roots.

use crate::object::Obj;
use pyaot_core_defs::Value;
use std::cell::UnsafeCell;
use std::collections::HashMap;

/// Lock-free class attribute storage for single-threaded access
struct ClassAttrStorage(UnsafeCell<Option<HashMap<(u8, u32), *mut Obj>>>);

// Safety: The runtime is single-threaded (AOT-compiled Python has no threading)
unsafe impl Sync for ClassAttrStorage {}

static CLASS_ATTRS: ClassAttrStorage = ClassAttrStorage(UnsafeCell::new(None));

#[inline(always)]
unsafe fn attrs_map() -> &'static mut Option<HashMap<(u8, u32), *mut Obj>> {
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

// ==================== Pointer API (uniform tagged slots) ====================
//
// A slot stores the raw `Value` bits of a class attribute. The value may be a
// tagged immediate (int/bool/None) rather than a real pointer, so the bits are
// stored verbatim — `rt_class_attr_set_ptr_abi` deliberately bypasses
// `unwrap_ptr`.

pub fn rt_class_attr_set_ptr(class_id: u8, attr_idx: u32, value: *mut Obj) {
    unsafe {
        if let Some(ref mut map) = *attrs_map() {
            map.insert((class_id, attr_idx), value);
        }
    }
}
#[export_name = "rt_class_attr_set_ptr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_class_attr_set_ptr_abi(class_id: u8, attr_idx: u32, value: Value) {
    // `value` is a stored class-attribute slot that may be a tagged immediate
    // (int/bool/None); pass raw bits so the tag survives instead of tripping
    // `unwrap_ptr`'s debug `is_ptr` assertion.
    rt_class_attr_set_ptr(class_id, attr_idx, value.0 as *mut Obj)
}

pub fn rt_class_attr_get_ptr(class_id: u8, attr_idx: u32) -> *mut Obj {
    unsafe {
        if let Some(ref map) = *attrs_map() {
            map.get(&(class_id, attr_idx))
                .copied()
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    }
}
#[export_name = "rt_class_attr_get_ptr"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_class_attr_get_ptr_abi(class_id: u8, attr_idx: u32) -> Value {
    Value::from_ptr(rt_class_attr_get_ptr(class_id, attr_idx))
}

// ==================== GC Integration ====================

/// Get all class attribute slots for GC marking.
///
/// A slot may hold a tagged immediate (int/bool/None); the GC's marker filters
/// non-pointer bits (`TAG_MASK`), so every non-null slot is reported here
/// verbatim.
pub fn get_class_attr_pointers() -> Vec<*mut Obj> {
    unsafe {
        if let Some(ref map) = *attrs_map() {
            map.values().filter(|&&p| !p.is_null()).copied().collect()
        } else {
            Vec::new()
        }
    }
}
