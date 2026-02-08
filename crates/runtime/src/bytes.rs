//! Bytes operations for Python runtime

use crate::exceptions;
use crate::exceptions::{rt_exc_raise, ExceptionType};
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
            rt_exc_raise(
                ExceptionType::MemoryError as u8,
                b"MemoryError: bytes size overflow".as_ptr(),
                31,
            );
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
            rt_exc_raise(
                ExceptionType::MemoryError as u8,
                b"MemoryError: bytes size overflow".as_ptr(),
                31,
            );
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
    use crate::object::{BytesObj, ListObj, ObjHeader, TypeTagKind};

    if list.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let list_obj = list as *mut ListObj;
        let len = (*list_obj).len;

        // Calculate size: header + len field + data bytes
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;

        // Allocate using GC
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

        let bytes_obj = obj as *mut BytesObj;
        (*bytes_obj).len = len;

        // Copy bytes from list
        // List elements for int type are stored as raw i64 values cast to *mut Obj
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
    use crate::object::{BytesObj, ObjHeader, StrObj, TypeTagKind};
    use std::ptr;

    if str_obj.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;

        // Calculate size: header + len field + data bytes
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;

        // Allocate using GC
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);

        let bytes_obj = obj as *mut BytesObj;
        (*bytes_obj).len = len;

        // Copy string data as bytes
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
        let msg = b"bytes index out of range";
        unsafe {
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::IndexError as u8,
                msg.as_ptr(),
                msg.len(),
            );
        }
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len as i64;

        // Handle negative index
        let idx = if index < 0 { len + index } else { index };

        if idx < 0 || idx >= len {
            let msg = b"bytes index out of range";
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::IndexError as u8,
                msg.as_ptr(),
                msg.len(),
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

        // Allocate new bytes
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + slice_len;
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

/// Decode bytes to string using specified encoding (utf-8 default)
/// encoding: pointer to StrObj for encoding name (null for utf-8)
/// Returns: pointer to allocated StrObj
#[no_mangle]
pub extern "C" fn rt_bytes_decode(bytes: *mut Obj, _encoding: *mut Obj) -> *mut Obj {
    use crate::object::BytesObj;
    use crate::string::rt_make_str;

    if bytes.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        // For now, only support UTF-8 encoding
        // In the future, could check encoding parameter and handle other encodings
        rt_make_str(data, len)
    }
}

/// Check if bytes starts with prefix
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_bytes_startswith(bytes: *mut Obj, prefix: *mut Obj) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || prefix.is_null() {
        return 0;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let prefix_obj = prefix as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let prefix_len = (*prefix_obj).len;

        if prefix_len > bytes_len {
            return 0;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let prefix_data = (*prefix_obj).data.as_ptr();

        for i in 0..prefix_len {
            if *bytes_data.add(i) != *prefix_data.add(i) {
                return 0;
            }
        }

        1
    }
}

/// Check if bytes ends with suffix
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_bytes_endswith(bytes: *mut Obj, suffix: *mut Obj) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || suffix.is_null() {
        return 0;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let suffix_obj = suffix as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let suffix_len = (*suffix_obj).len;

        if suffix_len > bytes_len {
            return 0;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let suffix_data = (*suffix_obj).data.as_ptr();
        let offset = bytes_len - suffix_len;

        for i in 0..suffix_len {
            if *bytes_data.add(offset + i) != *suffix_data.add(i) {
                return 0;
            }
        }

        1
    }
}

/// Find sub-bytes in bytes
/// Returns: index of first occurrence or -1 if not found
#[no_mangle]
pub extern "C" fn rt_bytes_find(bytes: *mut Obj, sub: *mut Obj) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || sub.is_null() {
        return -1;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let sub_obj = sub as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let sub_len = (*sub_obj).len;

        if sub_len == 0 {
            return 0;
        }
        if sub_len > bytes_len {
            return -1;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let sub_data = (*sub_obj).data.as_ptr();

        // Naive search
        for i in 0..=(bytes_len - sub_len) {
            let mut matches = true;
            for j in 0..sub_len {
                if *bytes_data.add(i + j) != *sub_data.add(j) {
                    matches = false;
                    break;
                }
            }
            if matches {
                return i as i64;
            }
        }

        -1
    }
}

