//! Core list operations: creation, access, and finalization

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::exceptions::ExceptionType;
use crate::gc;
use crate::object::{ListObj, Obj, TypeTagKind};
use pyaot_core_defs::Value;
use std::alloc::{alloc_zeroed, realloc, Layout};

/// Create a new list with given capacity.
/// Returns: pointer to allocated ListObj
pub fn rt_make_list(capacity: i64) -> *mut Obj {
    let capacity = capacity.max(0) as usize;

    // Calculate size for ListObj (header + len + capacity + data pointer + elem_tag)
    let list_size = std::mem::size_of::<ListObj>();

    // Allocate ListObj using GC
    let obj = gc::gc_alloc(list_size, TypeTagKind::List as u8);

    unsafe {
        let list = obj as *mut ListObj;
        (*list).len = 0;
        (*list).capacity = capacity;

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
#[export_name = "rt_make_list"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_list_abi(capacity: i64) -> Value {
    Value::from_ptr(rt_make_list(capacity))
}

/// Set element in list at given index
/// Supports negative indexing
pub fn rt_list_set(list: *mut Obj, index: i64, value: *mut Obj) {
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

        let data = (*list_obj).data;
        if !data.is_null() {
            *data.add(idx as usize) = Value(value as u64);
        }
    }
}
#[export_name = "rt_list_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_set_abi(list: Value, index: i64, value: Value) {
    rt_list_set(list.unwrap_ptr(), index, value.unwrap_ptr())
}

/// Get element from list at given index
/// Supports negative indexing
/// Returns: pointer to element or null if out of bounds
pub fn rt_list_get(list: *mut Obj, index: i64) -> *mut Obj {
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
        v.0 as *mut Obj
    }
}
#[export_name = "rt_list_get"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_get_abi(list: Value, index: i64) -> Value {
    Value::from_ptr(rt_list_get(list.unwrap_ptr(), index))
}

/// Get length of list
pub fn rt_list_len(list: *mut Obj) -> i64 {
    if list.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_len");
        let list_obj = list as *mut ListObj;
        (*list_obj).len as i64
    }
}
#[export_name = "rt_list_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_len_abi(list: Value) -> i64 {
    rt_list_len(list.unwrap_ptr())
}

/// Bounds-check helper returning the raw `Value` stored at `index`, or `None`.
unsafe fn list_get_value(list: *mut Obj, index: i64) -> Option<Value> {
    if list.is_null() {
        return None;
    }
    debug_assert_type_tag!(list, TypeTagKind::List, "list_get_value");
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
    Some(*data.add(idx as usize))
}

/// Get a typed scalar element from a list. Always returns i64.
///
/// `elem_kind`:
/// - 0 = Int:   raw i64 (`Value::unwrap_int`)
/// - 1 = Float: f64 bit-pattern from the heap-boxed `FloatObj`
/// - 2 = Bool:  0/1 i64 (`Value::unwrap_bool`)
pub fn rt_list_get_typed(list: *mut Obj, index: i64, elem_kind: u8) -> i64 {
    use crate::object::FloatObj;

    let v = match unsafe { list_get_value(list, index) } {
        Some(v) => v,
        None => return 0,
    };
    unsafe {
        match elem_kind {
            0 => v.unwrap_int(),
            1 => (*(v.0 as *mut FloatObj)).value.to_bits() as i64,
            2 => i64::from(v.unwrap_bool()),
            _ => unreachable!("invalid elem_kind: {elem_kind}"),
        }
    }
}
#[export_name = "rt_list_get_typed"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_get_typed_abi(list: Value, index: i64, elem_kind: u8) -> i64 {
    rt_list_get_typed(list.unwrap_ptr(), index, elem_kind)
}

/// Push element to end of list (used during list construction)
pub fn rt_list_push(list: *mut Obj, value: *mut Obj) {
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

        // Add element
        let data = (*list_obj).data;
        if !data.is_null() {
            *data.add(len) = Value(value as u64);
            (*list_obj).len = len + 1;
        }
    }
}
#[export_name = "rt_list_push"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_list_push_abi(list: Value, value: Value) {
    rt_list_push(list.unwrap_ptr(), value.unwrap_ptr())
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
