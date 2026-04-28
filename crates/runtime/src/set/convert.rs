//! Set conversion operations: to_list

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{ListObj, Obj, SetObj, TypeTagKind, TOMBSTONE};
use pyaot_core_defs::Value;
use std::alloc::{alloc_zeroed, Layout};

/// Convert set to list (for iteration support)
/// Returns: pointer to ListObj containing all set elements
pub fn rt_set_to_list(set: *mut Obj) -> *mut Obj {
    if set.is_null() {
        // Return empty list
        let size = std::mem::size_of::<ListObj>();
        let obj = gc::gc_alloc(size, TypeTagKind::List as u8);
        unsafe {
            let list = obj as *mut ListObj;
            (*list).len = 0;
            (*list).capacity = 0;
            (*list).data = std::ptr::null_mut();
        }
        return obj;
    }

    unsafe {
        debug_assert_type_tag!(set, TypeTagKind::Set, "rt_set_to_list");
        let set_obj = set as *mut SetObj;
        let set_len = (*set_obj).len;
        let capacity = (*set_obj).capacity;

        // Root the set across gc_alloc which may trigger a GC collection.
        let mut roots: [*mut Obj; 1] = [set];
        let mut frame = crate::gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        crate::gc::gc_push(&mut frame);

        // Allocate list with set length
        let list_size = std::mem::size_of::<ListObj>();
        let list_obj = gc::gc_alloc(list_size, TypeTagKind::List as u8);

        crate::gc::gc_pop();

        // Re-derive set pointer through rooted slot after allocation.
        let set_obj = roots[0] as *mut SetObj;

        let list = list_obj as *mut ListObj;

        // Allocate data array (raw allocator — does not trigger GC).
        // Post-S2.3: list storage is `[Value]`; the allocation size/align are
        // identical to the pre-S2.3 `*mut Obj` layout.
        let data_layout =
            Layout::array::<Value>(set_len).expect("Allocation size overflow - capacity too large");
        let data = alloc_zeroed(data_layout) as *mut Value;

        (*list).len = set_len;
        (*list).capacity = set_len;
        (*list).data = data;

        // Copy non-empty, non-tombstone elements to list. Each entry is a
        // heap pointer, so wrap with `Value::from_ptr`.
        let mut list_idx = 0;
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if elem.0 != 0 && elem != TOMBSTONE {
                *data.add(list_idx) = elem;
                list_idx += 1;
            }
        }

        list_obj
    }
}
#[export_name = "rt_set_to_list"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_set_to_list_abi(set: Value) -> Value {
    Value::from_ptr(rt_set_to_list(set.unwrap_ptr()))
}
