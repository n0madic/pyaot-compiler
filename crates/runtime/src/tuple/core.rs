//! Core tuple operations: creation, get, set, len, slice, concat, from_* conversions

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{Obj, TypeTagKind, ELEM_HEAP_OBJ};
use crate::slice_utils::{collect_step_indices, normalize_slice_indices, slice_length};

/// Create a new tuple with given size and element tag
/// elem_tag: ELEM_HEAP_OBJ (0), ELEM_RAW_INT (1), or ELEM_RAW_BOOL (2)
/// Returns: pointer to allocated TupleObj
#[no_mangle]
pub extern "C" fn rt_make_tuple(size: i64, elem_tag: u8) -> *mut Obj {
    use crate::object::{TupleObj, TypeTagKind};

    let size = size.max(0) as usize;

    // Calculate size: base struct size + inline data array
    // TupleObj has: ObjHeader(16) + len(8) + elem_tag(1) + padding(7) + data[0]
    // Use size_of::<TupleObj> for the base size (includes alignment padding)
    let data_size = match size.checked_mul(std::mem::size_of::<*mut Obj>()) {
        Some(s) => s,
        None => {
            let msg = b"MemoryError: tuple too large";
            unsafe {
                crate::exceptions::rt_exc_raise(
                    crate::exceptions::ExceptionType::MemoryError as u8,
                    msg.as_ptr(),
                    msg.len(),
                )
            }
        }
    };
    let tuple_size = match std::mem::size_of::<TupleObj>().checked_add(data_size) {
        Some(s) => s,
        None => {
            let msg = b"MemoryError: tuple too large";
            unsafe {
                crate::exceptions::rt_exc_raise(
                    crate::exceptions::ExceptionType::MemoryError as u8,
                    msg.as_ptr(),
                    msg.len(),
                )
            }
        }
    };

    // Allocate TupleObj using GC
    let obj = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8);

    unsafe {
        let tuple = obj as *mut TupleObj;
        (*tuple).len = size;
        (*tuple).elem_tag = elem_tag;
        // Default heap_field_mask: all fields are heap objects when ELEM_HEAP_OBJ,
        // no fields are heap objects when ELEM_RAW_INT/ELEM_RAW_BOOL.
        (*tuple).heap_field_mask = if elem_tag == ELEM_HEAP_OBJ {
            u64::MAX
        } else {
            0
        };

        // Initialize all elements to null
        let data_ptr = (*tuple).data.as_mut_ptr();
        for i in 0..size {
            *data_ptr.add(i) = std::ptr::null_mut();
        }
    }

    obj
}

/// Set the heap_field_mask on a tuple.
/// Called after rt_make_tuple when the caller knows per-field GC tracing info.
/// mask: bitmask where bit i = 1 means field i is a heap pointer.
#[no_mangle]
pub extern "C" fn rt_tuple_set_heap_mask(tuple: *mut Obj, mask: i64) {
    if tuple.is_null() {
        return;
    }
    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        (*tuple_obj).heap_field_mask = mask as u64;
    }
}

/// Set element in tuple at given index (used during tuple construction)
#[no_mangle]
pub extern "C" fn rt_tuple_set(tuple: *mut Obj, index: i64, value: *mut Obj) {
    if tuple.is_null() {
        return;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_set");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Only positive indices during construction
        if index < 0 || index >= len {
            return;
        }

        // Note: no validate_elem_tag! for tuples — the GC uses heap_field_mask for
        // precise per-field tracing, making the elem_tag-based validation unnecessary.
        // Mixed-type tuples (captures, *args) are safely handled by the mask.

        let data_ptr = (*tuple_obj).data.as_mut_ptr();
        *data_ptr.add(index as usize) = value;
    }
}

/// Get element from tuple at given index
/// Supports negative indexing
/// Returns: pointer to element or null if out of bounds
#[no_mangle]
pub extern "C" fn rt_tuple_get(tuple: *mut Obj, index: i64) -> *mut Obj {
    if tuple.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return std::ptr::null_mut();
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        *data_ptr.add(idx as usize)
    }
}