/// Find sub-bytes searching from the right
/// Returns: index of last occurrence or -1 if not found
#[no_mangle]
pub extern "C" fn rt_bytes_rfind(bytes: *mut Obj, sub: *mut Obj) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || sub.is_null() {
        return -1;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let sub_obj = sub as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let sub_len = (*sub_obj).len;

        if sub_len == 0 {
            return bytes_len as i64;
        }
        if sub_len > bytes_len {
            return -1;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let sub_data = (*sub_obj).data.as_ptr();

        // Search backwards
        let mut i = bytes_len - sub_len;
        loop {
            let mut matches = true;
            for j in 0..sub_len {
                if *bytes_data.add(i + j) != *sub_data.add(j) {
                    matches = false;
                    break;
                }
            }
            if matches {
                return i as i64;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }

        -1
    }
}

/// Find sub-bytes, raise ValueError if not found
/// Returns: index of first occurrence
#[no_mangle]
pub extern "C" fn rt_bytes_index(bytes: *mut Obj, sub: *mut Obj) -> i64 {
    let result = rt_bytes_find(bytes, sub);
    if result < 0 {
        unsafe {
            let msg = b"subsection not found";
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }
    result
}

/// Find sub-bytes from the right, raise ValueError if not found
/// Returns: index of last occurrence
#[no_mangle]
pub extern "C" fn rt_bytes_rindex(bytes: *mut Obj, sub: *mut Obj) -> i64 {
    let result = rt_bytes_rfind(bytes, sub);
    if result < 0 {
        unsafe {
            let msg = b"subsection not found";
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }
    }
    result
}

/// Count occurrences of sub-bytes
/// Returns: count of non-overlapping occurrences
#[no_mangle]
pub extern "C" fn rt_bytes_count(bytes: *mut Obj, sub: *mut Obj) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || sub.is_null() {
        return 0;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let sub_obj = sub as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let sub_len = (*sub_obj).len;

        if sub_len == 0 {
            return (bytes_len + 1) as i64;
        }
        if sub_len > bytes_len {
            return 0;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let sub_data = (*sub_obj).data.as_ptr();

        let mut count = 0i64;
        let mut i = 0;
        while i + sub_len <= bytes_len {
            let mut matches = true;
            for j in 0..sub_len {
                if *bytes_data.add(i + j) != *sub_data.add(j) {
                    matches = false;
                    break;
                }
            }
            if matches {
                count += 1;
                i += sub_len; // Non-overlapping
            } else {
                i += 1;
            }
        }

        count
    }
}

/// Replace occurrences of old with new in bytes
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_replace(bytes: *mut Obj, old: *mut Obj, new: *mut Obj) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let old_obj = old as *mut BytesObj;
        let new_obj = new as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let old_len = (*old_obj).len;
        let new_len = (*new_obj).len;

        if old_len == 0 {
            // Cannot replace empty bytes
            return bytes;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let old_data = (*old_obj).data.as_ptr();
        let new_data = (*new_obj).data.as_ptr();

        // Count occurrences
        let mut count = 0;
        let mut i = 0;
        while i + old_len <= bytes_len {
            let mut matches = true;
            for j in 0..old_len {
                if *bytes_data.add(i + j) != *old_data.add(j) {
                    matches = false;
                    break;
                }
            }
            if matches {
                count += 1;
                i += old_len;
            } else {
                i += 1;
            }
        }

        if count == 0 {
            return bytes;
        }

        // Calculate result length
        let result_len = bytes_len + count * new_len - count * old_len;
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);
        let result_obj = obj as *mut BytesObj;
        (*result_obj).len = result_len;

        // Build result
        let result_data = (*result_obj).data.as_mut_ptr();
        let mut src_i = 0;
        let mut dst_i = 0;

        while src_i < bytes_len {
            // Check for match
            if src_i + old_len <= bytes_len {
                let mut matches = true;
                for j in 0..old_len {
                    if *bytes_data.add(src_i + j) != *old_data.add(j) {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    // Copy new bytes
                    for j in 0..new_len {
                        *result_data.add(dst_i + j) = *new_data.add(j);
                    }
                    src_i += old_len;
                    dst_i += new_len;
                    continue;
                }
            }
            // Copy original byte
            *result_data.add(dst_i) = *bytes_data.add(src_i);
            src_i += 1;
            dst_i += 1;
        }

        obj
    }
}

// Due to length, I'll split this into a second append for the remaining functions

/// Split bytes by separator
/// Returns: list of BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_split(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::list::{rt_list_push, rt_make_list};
    use crate::object::{BytesObj, ELEM_HEAP_OBJ};

    if bytes.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let bytes_len = (*bytes_obj).len;
        let bytes_data = (*bytes_obj).data.as_ptr();

        let list = rt_make_list(0, ELEM_HEAP_OBJ);
        let max = if maxsplit < 0 { i64::MAX } else { maxsplit };

        // Protect list from GC
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        if sep.is_null() {
            // Split on whitespace
            let mut splits = 0i64;
            let mut start = 0;
            let mut in_segment = false;

            for i in 0..bytes_len {
                let c = *bytes_data.add(i);
                let is_ws = c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';

                if is_ws {
                    if in_segment {
                        if splits < max {
                            let part = rt_make_bytes(bytes_data.add(start), i - start);
                            rt_list_push(list, part);
                            splits += 1;
                        }
                        in_segment = false;
                    }
                } else if !in_segment {
                    start = i;
                    in_segment = true;
                }
            }

            if in_segment {
                let part = rt_make_bytes(bytes_data.add(start), bytes_len - start);
                rt_list_push(list, part);
            }
        } else {
            let sep_obj = sep as *mut BytesObj;
            let sep_len = (*sep_obj).len;
            let sep_data = (*sep_obj).data.as_ptr();

            if sep_len == 0 {
                rt_list_push(list, bytes);
                gc_pop();
                return list;
            }

            let mut splits = 0i64;
            let mut start = 0;
            let mut i = 0;

            while i + sep_len <= bytes_len {
                let mut matches = true;
                for j in 0..sep_len {
                    if *bytes_data.add(i + j) != *sep_data.add(j) {
                        matches = false;
                        break;
                    }
                }

                if matches && splits < max {
                    let part = rt_make_bytes(bytes_data.add(start), i - start);
                    rt_list_push(list, part);
                    splits += 1;
                    start = i + sep_len;
                    i = start;
                } else {
                    i += 1;
                }
            }

            let part = rt_make_bytes(bytes_data.add(start), bytes_len - start);
            rt_list_push(list, part);
        }

        gc_pop();
        list
    }
}

