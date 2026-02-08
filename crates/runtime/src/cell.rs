//! Cell objects for Python nonlocal statement support
//!
//! Cells are mutable reference holders that allow nested functions to modify
//! variables from enclosing scopes. When a variable is used with `nonlocal`,
//! it's wrapped in a Cell that gets passed to nested functions.

use crate::gc::gc_alloc;
use crate::object::{Obj, ObjHeader};

/// Cell type tag for runtime objects
pub const CELL_TYPE_TAG: u8 = 12;

/// Type tag for cell value
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellValueTag {
    Int = 0,
    Float = 1,
    Bool = 2,
    Ptr = 3, // Pointer to heap object (str, list, dict, etc.)
}

/// Cell object - mutable reference holder for nonlocal variables
#[repr(C)]
pub struct CellObj {
    pub header: ObjHeader,
    pub value_tag: u8,
    pub value: i64, // int, float bits, bool, or pointer
}

// ==================== Cell Creation Functions ====================

/// Create a new Cell holding an integer value
#[no_mangle]
pub extern "C" fn rt_make_cell_int(value: i64) -> *mut Obj {
    let size = std::mem::size_of::<CellObj>();
    let cell = gc_alloc(size, CELL_TYPE_TAG) as *mut CellObj;
    unsafe {
        (*cell).value_tag = CellValueTag::Int as u8;
        (*cell).value = value;
    }
    cell as *mut Obj
}

/// Create a new Cell holding a float value
#[no_mangle]
pub extern "C" fn rt_make_cell_float(value: f64) -> *mut Obj {
    let size = std::mem::size_of::<CellObj>();
    let cell = gc_alloc(size, CELL_TYPE_TAG) as *mut CellObj;
    unsafe {
        (*cell).value_tag = CellValueTag::Float as u8;
        (*cell).value = value.to_bits() as i64;
    }
    cell as *mut Obj
}

/// Create a new Cell holding a boolean value
#[no_mangle]
pub extern "C" fn rt_make_cell_bool(value: i8) -> *mut Obj {
    let size = std::mem::size_of::<CellObj>();
    let cell = gc_alloc(size, CELL_TYPE_TAG) as *mut CellObj;
    unsafe {
        (*cell).value_tag = CellValueTag::Bool as u8;
        (*cell).value = value as i64;
    }
    cell as *mut Obj
}

/// Create a new Cell holding a pointer (heap object)
#[no_mangle]
pub extern "C" fn rt_make_cell_ptr(value: *mut Obj) -> *mut Obj {
    let size = std::mem::size_of::<CellObj>();
    let cell = gc_alloc(size, CELL_TYPE_TAG) as *mut CellObj;
    unsafe {
        (*cell).value_tag = CellValueTag::Ptr as u8;
        (*cell).value = value as i64;
    }
    cell as *mut Obj
}

// ==================== Cell Get Functions ====================

/// Get integer value from cell
#[no_mangle]
pub extern "C" fn rt_cell_get_int(cell: *mut Obj) -> i64 {
    unsafe {
        let cell = cell as *mut CellObj;
        (*cell).value
    }
}

/// Get float value from cell
#[no_mangle]
pub extern "C" fn rt_cell_get_float(cell: *mut Obj) -> f64 {
    unsafe {
        let cell = cell as *mut CellObj;
        f64::from_bits((*cell).value as u64)
    }
}

/// Get boolean value from cell
#[no_mangle]
pub extern "C" fn rt_cell_get_bool(cell: *mut Obj) -> i8 {
    unsafe {
        let cell = cell as *mut CellObj;
        (*cell).value as i8
    }
}

/// Get pointer value from cell
#[no_mangle]
pub extern "C" fn rt_cell_get_ptr(cell: *mut Obj) -> *mut Obj {
    unsafe {
        let cell = cell as *mut CellObj;
        (*cell).value as *mut Obj
    }
}

// ==================== Cell Set Functions ====================

/// Set integer value in cell
#[no_mangle]
pub extern "C" fn rt_cell_set_int(cell: *mut Obj, value: i64) {
    unsafe {
        let cell = cell as *mut CellObj;
        (*cell).value_tag = CellValueTag::Int as u8;
        (*cell).value = value;
    }
}

/// Set float value in cell
#[no_mangle]
pub extern "C" fn rt_cell_set_float(cell: *mut Obj, value: f64) {
    unsafe {
        let cell = cell as *mut CellObj;
        (*cell).value_tag = CellValueTag::Float as u8;
        (*cell).value = value.to_bits() as i64;
    }
}

/// Set boolean value in cell
#[no_mangle]
pub extern "C" fn rt_cell_set_bool(cell: *mut Obj, value: i8) {
    unsafe {
        let cell = cell as *mut CellObj;
        (*cell).value_tag = CellValueTag::Bool as u8;
        (*cell).value = value as i64;
    }
}

/// Set pointer value in cell
#[no_mangle]
pub extern "C" fn rt_cell_set_ptr(cell: *mut Obj, value: *mut Obj) {
    unsafe {
        let cell = cell as *mut CellObj;
        (*cell).value_tag = CellValueTag::Ptr as u8;
        (*cell).value = value as i64;
    }
}

// ==================== GC Support ====================

/// Get the pointer value if this cell contains a pointer (for GC marking)
///
/// # Safety
/// `cell` must be a valid, non-null pointer to a CellObj.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn cell_get_ptr_for_gc(cell: *mut CellObj) -> Option<*mut Obj> {
    unsafe {
        if (*cell).value_tag == CellValueTag::Ptr as u8 && (*cell).value != 0 {
            Some((*cell).value as *mut Obj)
        } else {
            None
        }
    }
}