/// Get length of tuple
#[no_mangle]
pub extern "C" fn rt_tuple_len(tuple: *mut Obj) -> i64 {
    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_len");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        (*tuple_obj).len as i64
    }
}

/// Slice a tuple: tuple[start:end]
/// Negative indices are supported (counted from end)
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated TupleObj (shallow copy)
#[no_mangle]
pub extern "C" fn rt_tuple_slice(tuple: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if tuple.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_slice");
        let src = tuple as *mut crate::object::TupleObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility (step=1 for simple slice)
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);

        // Root the source tuple across rt_make_tuple which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [tuple];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Read elem_tag from the (still live) source before allocation.
        let elem_tag = (*(roots[0] as *mut crate::object::TupleObj)).elem_tag;
        let new_tuple = rt_make_tuple(slice_len as i64, elem_tag);

        gc_pop();

        let src = tuple as *mut crate::object::TupleObj;
        let new_tuple_obj = new_tuple as *mut crate::object::TupleObj;

        if slice_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_tuple_obj).data.as_mut_ptr();

            // Copy element pointers (shallow copy)
            for i in 0..slice_len {
                *dst_data.add(i) = *src_data.add(start as usize + i);
            }

            // Propagate heap_field_mask: extract bits [start..start+slice_len) from the
            // source mask and place them at [0..slice_len) in the new tuple's mask.
            // A right-shift by `start` aligns the relevant bits to position 0.
            let src_shift = start as u32;
            let shifted = (*src).heap_field_mask >> src_shift;
            // Build a mask covering exactly slice_len bits to clear any bits beyond the slice.
            let keep_mask = if slice_len >= 64 {
                u64::MAX
            } else {
                (1u64 << slice_len) - 1
            };
            (*new_tuple_obj).heap_field_mask = shifted & keep_mask;
        }

        new_tuple
    }
}

/// Slice a tuple and return as a list: used for starred unpacking
/// In Python, `a, *rest = (1, 2, 3)` makes rest a list, not a tuple
/// Negative indices are supported
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated ListObj (shallow copy of elements)
#[no_mangle]
pub extern "C" fn rt_tuple_slice_to_list(tuple: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::list::rt_make_list;

    if tuple.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_slice_to_list");
        let src = tuple as *mut crate::object::TupleObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility (step=1 for simple slice)
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);

        // Root the source tuple across rt_make_list which calls gc_alloc.
        let mut roots: [*mut Obj; 1] = [tuple];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let elem_tag = (*(roots[0] as *mut crate::object::TupleObj)).elem_tag;
        let new_list = rt_make_list(slice_len as i64, elem_tag);

        gc_pop();

        let src = tuple as *mut crate::object::TupleObj;
        let new_list_obj = new_list as *mut crate::object::ListObj;

        if slice_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_list_obj).data;

            // Copy element pointers (shallow copy)
            for i in 0..slice_len {
                *dst_data.add(i) = *src_data.add(start as usize + i);
            }
            // Set the actual length
            (*new_list_obj).len = slice_len;
        }

        new_list
    }
}

/// Slice a tuple with step: tuple[start:end:step]
/// Uses i64::MIN as sentinel for "default start" and i64::MAX for "default end"
/// Defaults depend on step direction:
///   - Positive step: start=0, end=len
///   - Negative step: start=len-1, end=-1 (before index 0)
///
/// Returns: pointer to new allocated TupleObj (shallow copy)
#[no_mangle]
pub extern "C" fn rt_tuple_slice_step(
    tuple: *mut Obj,
    start: i64,
    end: i64,
    step: i64,
) -> *mut Obj {
    if tuple.is_null() || step == 0 {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = tuple as *mut crate::object::TupleObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility
        let (start, end) = normalize_slice_indices(start, end, len, step);

        // Collect indices using shared utility
        let indices = collect_step_indices(start, end, step);
        let result_len = indices.len();
        let new_tuple = rt_make_tuple(result_len as i64, (*src).elem_tag);
        let new_tuple_obj = new_tuple as *mut crate::object::TupleObj;

        if result_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_tuple_obj).data.as_mut_ptr();

            for (dst_i, src_i) in indices.iter().enumerate() {
                *dst_data.add(dst_i) = *src_data.add(*src_i);
            }
        }

        new_tuple
    }
}