/// Split bytes from the right
/// Returns: list of BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_rsplit(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::list::{rt_list_push, rt_make_list};
    use crate::object::{BytesObj, ListObj, ELEM_HEAP_OBJ};

    if bytes.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let bytes_len = (*bytes_obj).len;
        let bytes_data = (*bytes_obj).data.as_ptr();

        let list = rt_make_list(0, ELEM_HEAP_OBJ);
        let max = if maxsplit < 0 { i64::MAX } else { maxsplit };

        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        if sep.is_null() {
            // Split on whitespace from the right
            let mut splits = 0i64;
            let mut end = bytes_len;
            let mut in_segment = false;

            for i in (0..bytes_len).rev() {
                let c = *bytes_data.add(i);
                let is_ws = c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';

                if is_ws {
                    if in_segment {
                        if splits < max {
                            let part = rt_make_bytes(bytes_data.add(i + 1), end - i - 1);
                            rt_list_push(list, part);
                            splits += 1;
                        }
                        in_segment = false;
                    }
                } else if !in_segment {
                    end = i + 1;
                    in_segment = true;
                }
            }

            if in_segment {
                let part = rt_make_bytes(bytes_data, end);
                rt_list_push(list, part);
            }

            // Reverse the list
            let list_obj = list as *mut ListObj;
            let len = (*list_obj).len;
            for i in 0..(len / 2) {
                let temp = *(*list_obj).data.add(i);
                *(*list_obj).data.add(i) = *(*list_obj).data.add(len - 1 - i);
                *(*list_obj).data.add(len - 1 - i) = temp;
            }
        } else {
            let sep_obj = sep as *mut BytesObj;
            let sep_len = (*sep_obj).len;
            let sep_data = (*sep_obj).data.as_ptr();

            if sep_len == 0 {
                rt_list_push(list, bytes);
                gc_pop();
                return list;
            }

            let mut splits = 0i64;
            let mut end = bytes_len;

            if bytes_len >= sep_len {
                let mut i = bytes_len - sep_len;
                loop {
                    let mut matches = true;
                    for j in 0..sep_len {
                        if *bytes_data.add(i + j) != *sep_data.add(j) {
                            matches = false;
                            break;
                        }
                    }

                    if matches && splits < max {
                        let part = rt_make_bytes(bytes_data.add(i + sep_len), end - i - sep_len);
                        rt_list_push(list, part);
                        splits += 1;
                        end = i;
                        if i == 0 {
                            break;
                        }
                        i = i.saturating_sub(1);
                    } else if i == 0 {
                        break;
                    } else {
                        i -= 1;
                    }
                }
            }

            let part = rt_make_bytes(bytes_data, end);
            rt_list_push(list, part);

            // Reverse the list
            let list_obj = list as *mut ListObj;
            let len = (*list_obj).len;
            for i in 0..(len / 2) {
                let temp = *(*list_obj).data.add(i);
                *(*list_obj).data.add(i) = *(*list_obj).data.add(len - 1 - i);
                *(*list_obj).data.add(len - 1 - i) = temp;
            }
        }

        gc_pop();
        list
    }
}

