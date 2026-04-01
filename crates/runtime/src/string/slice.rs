//! String slicing operations: slice, slice_step, getchar

use crate::exceptions;
use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};
use crate::slice_utils::{normalize_slice_indices, slice_length};

/// Slice a string: s[start:end]
/// Negative indices are supported (counted from end)
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_slice(str_obj: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use std::ptr;

    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility (step=1 for simple slice)
        let (start, end) = normalize_slice_indices(start, end, len, 1);
        let slice_len = slice_length(start, end);

        // Root str_obj across gc_alloc which may trigger a collection.
        let mut roots: [*mut Obj; 1] = [str_obj];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        // Allocate new string
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + slice_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = slice_len;

        // Copy slice data (re-derive src pointer after gc_alloc for clarity)
        if slice_len > 0 {
            let src = str_obj as *mut StrObj;
            ptr::copy_nonoverlapping(
                (*src).data.as_ptr().add(start as usize),
                (*new_str).data.as_mut_ptr(),
                slice_len,
            );
        }

        gc::gc_pop();
        obj
    }
}

/// Slice a string with step: s[start:end:step]
/// Uses i64::MIN as sentinel for "default start" and i64::MAX for "default end"
/// Defaults depend on step direction:
///   - Positive step: start=0, end=len
///   - Negative step: start=len-1, end=-1 (before index 0)
///
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_slice_step(
    str_obj: *mut Obj,
    start: i64,
    end: i64,
    step: i64,
) -> *mut Obj {
    if str_obj.is_null() || step == 0 {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len as i64;

        // Normalize indices using shared utility
        let (start, end) = normalize_slice_indices(start, end, len, step);

        // Collect characters at step indices (pre-copy data before gc_alloc)
        let mut result_chars = Vec::new();
        let src_data = (*src).data.as_ptr();

        if step > 0 {
            let mut i = start;
            while i < end {
                result_chars.push(*src_data.add(i as usize));
                i += step;
            }
        } else {
            let mut i = start;
            while i > end {
                result_chars.push(*src_data.add(i as usize));
                i += step;
            }
        }

        let result_len = result_chars.len();
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;

        // Root str_obj across gc_alloc for consistency (data already copied to result_chars)
        let mut roots: [*mut Obj; 1] = [str_obj];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = result_len;

        // Copy result data (from stack Vec, safe regardless of GC)
        if result_len > 0 {
            std::ptr::copy_nonoverlapping(
                result_chars.as_ptr(),
                (*new_str).data.as_mut_ptr(),
                result_len,
            );
        }

        gc::gc_pop();
        obj
    }
}

/// Get the UTF-8 byte width of a codepoint starting at `first_byte`.
/// Returns 1, 2, 3, or 4.
#[inline]
pub(crate) fn utf8_char_width(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte < 0xE0 {
        2
    } else if first_byte < 0xF0 {
        3
    } else {
        4
    }
}

/// Walk the UTF-8 bytes and return the byte offset of the n-th codepoint.
/// Negative `char_index` is interpreted as counting from the end.
/// Returns `None` if the index is out of range.
pub(crate) unsafe fn char_index_to_byte_offset(
    data: *const u8,
    byte_len: usize,
    char_index: i64,
) -> Option<usize> {
    // Count total codepoints so we can handle negative indices.
    let total_chars: i64 = {
        let mut n: i64 = 0;
        let mut i = 0usize;
        while i < byte_len {
            let w = utf8_char_width(*data.add(i));
            n += 1;
            i += w;
        }
        n
    };

    let normalized = if char_index < 0 {
        total_chars + char_index
    } else {
        char_index
    };

    if normalized < 0 || normalized >= total_chars {
        return None;
    }

    // Walk again to find the byte offset of the normalized codepoint.
    let mut byte_off = 0usize;
    let mut cp = 0i64;
    while byte_off < byte_len {
        if cp == normalized {
            return Some(byte_off);
        }
        let w = utf8_char_width(*data.add(byte_off));
        byte_off += w;
        cp += 1;
    }
    None
}

/// Get single character at a byte index (for string iteration).
/// The `byte_index` must point to the start of a UTF-8 codepoint.
/// Returns: pointer to new allocated StrObj containing one full codepoint.
#[no_mangle]
pub extern "C" fn rt_str_getchar(str_obj: *mut Obj, byte_index: i64) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len as i64;

        if byte_index < 0 || byte_index >= byte_len {
            // Index out of bounds - raise IndexError
            let msg = b"string index out of range";
            exceptions::rt_exc_raise(
                exceptions::ExceptionType::IndexError as u8,
                msg.as_ptr(),
                msg.len(),
            );
        }

        let data = (*src).data.as_ptr();
        let first_byte = *data.add(byte_index as usize);
        let char_width = utf8_char_width(first_byte);

        // Clamp to available bytes (guard against malformed UTF-8)
        let remaining = (byte_len - byte_index) as usize;
        let copy_len = char_width.min(remaining);

        // Root str_obj across gc_alloc which may trigger a collection.
        let mut roots: [*mut Obj; 1] = [str_obj];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        // Allocate string for one full codepoint
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + copy_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = copy_len;
        // Re-derive data pointer after gc_alloc (GC is non-moving, str_obj is rooted)
        let data = (*(str_obj as *mut StrObj)).data.as_ptr();
        std::ptr::copy_nonoverlapping(
            data.add(byte_index as usize),
            (*new_str).data.as_mut_ptr(),
            copy_len,
        );

        gc::gc_pop();
        obj
    }
}

/// Python-level string subscript `s[char_index]`.
/// `char_index` is a Unicode codepoint index (may be negative).
/// Raises IndexError if out of range.
/// Returns: pointer to new allocated StrObj containing one full codepoint.
#[no_mangle]
pub extern "C" fn rt_str_subscript(str_obj: *mut Obj, char_index: i64) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;
        let data = (*src).data.as_ptr();

        match char_index_to_byte_offset(data, byte_len, char_index) {
            Some(byte_off) => rt_str_getchar(str_obj, byte_off as i64),
            None => {
                let msg = b"string index out of range";
                exceptions::rt_exc_raise(
                    exceptions::ExceptionType::IndexError as u8,
                    msg.as_ptr(),
                    msg.len(),
                );
            }
        }
    }
}
