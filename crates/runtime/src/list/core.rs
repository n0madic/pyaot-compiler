//! Core list operations: creation, access, and finalization

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::exceptions::ExceptionType;
use crate::gc;
use crate::object::{ListObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;
use std::alloc::{alloc_zeroed, realloc, Layout};

use super::{load_value_as_raw, store_raw_as_value};

/// Create a new list with given capacity and element tag
/// elem_tag: ELEM_HEAP_OBJ (0), ELEM_RAW_INT (1), or ELEM_RAW_BOOL (2)
/// Returns: pointer to allocated ListObj
#[no_mangle]
pub extern "C" fn rt_make_list(capacity: i64, elem_tag: u8) -> *mut Obj {
    let capacity = capacity.max(0) as usize;

    // Calculate size for ListObj (header + len + capacity + data pointer + elem_tag)
    let list_size = std::mem::size_of::<ListObj>();

    // Allocate ListObj using GC
    let obj = gc::gc_alloc(list_size, TypeTagKind::List as u8);

    unsafe {
        let list = obj as *mut ListObj;
        (*list).len = 0;
        (*list).capacity = capacity;
        (*list).elem_tag = elem_tag;

        // Allocate data array separately if capacity > 0.
        // Physical layout is 8 bytes per slot — identical to the pre-S2.3
        // `*mut Obj` layout, so existing capacity math and GC assertions
        // survive unchanged.
        if capacity > 0 {
            let data_layout = Layout::array::<Value>(capacity)
                .expect("Allocation size overflow - capacity too large");
            let data_ptr = alloc_zeroed(data_layout) as *mut Value;
            (*list).data = data_ptr;
        } else {
            (*list).data = std::ptr::null_mut();
        }
    }

    obj
}

/// Set element in list at given index
/// Supports negative indexing
#[no_mangle]
pub extern "C" fn rt_list_set(list: *mut Obj, index: i64, value: *mut Obj) {
    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_set");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return;
        }

        // Validate elem_tag matches value type (debug mode only)
        crate::validate_elem_tag!("list", idx, (*list_obj).elem_tag, value);

        let data = (*list_obj).data;
        if !data.is_null() {
            let elem_tag = (*list_obj).elem_tag;
            *data.add(idx as usize) = store_raw_as_value(value, elem_tag);
        }
    }
}

/// Get element from list at given index
/// Supports negative indexing
/// Returns: pointer to element or null if out of bounds
#[no_mangle]
pub extern "C" fn rt_list_get(list: *mut Obj, index: i64) -> *mut Obj {
    if list.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_get");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return std::ptr::null_mut();
        }

        let data = (*list_obj).data;
        if data.is_null() {
            return std::ptr::null_mut();
        }

        let v = *data.add(idx as usize);
        load_value_as_raw(v, (*list_obj).elem_tag)
    }
}

/// Get length of list
#[no_mangle]
pub extern "C" fn rt_list_len(list: *mut Obj) -> i64 {
    if list.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_len");
        let list_obj = list as *mut ListObj;
        (*list_obj).len as i64
    }
}

/// Shared bounds-checking and element access for typed list getters.
/// Returns the raw-ABI element pointer (after Value→raw conversion) on
/// success, or `None` if out of bounds / null.
unsafe fn list_get_element(list: *mut Obj, index: i64) -> Option<*mut Obj> {
    if list.is_null() {
        return None;
    }

    debug_assert_type_tag!(list, TypeTagKind::List, "list_get_element");
    let list_obj = list as *mut ListObj;
    let len = (*list_obj).len as i64;

    let idx = if index < 0 { len + index } else { index };

    if idx < 0 || idx >= len {
        return None;
    }

    let data = (*list_obj).data;
    if data.is_null() {
        return None;
    }

    let v = *data.add(idx as usize);
    Some(load_value_as_raw(v, (*list_obj).elem_tag))
}

