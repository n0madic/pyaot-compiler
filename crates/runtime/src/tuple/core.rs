//! Core tuple operations: creation, get, set, len, slice, concat, from_* conversions

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::gc;
use crate::object::{Obj, TypeTagKind};
use crate::slice_utils::{collect_step_indices, normalize_slice_indices, slice_length};
use pyaot_core_defs::Value;

/// Create a new tuple with given size.
/// Returns: pointer to allocated TupleObj
pub fn rt_make_tuple(size: i64) -> *mut Obj {
    use crate::object::{TupleObj, TypeTagKind};

    let size = size.max(0) as usize;

    let data_size = match size.checked_mul(std::mem::size_of::<*mut Obj>()) {
        Some(s) => s,
        None => unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::MemoryError,
                "tuple too large"
            )
        },
    };
    let tuple_size = match std::mem::size_of::<TupleObj>().checked_add(data_size) {
        Some(s) => s,
        None => unsafe {
            raise_exc!(
                crate::exceptions::ExceptionType::MemoryError,
                "tuple too large"
            )
        },
    };

    // Allocate TupleObj using GC
    let obj = gc::gc_alloc(tuple_size, TypeTagKind::Tuple as u8);

    unsafe {
        let tuple = obj as *mut TupleObj;
        (*tuple).len = size;

        // Initialize all elements to Value(0) (null pointer encoding)
        let data_ptr = (*tuple).data.as_mut_ptr();
        for i in 0..size {
            *data_ptr.add(i) = pyaot_core_defs::Value(0);
        }
    }

    obj
}
#[export_name = "rt_make_tuple"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_tuple_abi(size: i64) -> Value {
    Value::from_ptr(rt_make_tuple(size))
}

/// Set element in tuple at given index (used during tuple construction)
pub fn rt_tuple_set(tuple: *mut Obj, index: i64, value: *mut Obj) {
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

        let data_ptr = (*tuple_obj).data.as_mut_ptr();
        // After F.7c values arrive as tagged Value bit-patterns; identity store.
        *data_ptr.add(index as usize) = pyaot_core_defs::Value(value as u64);
    }
}
#[export_name = "rt_tuple_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_set_abi(tuple: Value, index: i64, value: Value) {
    rt_tuple_set(tuple.unwrap_ptr(), index, value.unwrap_ptr())
}

/// Get element from tuple at given index.
/// Supports negative indexing.
/// Returns: the raw tagged `Value` bits as `*mut Obj`. After §F.4, the
/// caller is responsible for unboxing (UnwrapValueInt / UnwrapValueBool /
/// rt_unbox_float) based on the statically-known element type.
/// Returns null if out of bounds.
pub fn rt_tuple_get(tuple: *mut Obj, index: i64) -> *mut Obj {
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
        let val = *data_ptr.add(idx as usize);
        // Always return the raw Value bits; lowering applies unboxing at call-site.
        val.0 as *mut Obj
    }
}
#[export_name = "rt_tuple_get"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_get_abi(tuple: Value, index: i64) -> Value {
    Value::from_ptr(rt_tuple_get(tuple.unwrap_ptr(), index))
}

/// Get length of tuple
pub fn rt_tuple_len(tuple: *mut Obj) -> i64 {
    if tuple.is_null() {
        return 0;
    }

    unsafe {
        debug_assert_type_tag!(tuple, TypeTagKind::Tuple, "rt_tuple_len");
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        (*tuple_obj).len as i64
    }
}
#[export_name = "rt_tuple_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_len_abi(tuple: Value) -> i64 {
    rt_tuple_len(tuple.unwrap_ptr())
}