/// Get integer element from tuple, unboxing if necessary
/// Handles both raw integer storage and boxed IntObj storage transparently
#[no_mangle]
pub extern "C" fn rt_tuple_get_int(tuple: *mut Obj, index: i64) -> i64 {
    use crate::object::{IntObj, ELEM_HEAP_OBJ, ELEM_RAW_INT};

    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get_int");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return 0;
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        let elem = *data_ptr.add(idx as usize);
        let elem_tag = (*tuple_obj).elem_tag;

        match elem_tag {
            ELEM_RAW_INT => {
                // Element is stored as raw i64
                elem as i64
            }
            ELEM_HEAP_OBJ => {
                // Element is boxed - unbox it
                if elem.is_null() {
                    return 0;
                }
                let int_obj = elem as *mut IntObj;
                (*int_obj).value
            }
            _ => {
                // Unknown tag, treat as raw
                elem as i64
            }
        }
    }
}

/// Get float element from tuple, unboxing if necessary
/// Handles both raw float storage (as bitcast i64) and boxed FloatObj storage
#[no_mangle]
pub extern "C" fn rt_tuple_get_float(tuple: *mut Obj, index: i64) -> f64 {
    use crate::object::{FloatObj, ELEM_HEAP_OBJ};

    if tuple.is_null() {
        return 0.0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get_float");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return 0.0;
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        let elem = *data_ptr.add(idx as usize);
        let elem_tag = (*tuple_obj).elem_tag;

        match elem_tag {
            ELEM_HEAP_OBJ => {
                // Element is boxed - unbox it
                if elem.is_null() {
                    return 0.0;
                }
                let float_obj = elem as *mut FloatObj;
                (*float_obj).value
            }
            _ => {
                // Raw storage: element is f64 bitcast to pointer
                f64::from_bits(elem as u64)
            }
        }
    }
}

/// Get bool element from tuple, unboxing if necessary
/// Handles both raw bool storage (as i8 cast to pointer) and boxed BoolObj storage
#[no_mangle]
pub extern "C" fn rt_tuple_get_bool(tuple: *mut Obj, index: i64) -> i8 {
    use crate::object::{BoolObj, ELEM_HEAP_OBJ, ELEM_RAW_BOOL};

    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_get_bool");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let len = (*tuple_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        // Bounds check
        if idx < 0 || idx >= len {
            return 0;
        }

        let data_ptr = (*tuple_obj).data.as_ptr();
        let elem = *data_ptr.add(idx as usize);
        let elem_tag = (*tuple_obj).elem_tag;

        match elem_tag {
            ELEM_RAW_BOOL => {
                // Element is stored as raw i8
                elem as i8
            }
            ELEM_HEAP_OBJ => {
                // Element is boxed - unbox it
                if elem.is_null() {
                    return 0;
                }
                let bool_obj = elem as *mut BoolObj;
                if (*bool_obj).value {
                    1
                } else {
                    0
                }
            }
            _ => {
                // Unknown tag, treat as raw
                elem as i8
            }
        }
    }
}

/// Create a tuple from a list
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_list(list: *mut Obj) -> *mut Obj {
    use crate::object::ListObj;

    if list.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;
        let elem_tag = (*list_obj).elem_tag;

        let tuple = rt_make_tuple(len as i64, elem_tag);
        let tuple_obj = tuple as *mut crate::object::TupleObj;

        if len > 0 {
            let src_data = (*list_obj).data;
            let dst_data = (*tuple_obj).data.as_mut_ptr();

            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
        }

        tuple
    }
}

