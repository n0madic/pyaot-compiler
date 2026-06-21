//! Iterator factory functions
//!
//! Creates iterators for various collection types (forward and reversed).

#[allow(unused_imports)]
use crate::debug_assert_dict_family;
#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::dict::rt_dict_keys;
use crate::gc;
use crate::object::{Obj, TypeTagKind};
use pyaot_core_defs::Value;

/// Create a list iterator
/// Returns: pointer to new IteratorObj
pub fn rt_iter_list(list: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_iter_list");
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
#[export_name = "rt_iter_list"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_list_abi(list: Value) -> Value {
    Value::from_ptr(rt_iter_list(list.unwrap_ptr()))
}

/// Create a tuple iterator
/// Returns: pointer to new IteratorObj
pub fn rt_iter_tuple(tuple: *mut Obj) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_iter_tuple");
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
#[export_name = "rt_iter_tuple"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_tuple_abi(tuple: Value) -> Value {
    Value::from_ptr(rt_iter_tuple(tuple.unwrap_ptr()))
}

/// Create a dict key iterator
/// Returns: pointer to new IteratorObj
pub fn rt_iter_dict(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::{IteratorKind, IteratorObj};

    unsafe {
        debug_assert_dict_family!(dict, "rt_iter_dict");
    }

    // Get keys list — this is a gc_alloc. Root it before the next gc_alloc
    // (the iterator allocation below) so GC stress test cannot collect it.
    let keys_list = rt_dict_keys(dict);

    let mut roots: [*mut Obj; 1] = [keys_list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    // SAFETY: frame is a valid stack-allocated ShadowFrame that lives until
    // gc_pop() is called below.
    unsafe { gc_push(&mut frame) };

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    gc_pop();

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
#[export_name = "rt_iter_dict"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_dict_abi(dict: Value) -> Value {
    Value::from_ptr(rt_iter_dict(dict.unwrap_ptr()))
}

/// Create a string iterator
/// Returns: pointer to new IteratorObj
pub fn rt_iter_str(str_obj: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_iter_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_str_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_iter_str(str_obj.unwrap_ptr()))
}

/// Create a range iterator
/// Returns: pointer to new IteratorObj
pub fn rt_iter_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    use crate::object::{IteratorKind, IteratorObj};

    // CPython fidelity: `range(_, _, 0)` raises eagerly at construction. This
    // guards both the general for-loop path and the value form
    // (`list(range(0, 5, 0))`), which share this entry point.
    if step == 0 {
        unsafe {
            use crate::exceptions::ExceptionType;
            raise_exc!(ExceptionType::ValueError, "range() arg 3 must not be zero");
        }
    }

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
#[export_name = "rt_iter_range"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_range_abi(start: i64, stop: i64, step: i64) -> Value {
    Value::from_ptr(rt_iter_range(start, stop, step))
}

/// Create a reversed list iterator
/// Returns: pointer to new IteratorObj starting at end
pub fn rt_iter_reversed_list(list: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_iter_reversed_list"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_reversed_list_abi(list: Value) -> Value {
    Value::from_ptr(rt_iter_reversed_list(list.unwrap_ptr()))
}

/// Create a reversed tuple iterator
/// Returns: pointer to new IteratorObj starting at end
pub fn rt_iter_reversed_tuple(tuple: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_iter_reversed_tuple"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_reversed_tuple_abi(tuple: Value) -> Value {
    Value::from_ptr(rt_iter_reversed_tuple(tuple.unwrap_ptr()))
}

/// Create a reversed string iterator
/// Returns: pointer to new IteratorObj starting at end
pub fn rt_iter_reversed_str(str_obj: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_iter_reversed_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_reversed_str_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_iter_reversed_str(str_obj.unwrap_ptr()))
}

/// Create a reversed dict key iterator
/// Returns: pointer to new IteratorObj starting at end of keys
pub fn rt_iter_reversed_dict(dict: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::{IteratorKind, IteratorObj, ListObj};

    // Get keys list — this is a gc_alloc. Root it before the next gc_alloc
    // (the iterator allocation below) so GC stress test cannot collect it.
    let keys_list = rt_dict_keys(dict);

    let mut roots: [*mut Obj; 1] = [keys_list];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    // SAFETY: frame is a valid stack-allocated ShadowFrame that lives until
    // gc_pop() is called below.
    unsafe { gc_push(&mut frame) };

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    gc_pop();

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
#[export_name = "rt_iter_reversed_dict"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_reversed_dict_abi(dict: Value) -> Value {
    Value::from_ptr(rt_iter_reversed_dict(dict.unwrap_ptr()))
}

