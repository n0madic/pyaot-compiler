//! Bytes transformation operations: upper, lower, strip, replace

use crate::gc;
use crate::object::Obj;

use super::core::{rt_make_bytes, rt_make_bytes_zero};

/// Replace occurrences of old with new in bytes
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_replace(bytes: *mut Obj, old: *mut Obj, new: *mut Obj) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        if old.is_null() || new.is_null() {
            return bytes;
        }

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