/// Create a tuple from a string (each character becomes an element)
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::StrObj;
    use crate::string::rt_make_str;

    if str_obj.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let str = str_obj as *mut StrObj;
        let len = (*str).len;
        let data = (*str).data.as_ptr();

        let tuple = rt_make_tuple(len as i64, ELEM_HEAP_OBJ);

        if len == 0 {
            return tuple;
        }

        // Root tuple while rt_make_str allocates one string per character
        let mut roots: [*mut Obj; 1] = [tuple];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        for i in 0..len {
            let ch = *data.add(i);
            // Create single-character string
            let char_str = rt_make_str(&ch, 1);
            rt_tuple_set(roots[0], i as i64, char_str);
        }

        gc_pop();

        roots[0]
    }
}

/// Create a tuple from a range
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    use crate::object::ELEM_RAW_INT;

    if step == 0 {
        return rt_make_tuple(0, ELEM_RAW_INT);
    }

    let len = if step > 0 {
        if stop > start {
            ((stop - start + step - 1) / step) as usize
        } else {
            0
        }
    } else if start > stop {
        ((start - stop - step - 1) / (-step)) as usize
    } else {
        0
    };

    let tuple = rt_make_tuple(len as i64, ELEM_RAW_INT);

    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let data = (*tuple_obj).data.as_mut_ptr();

        let mut current = start;
        for i in 0..len {
            *data.add(i) = current as *mut Obj;
            current += step;
        }
    }

    tuple
}

/// Create a tuple by consuming an iterator
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_iter(iter: *mut Obj) -> *mut Obj {
    use crate::iterator::rt_iter_next_no_exc;
    use crate::list::{rt_list_push, rt_make_list};

    if iter.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }

    // First collect into a list (since we don't know the size)
    let list = rt_make_list(8, ELEM_HEAP_OBJ);

    loop {
        let elem = rt_iter_next_no_exc(iter);
        if elem.is_null() {
            break;
        }
        rt_list_push(list, elem);
    }

    // Convert list to tuple
    rt_tuple_from_list(list)
}

/// Create a tuple from a set
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_set(set: *mut Obj) -> *mut Obj {
    use crate::set::rt_set_to_list;

    // First convert set to list, then list to tuple
    let list = rt_set_to_list(set);
    rt_tuple_from_list(list)
}

/// Create a tuple from a dict (keys only)
/// Returns: pointer to new TupleObj
#[no_mangle]
pub extern "C" fn rt_tuple_from_dict(dict: *mut Obj) -> *mut Obj {
    use crate::dict::rt_dict_keys;

    // First get keys as list, then convert to tuple
    let list = rt_dict_keys(dict, crate::object::ELEM_HEAP_OBJ);
    rt_tuple_from_list(list)
}

/// Concatenate two tuples into a new tuple
/// Used for combining extra positional args with list-unpacked varargs
/// Returns: pointer to new TupleObj containing elements from tuple1 followed by tuple2
#[no_mangle]
pub extern "C" fn rt_tuple_concat(tuple1: *mut Obj, tuple2: *mut Obj) -> *mut Obj {
    use crate::object::TupleObj;

    // Handle null cases
    if tuple1.is_null() && tuple2.is_null() {
        return rt_make_tuple(0, ELEM_HEAP_OBJ);
    }
    if tuple1.is_null() {
        return tuple2;
    }
    if tuple2.is_null() {
        return tuple1;
    }

    unsafe {
        let t1 = tuple1 as *mut TupleObj;
        let t2 = tuple2 as *mut TupleObj;
        let len1 = (*t1).len;
        let len2 = (*t2).len;
        let total_len = len1 + len2;

        // Use elem_tag from the first tuple (or HEAP_OBJ if first is empty)
        let elem_tag = if len1 > 0 {
            (*t1).elem_tag
        } else {
            (*t2).elem_tag
        };

        // Create new tuple
        let result = rt_make_tuple(total_len as i64, elem_tag);
        let result_obj = result as *mut TupleObj;

        // Copy elements from tuple1
        if len1 > 0 {
            let src_data = (*t1).data.as_ptr();
            let dst_data = (*result_obj).data.as_mut_ptr();
            for i in 0..len1 {
                *dst_data.add(i) = *src_data.add(i);
            }
        }

        // Copy elements from tuple2
        if len2 > 0 {
            let src_data = (*t2).data.as_ptr();
            let dst_data = (*result_obj).data.as_mut_ptr();
            for i in 0..len2 {
                *dst_data.add(len1 + i) = *src_data.add(i);
            }
        }

        result
    }
}

