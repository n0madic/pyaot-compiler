//! Core bytes operations: creation, get, len, eq, slice, concat, repeat

use crate::exceptions;
use crate::exceptions::ExceptionType;
use crate::gc;
use crate::object::Obj;
use crate::slice_utils::{normalize_slice_indices, slice_length};

/// Create a new bytes object on the heap
/// data: pointer to bytes data
/// len: length of the bytes
/// Returns: pointer to allocated BytesObj
///
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_make_bytes(data: *const u8, len: usize) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};
    use std::ptr;

    // Calculate size: header + len field + data bytes
    // Use checked arithmetic to prevent overflow
    let size = std::mem::size_of::<ObjHeader>()
        .checked_add(std::mem::size_of::<usize>())
        .and_then(|s| s.checked_add(len))
        .unwrap_or_else(|| {
            raise_exc!(ExceptionType::MemoryError, "bytes size overflow");
        });

    // Allocate using GC
    let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

    let bytes_obj = obj as *mut BytesObj;
    (*bytes_obj).len = len;

    // Copy bytes data
    if len > 0 && !data.is_null() {
        ptr::copy_nonoverlapping(data, (*bytes_obj).data.as_mut_ptr(), len);
    }

    obj
}

/// Create bytes filled with zeros
/// len: number of zero bytes
/// Returns: pointer to allocated BytesObj
#[no_mangle]
pub extern "C" fn rt_make_bytes_zero(len: i64) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    let len = len.max(0) as usize;

    // Calculate size: header + len field + data bytes
    // Use checked arithmetic to prevent overflow
    let size = std::mem::size_of::<ObjHeader>()
        .checked_add(std::mem::size_of::<usize>())
        .and_then(|s| s.checked_add(len))
        .unwrap_or_else(|| unsafe {
            raise_exc!(ExceptionType::MemoryError, "bytes size overflow");
        });

    // Allocate using GC (gc_alloc zeros the memory)
    let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

    unsafe {
        let bytes_obj = obj as *mut BytesObj;
        (*bytes_obj).len = len;
        // Data is already zeroed by gc_alloc
    }

    obj
}

/// Create bytes from a list of integers
/// list: pointer to ListObj containing integers (0-255)
/// Returns: pointer to allocated BytesObj
#[no_mangle]
pub extern "C" fn rt_make_bytes_from_list(list: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::{BytesObj, ListObj, ObjHeader, TypeTagKind};

    if list.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;

        // Calculate size: header + len field + data bytes (checked for overflow)
        let size = std::mem::size_of::<ObjHeader>()
            .checked_add(std::mem::size_of::<usize>())
            .and_then(|s| s.checked_add(len))
            .unwrap_or_else(|| {
                raise_exc!(ExceptionType::OverflowError, "bytes too large");
            });

        // Root `list` across gc_alloc: a GC collection would free the ListObj
        // and invalidate both the `list_obj` pointer and the element data it
        // points to if the caller has not rooted it.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Allocate using GC
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

        gc_pop();

        let bytes_obj = obj as *mut BytesObj;
        (*bytes_obj).len = len;

        // Copy bytes from list.  Re-derive list_obj through the original
        // pointer (GC is non-moving so the address is stable; re-deriving
        // makes the live range explicit).
        let list_obj = list as *mut ListObj;
        let data = (*list_obj).data;
        let bytes_data = (*bytes_obj).data.as_mut_ptr();
        for i in 0..len {
            let elem = *data.add(i);
            // Elements are raw i64 values cast to pointer
            let value = elem as i64;
            // Clamp to 0-255
            *bytes_data.add(i) = (value & 0xFF) as u8;
        }

        obj
    }
}

/// Create bytes from a string (UTF-8 encoding)
/// str_obj: pointer to StrObj
/// Returns: pointer to allocated BytesObj
#[no_mangle]
pub extern "C" fn rt_make_bytes_from_str(str_obj: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::{BytesObj, ObjHeader, StrObj, TypeTagKind};
    use std::ptr;

    if str_obj.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Calculate size: header + len field + data bytes (checked for overflow)
        let size = std::mem::size_of::<ObjHeader>()
            .checked_add(std::mem::size_of::<usize>())
            .and_then(|s| s.checked_add(len))
            .unwrap_or_else(|| {
                raise_exc!(ExceptionType::OverflowError, "bytes too large");
            });

        // Root `str_obj` across gc_alloc: a GC collection could free the StrObj
        // and invalidate the `src` pointer and the bytes it contains.
        let mut roots: [*mut Obj; 1] = [str_obj];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Allocate using GC
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

        gc_pop();

        let bytes_obj = obj as *mut BytesObj;
        (*bytes_obj).len = len;

        // Copy string data as bytes.  Re-derive src through the original pointer
        // (GC is non-moving; re-deriving makes the live range explicit).
        let src = str_obj as *mut StrObj;
        if len > 0 {
            ptr::copy_nonoverlapping((*src).data.as_ptr(), (*bytes_obj).data.as_mut_ptr(), len);
        }

        obj
    }
}