/// Slice a tuple: tuple[start:end]
/// Negative indices are supported (counted from end)
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated TupleObj (shallow copy)
pub fn rt_tuple_slice(tuple: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if tuple.is_null() {
        return rt_make_tuple(0);
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

        let new_tuple = rt_make_tuple(slice_len as i64);

        gc_pop();

        let src = tuple as *mut crate::object::TupleObj;
        let new_tuple_obj = new_tuple as *mut crate::object::TupleObj;

        if slice_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_tuple_obj).data.as_mut_ptr();

            // Copy element Values (shallow copy)
            for i in 0..slice_len {
                *dst_data.add(i) = *src_data.add(start as usize + i);
            }
        }

        new_tuple
    }
}
#[export_name = "rt_tuple_slice"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_slice_abi(tuple: Value, start: i64, end: i64) -> Value {
    Value::from_ptr(rt_tuple_slice(tuple.unwrap_ptr(), start, end))
}

/// Slice a tuple and return as a list: used for starred unpacking
/// In Python, `a, *rest = (1, 2, 3)` makes rest a list, not a tuple
/// Negative indices are supported
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated ListObj (shallow copy of elements)
pub fn rt_tuple_slice_to_list(tuple: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::list::rt_make_list;

    if tuple.is_null() {
        return rt_make_list(0);
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

        let new_list = rt_make_list(slice_len as i64);

        gc_pop();

        let src = tuple as *mut crate::object::TupleObj;
        let new_list_obj = new_list as *mut crate::object::ListObj;

        if slice_len > 0 {
            let src_data = (*src).data.as_ptr();
            let dst_data = (*new_list_obj).data;

            // Both tuple and list now use Value storage — direct copy.
            for i in 0..slice_len {
                *dst_data.add(i) = *src_data.add(start as usize + i);
            }
            // Set the actual length
            (*new_list_obj).len = slice_len;
        }

        new_list
    }
}
#[export_name = "rt_tuple_slice_to_list"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_slice_to_list_abi(tuple: Value, start: i64, end: i64) -> Value {
    Value::from_ptr(rt_tuple_slice_to_list(tuple.unwrap_ptr(), start, end))
}