/// Maximum number of arguments supported for *args forwarding via indirect call.
const MAX_CALL_ARGS: usize = 8;

/// Call a function pointer with arguments unpacked from a tuple.
/// Used for *args forwarding in decorator wrappers: `func(*args)`.
///
/// All arguments are passed as i64 (raw ints stay as i64, heap objects as pointers cast to i64).
/// The function pointer must use the SystemV calling convention.
#[no_mangle]
pub extern "C" fn rt_call_with_tuple_args(func_ptr: i64, args_tuple: *mut Obj) -> i64 {
    use crate::object::TupleObj;

    if func_ptr == 0 {
        return 0;
    }

    unsafe {
        let len = if args_tuple.is_null() {
            0
        } else {
            let tuple_obj = args_tuple as *mut TupleObj;
            (*tuple_obj).len
        };

        // Extract arguments from tuple
        let mut call_args = [0i64; MAX_CALL_ARGS];
        if !args_tuple.is_null() && len > 0 {
            let tuple_obj = args_tuple as *mut TupleObj;
            let data_ptr = (*tuple_obj).data.as_ptr();
            for (slot, i) in (0..len.min(MAX_CALL_ARGS)).enumerate() {
                call_args[slot] = *data_ptr.add(i) as i64;
            }
        }

        // Dispatch based on argument count
        type F0 = extern "C" fn() -> i64;
        type F1 = extern "C" fn(i64) -> i64;
        type F2 = extern "C" fn(i64, i64) -> i64;
        type F3 = extern "C" fn(i64, i64, i64) -> i64;
        type F4 = extern "C" fn(i64, i64, i64, i64) -> i64;
        type F5 = extern "C" fn(i64, i64, i64, i64, i64) -> i64;
        type F6 = extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64;
        type F7 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64;
        type F8 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64;

        match len {
            0 => {
                let f: F0 = std::mem::transmute(func_ptr as usize);
                f()
            }
            1 => {
                let f: F1 = std::mem::transmute(func_ptr as usize);
                f(call_args[0])
            }
            2 => {
                let f: F2 = std::mem::transmute(func_ptr as usize);
                f(call_args[0], call_args[1])
            }
            3 => {
                let f: F3 = std::mem::transmute(func_ptr as usize);
                f(call_args[0], call_args[1], call_args[2])
            }
            4 => {
                let f: F4 = std::mem::transmute(func_ptr as usize);
                f(call_args[0], call_args[1], call_args[2], call_args[3])
            }
            5 => {
                let f: F5 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                )
            }
            6 => {
                let f: F6 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                    call_args[5],
                )
            }
            7 => {
                let f: F7 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                    call_args[5],
                    call_args[6],
                )
            }
            8 => {
                let f: F8 = std::mem::transmute(func_ptr as usize);
                f(
                    call_args[0],
                    call_args[1],
                    call_args[2],
                    call_args[3],
                    call_args[4],
                    call_args[5],
                    call_args[6],
                    call_args[7],
                )
            }
            _ => 0, // Unsupported arity
        }
    }
}