/// Create a reversed range iterator
/// reversed(range(start, stop, step)) is equivalent to range(stop-step, start-step, -step)
/// but we need to be careful about the math
/// Returns: pointer to new IteratorObj
pub fn rt_iter_reversed_range(start: i64, stop: i64, step: i64) -> *mut Obj {
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
#[export_name = "rt_iter_reversed_range"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_reversed_range_abi(start: i64, stop: i64, step: i64) -> Value {
    Value::from_ptr(rt_iter_reversed_range(start, stop, step))
}

/// Create an iterator for a generator
/// Generators are their own iterators, so this just returns the generator itself
/// Returns: the same generator object
pub fn rt_iter_generator(gen: *mut Obj) -> *mut Obj {
    gen
}
#[export_name = "rt_iter_generator"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_generator_abi(gen: Value) -> Value {
    Value::from_ptr(rt_iter_generator(gen.unwrap_ptr()))
}

/// Create an iterator for a user-class instance whose class defines
/// `__iter__`/`__next__` (lazy user-class iterator protocol). `source` is the
/// iterator instance — the `__iter__()` result: `self` for a self-iterator, or
/// a separate iterator object. `IterNext` dispatches to the class's compiled
/// `<iternext>` thunk via `INSTANCE_ITERNEXT_REGISTRY`.
///
/// `source` is rooted across the `IteratorObj` allocation: the `__iter__()`
/// result may be a freshly-allocated instance not yet held in any GC-rooted
/// slot, so a stress-test GC during the alloc below could otherwise free it
/// (the same self-rooting `rt_iter_dict`/`rt_iter_deque` do for their derived
/// sources).
fn rt_iter_instance(source: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::{IteratorKind, IteratorObj};

    let mut roots: [*mut Obj; 1] = [source];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    let size = std::mem::size_of::<IteratorObj>();
    let obj = gc::gc_alloc(size, TypeTagKind::Iterator as u8);

    gc_pop();

    unsafe {
        let iter = obj as *mut IteratorObj;
        (*iter).kind = IteratorKind::Instance as u8;
        (*iter).exhausted = false;
        (*iter).reversed = false;
        (*iter).source = roots[0];
        (*iter).index = 0;
        (*iter).range_stop = 0;
        (*iter).range_step = 0;
    }

    obj
}

/// Create an iterator for a dynamically-typed Value (runtime type dispatch).
/// Used when the iterable has `Any`/`HeapAny` type at compile time.
pub fn rt_iter_value_dyn(obj: *mut Obj) -> *mut Obj {
    let type_tag = unsafe { (*obj).header.type_tag };
    match type_tag {
        TypeTagKind::List => rt_iter_list(obj),
        TypeTagKind::Tuple => rt_iter_tuple(obj),
        // The dict family (Dict / DefaultDict / Counter) shares `DictObj` layout;
        // iterating any of them yields its keys.
        TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter => rt_iter_dict(obj),
        // FrozenSet shares `SetObj` layout, so the set iterator drives it too.
        TypeTagKind::Set | TypeTagKind::FrozenSet => rt_iter_set(obj),
        // A deque is a ring buffer, not an `IteratorObj`; `rt_iter_deque`
        // materializes a left-to-right list snapshot and iterates that (the same
        // derived-list strategy set/dict use), so `for`/`list`/`sum`/`",".join`
        // over a deque all work through this one generic seam.
        TypeTagKind::Deque => rt_iter_deque(obj),
        TypeTagKind::Str => rt_iter_str(obj),
        TypeTagKind::Bytes => rt_iter_bytes(obj),
        TypeTagKind::ByteArray => crate::bytearray::rt_iter_bytearray(obj),
        TypeTagKind::Iterator | TypeTagKind::Generator => obj,
        // A user-class instance: `for x in inst` / `iter(inst)` where the class
        // defines `__iter__`. Dispatch `__iter__()` to obtain the iterator, then
        // wrap it. The tag-check on the result avoids infinite recursion: a
        // self-iterator returns its own Instance (first arm), while a built-in
        // iterable (list / generator / …) recurses through the generic path.
        TypeTagKind::Instance => unsafe {
            use crate::gc::{gc_pop, gc_push, ShadowFrame};
            use crate::ops::try_iter_dunder;
            match try_iter_dunder(obj) {
                Some(it) => {
                    // Root `it` (a possibly-fresh `__iter__()` result) across the
                    // iterator construction below; both arms gc_alloc.
                    let mut roots: [*mut Obj; 1] = [it];
                    let mut frame = ShadowFrame {
                        prev: std::ptr::null_mut(),
                        nroots: 1,
                        roots: roots.as_mut_ptr(),
                    };
                    gc_push(&mut frame);
                    let result = if (*it).header.type_tag == TypeTagKind::Instance {
                        rt_iter_instance(it)
                    } else {
                        rt_iter_value_dyn(it)
                    };
                    gc_pop();
                    result
                }
                None => {
                    use crate::exceptions::ExceptionType;
                    raise_exc!(
                        ExceptionType::TypeError,
                        "'{}' object is not iterable",
                        type_tag.type_name()
                    );
                }
            }
        },
        _ => unsafe {
            use crate::exceptions::ExceptionType;
            raise_exc!(
                ExceptionType::TypeError,
                "'{}' object is not iterable",
                type_tag.type_name()
            );
        },
    }
}
#[export_name = "rt_iter_value"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_value_abi(val: Value) -> Value {
    if !val.is_ptr() {
        unsafe {
            use crate::exceptions::ExceptionType;
            raise_exc!(ExceptionType::TypeError, "object is not iterable");
        }
    }
    Value::from_ptr(rt_iter_value_dyn(val.unwrap_ptr()))
}