/// Join bytes with separator
/// sep: separator bytes
/// iterable: list of bytes objects
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_join(sep: *mut Obj, iterable: *mut Obj) -> *mut Obj {
    use crate::list::rt_list_len;
    use crate::object::{BytesObj, ListObj, ObjHeader, TypeTagKind};

    if iterable.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let sep_obj = if sep.is_null() {
            std::ptr::null()
        } else {
            sep as *mut BytesObj
        };
        let sep_len = if sep_obj.is_null() { 0 } else { (*sep_obj).len };

        let list = iterable as *mut ListObj;
        let len = rt_list_len(iterable);

        if len == 0 {
            return rt_make_bytes_zero(0);
        }

        // Calculate total length
        let mut total_len = 0;
        for i in 0..len as usize {
            let item = *(*list).data.add(i);
            if !item.is_null() {
                let item_bytes = item as *mut BytesObj;
                total_len += (*item_bytes).len;
            }
        }
        if len > 1 {
            total_len += sep_len * ((len - 1) as usize);
        }

        // Allocate result
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + total_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);
        let result = obj as *mut BytesObj;
        (*result).len = total_len;

        // Copy bytes with separators
        let dst_data = (*result).data.as_mut_ptr();
        let mut dst_idx = 0;

        for i in 0..len as usize {
            if i > 0 && !sep_obj.is_null() {
                std::ptr::copy_nonoverlapping(
                    (*sep_obj).data.as_ptr(),
                    dst_data.add(dst_idx),
                    sep_len,
                );
                dst_idx += sep_len;
            }

            let item = *(*list).data.add(i);
            if !item.is_null() {
                let item_bytes = item as *mut BytesObj;
                let item_len = (*item_bytes).len;
                std::ptr::copy_nonoverlapping(
                    (*item_bytes).data.as_ptr(),
                    dst_data.add(dst_idx),
                    item_len,
                );
                dst_idx += item_len;
            }
        }

        obj
    }
}