/// Slice a tuple with step: tuple[start:end:step]
/// Uses i64::MIN as sentinel for "default start" and i64::MAX for "default end"
/// Defaults depend on step direction:
///   - Positive step: start=0, end=len
///   - Negative step: start=len-1, end=-1 (before index 0)
///
/// Returns: pointer to new allocated TupleObj (shallow copy)
pub fn rt_tuple_slice_step(tuple: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj {
    if tuple.is_null() || step == 0 {
        return rt_make_tuple(0);
    }

    unsafe {
        let src = tuple as *mut crate::object::TupleObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility
        let (start, end) = normalize_slice_indices(start, end, len, step);

        // Collect indices using shared utility
        let indices = collect_step_indices(start, end, step);
        let result_len = indices.len();
        let new_tuple = rt_make_tuple(result_len as i64);
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
#[export_name = "rt_tuple_slice_step"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_slice_step_abi(tuple: Value, start: i64, end: i64, step: i64) -> Value {
    Value::from_ptr(rt_tuple_slice_step(tuple.unwrap_ptr(), start, end, step))
}

/// Create a tuple from a list
/// Returns: pointer to new TupleObj
pub fn rt_tuple_from_list(list: *mut Obj) -> *mut Obj {
    use crate::object::ListObj;

    if list.is_null() {
        return rt_make_tuple(0);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;

        let tuple = rt_make_tuple(len as i64);
        let tuple_obj = tuple as *mut crate::object::TupleObj;

        if len > 0 {
            let dst_data = (*tuple_obj).data.as_mut_ptr();

            // Both list and tuple storage are `[Value]`; direct slot copy.
            let src_data = (*list_obj).data;
            for i in 0..len {
                *dst_data.add(i) = *src_data.add(i);
            }
        }

        tuple
    }
}
#[export_name = "rt_tuple_from_list"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_from_list_abi(list: Value) -> Value {
    Value::from_ptr(rt_tuple_from_list(list.unwrap_ptr()))
}

/// Create a tuple from a string (each character becomes an element)
/// Returns: pointer to new TupleObj
pub fn rt_tuple_from_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::StrObj;
    use crate::string::rt_make_str;

    if str_obj.is_null() {
        return rt_make_tuple(0);
    }

    unsafe {
        let str = str_obj as *mut StrObj;
        let len = (*str).len;
        let data = (*str).data.as_ptr();

        let tuple = rt_make_tuple(len as i64);

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
#[export_name = "rt_tuple_from_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_from_str_abi(str_obj: Value) -> Value {
    Value::from_ptr(rt_tuple_from_str(str_obj.unwrap_ptr()))
}

/// Create a tuple from a range
/// Returns: pointer to new TupleObj
pub fn rt_tuple_from_range(start: i64, stop: i64, step: i64) -> *mut Obj {
    if step == 0 {
        return rt_make_tuple(0);
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

    let tuple = rt_make_tuple(len as i64);

    unsafe {
        let tuple_obj = tuple as *mut crate::object::TupleObj;
        let data = (*tuple_obj).data.as_mut_ptr();

        let mut current = start;
        for i in 0..len {
            *data.add(i) = pyaot_core_defs::Value::from_int(current);
            current += step;
        }
    }

    tuple
}
#[export_name = "rt_tuple_from_range"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_from_range_abi(start: i64, stop: i64, step: i64) -> Value {
    Value::from_ptr(rt_tuple_from_range(start, stop, step))
}

/// Create a tuple by consuming an iterator
/// Returns: pointer to new TupleObj
pub fn rt_tuple_from_iter(iter: *mut Obj) -> *mut Obj {
    use crate::iterator::rt_iter_next_no_exc;
    use crate::list::{rt_list_push, rt_make_list};

    if iter.is_null() {
        return rt_make_tuple(0);
    }

    // First collect into a list (since we don't know the size)
    let list = rt_make_list(8);

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
#[export_name = "rt_tuple_from_iter"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_from_iter_abi(iter: Value) -> Value {
    Value::from_ptr(rt_tuple_from_iter(iter.unwrap_ptr()))
}

/// Create a tuple from a set
/// Returns: pointer to new TupleObj
pub fn rt_tuple_from_set(set: *mut Obj) -> *mut Obj {
    use crate::set::rt_set_to_list;

    // First convert set to list, then list to tuple
    let list = rt_set_to_list(set);
    rt_tuple_from_list(list)
}
#[export_name = "rt_tuple_from_set"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_from_set_abi(set: Value) -> Value {
    Value::from_ptr(rt_tuple_from_set(set.unwrap_ptr()))
}

/// Create a tuple from a dict (keys only)
/// Returns: pointer to new TupleObj
pub fn rt_tuple_from_dict(dict: *mut Obj) -> *mut Obj {
    use crate::dict::rt_dict_keys;

    // First get keys as list, then convert to tuple
    let list = rt_dict_keys(dict);
    rt_tuple_from_list(list)
}
#[export_name = "rt_tuple_from_dict"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_from_dict_abi(dict: Value) -> Value {
    Value::from_ptr(rt_tuple_from_dict(dict.unwrap_ptr()))
}