/// Create an enumerate iterator wrapping an inner iterator
/// Returns: pointer to new IteratorObj with Enumerate kind
pub fn rt_iter_enumerate(inner_iter: *mut Obj, start: i64) -> *mut Obj {
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
#[export_name = "rt_iter_enumerate"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_enumerate_abi(inner_iter: Value, start: i64) -> Value {
    Value::from_ptr(rt_iter_enumerate(inner_iter.unwrap_ptr(), start))
}

/// Create a set iterator
/// Returns: pointer to IteratorObj
pub fn rt_iter_set(set: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_iter_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_set_abi(set: Value) -> Value {
    Value::from_ptr(rt_iter_set(set.unwrap_ptr()))
}

/// Create a bytes iterator
/// Returns: pointer to new IteratorObj
pub fn rt_iter_bytes(bytes: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_iter_bytes"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_bytes_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_iter_bytes(bytes.unwrap_ptr()))
}

/// Create a reversed bytes iterator
/// Returns: pointer to new IteratorObj starting at end
pub fn rt_iter_reversed_bytes(bytes: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_iter_reversed_bytes"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_reversed_bytes_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_iter_reversed_bytes(bytes.unwrap_ptr()))
}

/// Create a deque iterator.
///
/// A `DequeObj` is a ring buffer, not an `IteratorObj`, so we materialize a
/// left-to-right list snapshot and iterate that — the same "iterate a derived
/// list" strategy used for set/dict. The snapshot is rooted across the
/// iterator allocation (`rt_iter_list` calls `gc_alloc`) so a stress-test GC
/// cannot free it before the iterator's `source` field pins it.
pub fn rt_iter_deque(deque: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    let snapshot = crate::deque::rt_list_from_deque(deque);

    let mut roots: [*mut Obj; 1] = [snapshot];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    let iter = rt_iter_list(roots[0]);

    gc_pop();
    iter
}
#[export_name = "rt_iter_deque"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_deque_abi(deque: Value) -> Value {
    Value::from_ptr(rt_iter_deque(deque.unwrap_ptr()))
}

/// Create a reversed deque iterator (snapshot list iterated end-to-front).
/// See `rt_iter_deque` for the snapshot/rooting rationale.
pub fn rt_iter_reversed_deque(deque: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    let snapshot = crate::deque::rt_list_from_deque(deque);

    let mut roots: [*mut Obj; 1] = [snapshot];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    unsafe { gc_push(&mut frame) };

    let iter = rt_iter_reversed_list(roots[0]);

    gc_pop();
    iter
}
#[export_name = "rt_iter_reversed_deque"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_iter_reversed_deque_abi(deque: Value) -> Value {
    Value::from_ptr(rt_iter_reversed_deque(deque.unwrap_ptr()))
}
