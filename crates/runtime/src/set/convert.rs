//! Set conversion operations: to_list

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{ListObj, Obj, SetObj, TypeTagKind, ELEM_HEAP_OBJ, TOMBSTONE};
use std::alloc::{alloc_zeroed, Layout};

/// Convert set to list (for iteration support)
/// Returns: pointer to ListObj containing all set elements
#[no_mangle]
pub extern "C" fn rt_set_to_list(set: *mut Obj) -> *mut Obj {
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

        // Allocate data array (raw allocator — does not trigger GC)
        let data_layout = Layout::array::<*mut Obj>(set_len)
            .expect("Allocation size overflow - capacity too large");
        let data = alloc_zeroed(data_layout) as *mut *mut Obj;

        (*list).elem_tag = ELEM_HEAP_OBJ;
        (*list).len = set_len;
        (*list).capacity = set_len;
        (*list).data = data;

        // Copy non-empty, non-tombstone elements to list
        let mut list_idx = 0;
        for i in 0..capacity {
            let entry = (*set_obj).entries.add(i);
            let elem = (*entry).elem;
            if !elem.is_null() && elem != TOMBSTONE {
                *data.add(list_idx) = elem;
                list_idx += 1;
            }
        }

        list_obj
    }
}