/// Strip whitespace from both ends of bytes
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_strip(bytes: *mut Obj) -> *mut Obj {
    use crate::object::BytesObj;

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        let mut start = 0;
        while start < len {
            let c = *data.add(start);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                break;
            }
            start += 1;
        }

        let mut end = len;
        while end > start {
            let c = *data.add(end - 1);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                break;
            }
            end -= 1;
        }

        rt_make_bytes(data.add(start), end - start)
    }
}

/// Strip whitespace from left end of bytes
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_lstrip(bytes: *mut Obj) -> *mut Obj {
    use crate::object::BytesObj;

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        let mut start = 0;
        while start < len {
            let c = *data.add(start);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                break;
            }
            start += 1;
        }

        rt_make_bytes(data.add(start), len - start)
    }
}

/// Strip whitespace from right end of bytes
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_rstrip(bytes: *mut Obj) -> *mut Obj {
    use crate::object::BytesObj;

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        let mut end = len;
        while end > 0 {
            let c = *data.add(end - 1);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                break;
            }
            end -= 1;
        }

        rt_make_bytes(data, end)
    }
}

/// Convert bytes to uppercase
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_upper(bytes: *mut Obj) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);
        let result = obj as *mut BytesObj;
        (*result).len = len;

        let result_data = (*result).data.as_mut_ptr();
        for i in 0..len {
            let c = *data.add(i);
            *result_data.add(i) = c.to_ascii_uppercase();
        }

        obj
    }
}

/// Convert bytes to lowercase
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_lower(bytes: *mut Obj) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);
        let result = obj as *mut BytesObj;
        (*result).len = len;

        let result_data = (*result).data.as_mut_ptr();
        for i in 0..len {
            let c = *data.add(i);
            *result_data.add(i) = c.to_ascii_lowercase();
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

/// Create bytes from hex string
/// hex_str: pointer to StrObj containing hex digits
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_fromhex(hex_str: *mut Obj) -> *mut Obj {
    use crate::object::StrObj;

    if hex_str.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let str_obj = hex_str as *mut StrObj;
        let str_len = (*str_obj).len;
        let str_data = (*str_obj).data.as_ptr();

        // Skip whitespace and count hex digits
        let mut hex_chars = Vec::new();
        for i in 0..str_len {
            let c = *str_data.add(i);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                hex_chars.push(c);
            }
        }

        if hex_chars.len() % 2 != 0 {
            let msg = b"non-hexadecimal number found in fromhex() arg";
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }

        let byte_count = hex_chars.len() / 2;
        let mut bytes_vec = Vec::with_capacity(byte_count);

        for i in 0..byte_count {
            let high = hex_chars[i * 2];
            let low = hex_chars[i * 2 + 1];

            let high_val = match high {
                b'0'..=b'9' => high - b'0',
                b'a'..=b'f' => high - b'a' + 10,
                b'A'..=b'F' => high - b'A' + 10,
                _ => {
                    let msg = b"non-hexadecimal number found in fromhex() arg";
                    exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
                }
            };

            let low_val = match low {
                b'0'..=b'9' => low - b'0',
                b'a'..=b'f' => low - b'a' + 10,
                b'A'..=b'F' => low - b'A' + 10,
                _ => {
                    let msg = b"non-hexadecimal number found in fromhex() arg";
                    exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
                }
            };

            bytes_vec.push((high_val << 4) | low_val);
        }

        rt_make_bytes(bytes_vec.as_ptr(), byte_count)
    }
}

/// Check if sub-bytes is contained in bytes
/// Returns: 1 (true) or 0 (false)
#[no_mangle]
pub extern "C" fn rt_bytes_contains(bytes: *mut Obj, sub: *mut Obj) -> i64 {
    if rt_bytes_find(bytes, sub) >= 0 {
        1
    } else {
        0
    }
}
