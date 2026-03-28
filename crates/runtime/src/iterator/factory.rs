//! Iterator factory functions
//!
//! Creates iterators for various collection types (forward and reversed).

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::dict::rt_dict_keys;
use crate::gc;
use crate::object::{Obj, TypeTagKind};

/// Create a list iterator
/// Returns: pointer to new IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_list(list: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::List as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = list;
        (*iter).index = 0;
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a tuple iterator
/// Returns: pointer to new IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_tuple(tuple: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Tuple as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = tuple;
        (*iter).index = 0;
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a dict key iterator
/// Returns: pointer to new IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_dict(dict: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    // Get keys list and iterate over that
    let keys_list = rt_dict_keys(dict, crate::object::ELEM_HEAP_OBJ);

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Dict as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = keys_list; // Store keys list instead of dict
        (*iter).index = 0;
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a string iterator
/// Returns: pointer to new IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::String as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = str_obj;
        (*iter).index = 0;
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a range iterator
/// Returns: pointer to new IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Range as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = std::ptr::null_mut();
        (*iter).index = start;
        (*iter).range_stop = stop;
        (*iter).range_step = step;
    }

    obj
}

/// Create a reversed list iterator
/// Returns: pointer to new IteratorObj starting at end
#[no_mangle]
pub extern "C" fn rt_iter_reversed_list(list: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj, ListObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_iter_reversed_list");
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len as i64;

        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::List as u8;
        (*iter).exhausted = false;
        (*iter).reversed = true;
        (*iter).source = list;
        (*iter).index = len - 1; // Start at last element
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a reversed tuple iterator
/// Returns: pointer to new IteratorObj starting at end
#[no_mangle]
pub extern "C" fn rt_iter_reversed_tuple(tuple: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj, TupleObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_iter_reversed_tuple");
        let tuple_obj = tuple as *mut TupleObj;
        let len = (*tuple_obj).len as i64;

        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Tuple as u8;
        (*iter).exhausted = false;
        (*iter).reversed = true;
        (*iter).source = tuple;
        (*iter).index = len - 1; // Start at last element
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a reversed string iterator
/// Returns: pointer to new IteratorObj starting at end
#[no_mangle]
pub extern "C" fn rt_iter_reversed_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj, StrObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_iter_reversed_str");
        let s = str_obj as *mut StrObj;
        let byte_len = (*s).len as i64;

        // Walk backwards from the last byte to find the start of the last codepoint.
        // UTF-8 continuation bytes have the pattern 10xxxxxx (0x80..=0xBF).
        let start_idx = if byte_len == 0 {
            -1 // Empty string — mark exhausted below
        } else {
            let data = (*s).data.as_ptr();
            let mut idx = byte_len - 1;
            while idx > 0 && (*data.add(idx as usize) & 0xC0) == 0x80 {
                idx -= 1;
            }
            idx
        };

        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::String as u8;
        (*iter).exhausted = byte_len == 0;
        (*iter).reversed = true;
        (*iter).source = str_obj;
        (*iter).index = start_idx; // Byte offset of the last codepoint's first byte
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a reversed dict key iterator
/// Returns: pointer to new IteratorObj starting at end of keys
#[no_mangle]
pub extern "C" fn rt_iter_reversed_dict(dict: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj, ListObj};

    // Get keys list and iterate over that in reverse
    let keys_list = rt_dict_keys(dict, crate::object::ELEM_HEAP_OBJ);

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let keys = keys_list as *mut ListObj;
        let len = (*keys).len as i64;

        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Dict as u8;
        (*iter).exhausted = false;
        (*iter).reversed = true;
        (*iter).source = keys_list; // Store keys list instead of dict
        (*iter).index = len - 1; // Start at last key
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a reversed range iterator
/// reversed(range(start, stop, step)) is equivalent to range(stop-step, start-step, -step)
/// but we need to be careful about the math
/// Returns: pointer to new IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_reversed_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Range as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false; // We compute the reversed range directly

        // For reversed range, compute the actual start value
        // e.g., reversed(range(0, 5, 1)) = [4, 3, 2, 1, 0]
        // We need to find the last value that range would produce
        if step > 0 {
            // Positive step: start < stop
            if start >= stop {
                // Empty range
                (*iter).exhausted = true;
                (*iter).index = 0;
                (*iter).range_stop = 0;
                (*iter).range_step = 0;
            } else {
                // Last value is: start + ((stop - start - 1) / step) * step
                let count = (stop - start - 1) / step + 1;
                let last_value = start + (count - 1) * step;
                (*iter).index = last_value;
                (*iter).range_stop = start - step; // Stop before start
                (*iter).range_step = -step; // Negative step
            }
        } else if step < 0 {
            // Negative step: start > stop
            if start <= stop {
                // Empty range
                (*iter).exhausted = true;
                (*iter).index = 0;
                (*iter).range_stop = 0;
                (*iter).range_step = 0;
            } else {
                // Last value is: start + ((stop - start + 1) / step) * step
                let count = (start - stop - 1) / (-step) + 1;
                let last_value = start + (count - 1) * step;
                (*iter).index = last_value;
                (*iter).range_stop = start - step; // Stop before start
                (*iter).range_step = -step; // Positive step (reversed)
            }
        } else {
            // step == 0 is invalid, mark as exhausted
            (*iter).exhausted = true;
            (*iter).index = 0;
            (*iter).range_stop = 0;
            (*iter).range_step = 0;
        }

        (*iter).source = std::ptr::null_mut();
    }

    obj
}

/// Create an iterator for a generator
/// Generators are their own iterators, so this just returns the generator itself
/// Returns: the same generator object
#[no_mangle]
pub extern "C" fn rt_iter_generator(gen: *mut Obj) -> *mut Obj {
    gen
}

/// Create an enumerate iterator wrapping an inner iterator
/// Returns: pointer to new IteratorObj with Enumerate kind
#[no_mangle]
pub extern "C" fn rt_iter_enumerate(inner_iter: *mut Obj, start: i64) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Enumerate as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = inner_iter; // Inner iterator
        (*iter).index = start; // Counter starts at `start`
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a set iterator
/// Returns: pointer to IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_set(set: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Set as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = set;
        (*iter).index = 0; // Start at first slot
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a bytes iterator
/// Returns: pointer to new IteratorObj
#[no_mangle]
pub extern "C" fn rt_iter_bytes(bytes: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Bytes as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = bytes;
        (*iter).index = 0;
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create a reversed bytes iterator
/// Returns: pointer to new IteratorObj starting at end
#[no_mangle]
pub extern "C" fn rt_iter_reversed_bytes(bytes: *mut Obj) -> *mut Obj {
    use crate::object::{BytesObj, IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        debug_assert_type_tag!(bytes, TypeTagKind::Bytes, "rt_iter_reversed_bytes");
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len as i64;

        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Bytes as u8;
        (*iter).exhausted = false;
        (*iter).reversed = true;
        (*iter).source = bytes;
        (*iter).index = len - 1; // Start at last byte
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}