/// Concatenate two tuples into a new tuple
/// Used for combining extra positional args with list-unpacked varargs
/// Returns: pointer to new TupleObj containing elements from tuple1 followed by tuple2
pub fn rt_tuple_concat(tuple1: *mut Obj, tuple2: *mut Obj) -> *mut Obj {
    use crate::object::TupleObj;

    // Handle null cases
    if tuple1.is_null() && tuple2.is_null() {
        return rt_make_tuple(0);
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

        // Create new tuple
        let result = rt_make_tuple(total_len as i64);
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
#[export_name = "rt_tuple_concat"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_tuple_concat_abi(tuple1: Value, tuple2: Value) -> Value {
    Value::from_ptr(rt_tuple_concat(tuple1.unwrap_ptr(), tuple2.unwrap_ptr()))
}

/// Maximum number of arguments supported for tuple-args indirect calls.
///
/// Dynamic closure/decorator dispatch prepends captured values ahead of the
/// user-visible arguments, so the trampoline must support more than the
/// closure-capture limit alone. Runtime integration currently exercises 9
/// total args (`func + 7 captures + 1 user arg`), so keep headroom here.
const MAX_CALL_ARGS: usize = 16;

/// Extract elements from a tuple into a fixed-size argument array (args path).
/// Unwraps tagged Values to raw scalars: Int→i64, Bool→i64, Ptr→bits as i64.
/// Used when the callee expects raw primitive params (user-visible args).
unsafe fn extract_tuple_unwrapping_values(
    tuple: *mut Obj,
    out_args: &mut [i64; MAX_CALL_ARGS],
    start_idx: usize,
) -> usize {
    use crate::object::TupleObj;
    if tuple.is_null() {
        return 0;
    }
    let tuple_obj = tuple as *mut TupleObj;
    let len = (*tuple_obj).len;
    if len == 0 {
        return 0;
    }
    let data_ptr = (*tuple_obj).data.as_ptr();
    let avail = MAX_CALL_ARGS - start_idx;
    let n = len.min(avail);
    for i in 0..n {
        let val = *data_ptr.add(i);
        out_args[start_idx + i] = if val.is_int() {
            val.unwrap_int()
        } else if val.is_bool() {
            i64::from(val.unwrap_bool())
        } else {
            val.0 as i64
        };
    }
    n
}

/// Extract elements from a tuple into a fixed-size argument array (captures path).
/// Keeps tagged Value bit-patterns intact so the callee's prologue unbox decodes them.
unsafe fn extract_tuple_keeping_values(
    tuple: *mut Obj,
    out_args: &mut [i64; MAX_CALL_ARGS],
    start_idx: usize,
) -> usize {
    use crate::object::TupleObj;
    if tuple.is_null() {
        return 0;
    }
    let tuple_obj = tuple as *mut TupleObj;
    let len = (*tuple_obj).len;
    if len == 0 {
        return 0;
    }
    let data_ptr = (*tuple_obj).data.as_ptr();
    let avail = MAX_CALL_ARGS - start_idx;
    let n = len.min(avail);
    for i in 0..n {
        out_args[start_idx + i] = (*data_ptr.add(i)).0 as i64;
    }
    n
}

/// Dispatch a call to `func_ptr` with `total` arguments from `call_args`.
/// All arguments are passed as i64 (raw ints, raw bools as i64, heap
/// pointers cast to i64). Function pointer uses SystemV calling
/// convention. Returns the function's i64 result.
unsafe fn dispatch_call_with_args(
    func_ptr: i64,
    call_args: &[i64; MAX_CALL_ARGS],
    total: usize,
) -> i64 {
    type F0 = extern "C" fn() -> i64;
    type F1 = extern "C" fn(i64) -> i64;
    type F2 = extern "C" fn(i64, i64) -> i64;
    type F3 = extern "C" fn(i64, i64, i64) -> i64;
    type F4 = extern "C" fn(i64, i64, i64, i64) -> i64;
    type F5 = extern "C" fn(i64, i64, i64, i64, i64) -> i64;
    type F6 = extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64;
    type F7 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F8 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F9 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F10 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F11 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F12 = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F13 =
        extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F14 =
        extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    type F15 = extern "C" fn(
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) -> i64;
    type F16 = extern "C" fn(
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) -> i64;

    match total {
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
        9 => {
            let f: F9 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
            )
        }
        10 => {
            let f: F10 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
                call_args[9],
            )
        }
        11 => {
            let f: F11 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
                call_args[9],
                call_args[10],
            )
        }
        12 => {
            let f: F12 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
                call_args[9],
                call_args[10],
                call_args[11],
            )
        }
        13 => {
            let f: F13 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
                call_args[9],
                call_args[10],
                call_args[11],
                call_args[12],
            )
        }
        14 => {
            let f: F14 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
                call_args[9],
                call_args[10],
                call_args[11],
                call_args[12],
                call_args[13],
            )
        }
        15 => {
            let f: F15 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
                call_args[9],
                call_args[10],
                call_args[11],
                call_args[12],
                call_args[13],
                call_args[14],
            )
        }
        16 => {
            let f: F16 = std::mem::transmute(func_ptr as usize);
            f(
                call_args[0],
                call_args[1],
                call_args[2],
                call_args[3],
                call_args[4],
                call_args[5],
                call_args[6],
                call_args[7],
                call_args[8],
                call_args[9],
                call_args[10],
                call_args[11],
                call_args[12],
                call_args[13],
                call_args[14],
                call_args[15],
            )
        }
        _ => 0, // Unsupported arity
    }
}