/// Get a typed scalar element from a list, unboxing if necessary. Always returns i64.
///
/// `elem_kind` selects the element type:
/// - 0 = Int:   returns raw i64 (unboxes IntObj when stored as ELEM_HEAP_OBJ)
/// - 1 = Float: returns f64 bit-pattern as i64 (unboxes FloatObj when stored as ELEM_HEAP_OBJ)
/// - 2 = Bool:  returns i8 zero-extended to i64 (unboxes BoolObj when stored as ELEM_HEAP_OBJ)
///
/// Replaces the three separate `rt_list_get_int`, `rt_list_get_float`, `rt_list_get_bool`
/// functions. The caller (generated code) passes the statically-known elem_kind tag; the
/// codegen descriptor system handles the I64→F64 bitcast for float destinations.
#[no_mangle]
pub extern "C" fn rt_list_get_typed(list: *mut Obj, index: i64, elem_kind: u8) -> i64 {
    use crate::object::{BoolObj, FloatObj, IntObj, ELEM_HEAP_OBJ, ELEM_RAW_BOOL, ELEM_RAW_INT};

    unsafe {
        let elem = match list_get_element(list, index) {
            Some(e) => e,
            None => return 0,
        };
        let stored_tag = (*(list as *mut ListObj)).elem_tag;

        match elem_kind {
            0 => {
                // Int: raw i64 or unbox IntObj
                match stored_tag {
                    ELEM_RAW_INT => elem as i64,
                    ELEM_HEAP_OBJ => {
                        if elem.is_null() {
                            return 0;
                        }
                        (*(elem as *mut IntObj)).value
                    }
                    _ => elem as i64,
                }
            }
            1 => {
                // Float: unbox FloatObj or treat raw bits as f64
                let f = match stored_tag {
                    ELEM_HEAP_OBJ => {
                        if elem.is_null() {
                            return 0;
                        }
                        (*(elem as *mut FloatObj)).value
                    }
                    _ => f64::from_bits(elem as u64),
                };
                f.to_bits() as i64
            }
            _ => {
                // Bool (elem_kind=2): raw i8 or unbox BoolObj
                let b: i8 = match stored_tag {
                    ELEM_RAW_BOOL => elem as i8,
                    ELEM_HEAP_OBJ => {
                        if elem.is_null() {
                            return 0;
                        }
                        i8::from((*(elem as *mut BoolObj)).value)
                    }
                    _ => elem as i8,
                };
                b as i64
            }
        }
    }
}

/// Push element to end of list (used during list construction)
#[no_mangle]
pub extern "C" fn rt_list_push(list: *mut Obj, value: *mut Obj) {
    if list.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_push");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let capacity = (*list_obj).capacity;

        // Grow if needed (CPython-style growth: ~12.5% for large lists)
        if len >= capacity {
            let new_capacity = super::list_grow_capacity(capacity);
            let data = (*list_obj).data;

            if data.is_null() {
                let new_layout = Layout::array::<Value>(new_capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_data = alloc_zeroed(new_layout) as *mut Value;
                (*list_obj).data = new_data;
            } else {
                let old_layout = Layout::array::<Value>(capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_layout = Layout::array::<Value>(new_capacity)
                    .expect("Allocation size overflow - capacity too large");
                let new_data =
                    realloc(data as *mut u8, old_layout, new_layout.size()) as *mut Value;
                if new_data.is_null() {
                    raise_exc!(
                        ExceptionType::MemoryError,
                        "cannot allocate memory for list"
                    );
                }
                // Zero new elements: Value(0) is a null-pointer-tagged Value,
                // which GC correctly ignores.
                for i in capacity..new_capacity {
                    *new_data.add(i) = Value(0);
                }
                (*list_obj).data = new_data;
            }
            (*list_obj).capacity = new_capacity;
        }

        // Validate elem_tag matches value type (debug mode only)
        crate::validate_elem_tag!("list.push", len, (*list_obj).elem_tag, value);

        // Add element
        let data = (*list_obj).data;
        if !data.is_null() {
            let elem_tag = (*list_obj).elem_tag;
            *data.add(len) = store_raw_as_value(value, elem_tag);
            (*list_obj).len = len + 1;
        }
    }
}

/// Finalize a list by freeing its data array
/// Called by GC during sweep phase before freeing the ListObj itself
///
/// # Safety
/// The caller must ensure that `list` is a valid pointer to a ListObj
/// that is about to be deallocated.
pub unsafe fn list_finalize(list: *mut Obj) {
    use std::alloc::dealloc;

    if list.is_null() {
        return;
    }

    let list_obj = list as *mut ListObj;
    let data = (*list_obj).data;
    let capacity = (*list_obj).capacity;

    // Free the data array if allocated
    if !data.is_null() && capacity > 0 {
        let data_layout = Layout::array::<Value>(capacity)
            .expect("Allocation size overflow - capacity too large");
        dealloc(data as *mut u8, data_layout);
    }
}
