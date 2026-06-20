//! Bytes transformation operations: upper, lower, strip, replace

use crate::gc;
use crate::object::Obj;
use pyaot_core_defs::Value;

use super::core::{rt_make_bytes, rt_make_bytes_zero};

/// Allocate a new bytes object from a slice `[ptr, ptr+len)` of `src`'s buffer,
/// keeping `src` rooted across the allocation. `rt_make_bytes` calls `gc_alloc`,
/// which may collect; the GC is non-moving, so the borrowed slice pointer stays
/// valid as long as `src` remains reachable on the shadow stack.
///
/// # Safety
/// `ptr` must point into `src`'s live data buffer for `len` bytes.
unsafe fn make_bytes_from_rooted(src: *mut Obj, ptr: *const u8, len: usize) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    let mut root: *mut Obj = src;
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: &mut root as *mut *mut Obj,
    };
    gc_push(&mut frame);
    let result = rt_make_bytes(ptr, len);
    gc_pop();
    result
}

/// Replace up to `count` occurrences of old with new in bytes (`count < 0` ⇒
/// replace all, §9).
/// Returns: pointer to new BytesObj
pub fn rt_bytes_replace(bytes: *mut Obj, old: *mut Obj, new: *mut Obj, count: i64) -> *mut Obj {
    use crate::object::{BytesObj, ObjHeader, TypeTagKind};

    if bytes.is_null() {
        return rt_make_bytes_zero(0);
    }
    let limit = if count < 0 {
        usize::MAX
    } else {
        count as usize
    };
    if limit == 0 {
        return bytes; // count == 0 ⇒ no replacements
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

        // Count occurrences (capped at `limit`).
        let mut n_matches = 0;
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
                n_matches += 1;
                if n_matches >= limit {
                    break;
                }
                i += old_len;
            } else {
                i += 1;
            }
        }

        if n_matches == 0 {
            return bytes;
        }

        // Calculate result length
        let result_len = bytes_len + n_matches * new_len - n_matches * old_len;
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Bytes as u8);
        let result_obj = obj as *mut BytesObj;
        (*result_obj).len = result_len;

        // Build result, replacing only the first `n_matches` occurrences then
        // copying the tail verbatim.
        let result_data = (*result_obj).data.as_mut_ptr();
        let mut src_i = 0;
        let mut dst_i = 0;
        let mut replaced = 0;

        while src_i < bytes_len {
            // Check for match (only while under the cap).
            if replaced < n_matches && src_i + old_len <= bytes_len {
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
                    replaced += 1;
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
#[export_name = "rt_bytes_replace"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_replace_abi(bytes: Value, old: Value, new: Value, count: i64) -> Value {
    Value::from_ptr(rt_bytes_replace(
        bytes.unwrap_ptr(),
        old.unwrap_ptr(),
        new.unwrap_ptr(),
        count,
    ))
}

/// Strip whitespace from both ends of bytes
/// Returns: pointer to new BytesObj
pub fn rt_bytes_strip(bytes: *mut Obj) -> *mut Obj {
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

        make_bytes_from_rooted(bytes, data.add(start), end - start)
    }
}
#[export_name = "rt_bytes_strip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_strip_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_bytes_strip(bytes.unwrap_ptr()))
}

/// Strip whitespace from left end of bytes
/// Returns: pointer to new BytesObj
pub fn rt_bytes_lstrip(bytes: *mut Obj) -> *mut Obj {
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

        make_bytes_from_rooted(bytes, data.add(start), len - start)
    }
}
#[export_name = "rt_bytes_lstrip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_lstrip_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_bytes_lstrip(bytes.unwrap_ptr()))
}

/// Strip whitespace from right end of bytes
/// Returns: pointer to new BytesObj
pub fn rt_bytes_rstrip(bytes: *mut Obj) -> *mut Obj {
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

        make_bytes_from_rooted(bytes, data, end)
    }
}
#[export_name = "rt_bytes_rstrip"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_rstrip_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_bytes_rstrip(bytes.unwrap_ptr()))
}

/// Convert bytes to uppercase
/// Returns: pointer to new BytesObj
pub fn rt_bytes_upper(bytes: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_bytes_upper"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_upper_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_bytes_upper(bytes.unwrap_ptr()))
}

/// Convert bytes to lowercase
/// Returns: pointer to new BytesObj
pub fn rt_bytes_lower(bytes: *mut Obj) -> *mut Obj {
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
#[export_name = "rt_bytes_lower"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_lower_abi(bytes: Value) -> Value {
    Value::from_ptr(rt_bytes_lower(bytes.unwrap_ptr()))
}