/// Call a function pointer with arguments unpacked from a tuple.
/// Used for *args forwarding in decorator wrappers: `func(*args)`.
///
/// All arguments are passed as i64 (raw ints stay as i64, heap objects as
/// pointers cast to i64). The function pointer must use the SystemV
/// calling convention.
pub fn rt_call_with_tuple_args(func_ptr: i64, args_tuple: *mut Obj) -> i64 {
    if func_ptr == 0 {
        return 0;
    }
    unsafe {
        let mut call_args = [0i64; MAX_CALL_ARGS];
        let n = extract_tuple_unwrapping_values(args_tuple, &mut call_args, 0);
        dispatch_call_with_args(func_ptr, &call_args, n)
    }
}
#[export_name = "rt_call_with_tuple_args"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_call_with_tuple_args_abi(func_ptr: i64, args_tuple: Value) -> i64 {
    rt_call_with_tuple_args(func_ptr, args_tuple.unwrap_ptr())
}

/// Stage E (unified closure ABI): closure-trampoline call entry point that
/// extracts captures and user-args from SEPARATE tuples respecting each
/// tuple's own elem_tag. Replaces the prior `rt_tuple_concat` +
/// `rt_call_with_tuple_args` combo, which forced both halves into the
/// first tuple's captures and then delivered user-args as tagged Value bits
/// to a callee that still expected raw primitives in user-visible param slots.
///
/// Capture slots arrive as tagged Values (ValueFromInt/ValueFromBool); the
/// callee's prologue unwraps them. Args slots arrive as raw scalars; the
/// helper unwraps them so user-visible Int/Bool params receive raw values.
pub fn rt_call_with_captures_and_args(
    func_ptr: i64,
    captures_tuple: *mut Obj,
    args_tuple: *mut Obj,
) -> i64 {
    if func_ptr == 0 {
        return 0;
    }
    unsafe {
        let mut call_args = [0i64; MAX_CALL_ARGS];
        let n_caps = extract_tuple_keeping_values(captures_tuple, &mut call_args, 0);
        let n_args = extract_tuple_unwrapping_values(args_tuple, &mut call_args, n_caps);
        dispatch_call_with_args(func_ptr, &call_args, n_caps + n_args)
    }
}
#[export_name = "rt_call_with_captures_and_args"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_call_with_captures_and_args_abi(
    func_ptr: i64,
    captures_tuple: Value,
    args_tuple: Value,
) -> i64 {
    rt_call_with_captures_and_args(
        func_ptr,
        captures_tuple.unwrap_ptr(),
        args_tuple.unwrap_ptr(),
    )
}

#[cfg(test)]
mod tests {
    use super::{rt_call_with_tuple_args, rt_make_tuple, rt_tuple_set};
    use crate::object::Obj;

    extern "C" fn sum9(
        a0: i64,
        a1: i64,
        a2: i64,
        a3: i64,
        a4: i64,
        a5: i64,
        a6: i64,
        a7: i64,
        a8: i64,
    ) -> i64 {
        a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8
    }

    #[test]
    fn call_with_tuple_args_supports_nine_args() {
        let _guard = crate::RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::gc::init();

        // After F.7c values are tagged Value bits — use Value::from_int for int args.
        let tuple = rt_make_tuple(9);
        for i in 0..9i64 {
            let val = pyaot_core_defs::Value::from_int(i + 1);
            rt_tuple_set(tuple, i, val.0 as *mut Obj);
        }

        let result = rt_call_with_tuple_args(sum9 as *const () as usize as i64, tuple);
        assert_eq!(result, 45);
    }
}