/// Get byte at index
/// Returns: byte value (0-255) as i64
#[no_mangle]
pub extern "C" fn rt_bytes_get(bytes: *mut Obj, index: i64) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() {
        unsafe {
            raise_exc!(
                exceptions::ExceptionType::IndexError,
                "bytes index out of range"
            );
        }
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        if idx < 0 || idx >= len {
            raise_exc!(
                exceptions::ExceptionType::IndexError,
                "bytes index out of range"
            );
        }

        *(*bytes_obj).data.as_ptr().add(idx as usize) as i64
    }
}

/// Get length of bytes
#[no_mangle]
pub extern "C" fn rt_bytes_len(bytes: *mut Obj) -> i64 {
    if bytes.is_null() {
        return 0;
    }

    unsafe {
        let bytes_obj = bytes as *mut crate::object::BytesObj;
        (*bytes_obj).len as i64
    }
}

/// Compare two bytes objects for equality
/// Returns: 1 if equal, 0 if not equal
#[no_mangle]
pub extern "C" fn rt_bytes_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    use crate::object::BytesObj;

    if a.is_null() || b.is_null() {
        return if a.is_null() && b.is_null() { 1 } else { 0 };
    }

    unsafe {
        let a_obj = a as *mut BytesObj;
        let b_obj = b as *mut BytesObj;

        let a_len = (*a_obj).len;
        let b_len = (*b_obj).len;

        if a_len != b_len {
            return 0;
        }

        // Compare bytes
        let a_data = (*a_obj).data.as_ptr();
        let b_data = (*b_obj).data.as_ptr();
        for i in 0..a_len {
            if *a_data.add(i) != *b_data.add(i) {
                return 0;
            }
        }

        1
    }
}

/// Slice bytes: bytes[start:end]
/// Negative indices are supported (counted from end)
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_slice(bytes: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};
    use std::ptr;

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let src = bytes as *mut BytesObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility (step=1 for simple slice)
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);

        // Allocate new bytes (checked arithmetic to prevent overflow)
        let size = std::mem::size_of::<ObjHeader>()
            .checked_add(std::mem::size_of::<usize>())
            .and_then(|s| s.checked_add(slice_len))
            .unwrap_or_else(|| raise_exc!(ExceptionType::OverflowError, "bytes slice too large"));
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

        let new_bytes = obj as *mut BytesObj;
        (*new_bytes).len = slice_len;

        // Copy slice data
        if slice_len > 0 {
            ptr::copy_nonoverlapping(
                (*src).data.as_ptr().add(start as usize),
                (*new_bytes).data.as_mut_ptr(),
                slice_len,
            );
        }

        obj
    }
}

/// Slice bytes with step: bytes[start:end:step]
/// Uses i64::MIN as sentinel for "default start" and i64::MAX for "default end"
/// Returns: pointer to new allocated BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_slice_step(
    bytes: *mut Obj,
    start: i64,
    end: i64,
    step: i64,
) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if bytes.is_null() || step == 0 {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let src = bytes as *mut BytesObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility
        let (start, end) = normalize_slice_indices(start, end, len, step);

        // Collect bytes at step indices
        let mut result_bytes = Vec::new();
        let src_data = (*src).data.as_ptr();

        if step > 0 {
            let mut i = start;
            while i < end {
                result_bytes.push(*src_data.add(i as usize));
                i += step;
            }
        } else {
            let mut i = start;
            while i > end {
                result_bytes.push(*src_data.add(i as usize));
                i += step; // step is negative
            }
        }

        // Allocate new bytes
        let result_len = result_bytes.len();
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

        let new_bytes = obj as *mut BytesObj;
        (*new_bytes).len = result_len;

        // Copy result bytes
        for (i, &byte) in result_bytes.iter().enumerate() {
            *(*new_bytes).data.as_mut_ptr().add(i) = byte;
        }

        obj
    }
}

/// Concatenate two bytes objects
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if a.is_null() || b.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let a_obj = a as *mut BytesObj;
        let b_obj = b as *mut BytesObj;

        let a_len = (*a_obj).len;
        let b_len = (*b_obj).len;
        let total_len = a_len + b_len;

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + total_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);
        let result = obj as *mut BytesObj;
        (*result).len = total_len;

        let result_data = (*result).data.as_mut_ptr();
        if a_len > 0 {
            std::ptr::copy_nonoverlapping((*a_obj).data.as_ptr(), result_data, a_len);
        }
        if b_len > 0 {
            std::ptr::copy_nonoverlapping((*b_obj).data.as_ptr(), result_data.add(a_len), b_len);
        }

        obj
    }
}

/// Repeat bytes count times
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_repeat(bytes: *mut Obj, count: i64) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if bytes.is_null() || count <= 0 {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        let total_len = len * (count as usize);
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + total_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);
        let result = obj as *mut BytesObj;
        (*result).len = total_len;

        let result_data = (*result).data.as_mut_ptr();
        for i in 0..(count as usize) {
            std::ptr::copy_nonoverlapping(data, result_data.add(i * len), len);
        }

        obj
    }
}
