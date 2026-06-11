//! String slicing operations: slice, slice_step, getchar

use crate::exceptions;
use crate::gc;
use crate::object::{Obj, StrObj, TypeTagKind};
use crate::slice_utils::{normalize_slice_indices, slice_length};
#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use pyaot_core_defs::Value;

/// Map the character range `[char_start, char_start + char_count)` to a byte
/// range, where codepoint starts are non-continuation bytes
/// (`(b & 0xC0) != 0x80`) — the same rule as `count_codepoints`, so the result
/// is always consistent with the `char_len` cache, even on malformed UTF-8.
/// `char_start + char_count` must be <= the total codepoint count.
unsafe fn char_range_to_byte_range(
    data: *const u8,
    byte_len: usize,
    char_start: usize,
    char_count: usize,
) -> (usize, usize) {
    let char_end = char_start + char_count;
    let mut byte_start = byte_len;
    let mut byte_end = byte_len;
    let mut cp = 0usize;
    for i in 0..byte_len {
        if (*data.add(i)) & 0xC0 != 0x80 {
            if cp == char_start {
                byte_start = i;
            }
            if cp == char_end {
                byte_end = i;
                break;
            }
            cp += 1;
        }
    }
    if char_count == 0 {
        byte_end = byte_start;
    }
    (byte_start, byte_end)
}

/// Slice a string: s[start:end]
/// `start`/`end` are Unicode codepoint indices (CPython semantics); negative
/// indices are supported (counted from the end in characters).
/// Uses i64::MIN as sentinel for "default start" (0) and i64::MAX for "default end" (len)
/// Returns: pointer to new allocated StrObj
pub fn rt_str_slice(str_obj: *mut Obj, start: i64, end: i64) -> *mut Obj {
    use std::ptr;

    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_slice");
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;
        let char_len = (*src).char_len;
        let data = (*src).data.as_ptr();

        // Normalize indices in CHARACTER space (step=1 for simple slice).
        let (start, end) = normalize_slice_indices(start, end, char_len as i64, 1);
        let char_count = slice_length(start, end);

        // Convert the character range to a byte range. Proven ASCII
        // (char_len == byte_len) means char index == byte index; otherwise a
        // single forward walk finds both boundaries (no offsets Vec).
        let (byte_start, byte_end) = if char_len == byte_len {
            (start as usize, start as usize + char_count)
        } else {
            char_range_to_byte_range(data, byte_len, start as usize, char_count)
        };
        let slice_len = byte_end - byte_start;

        // Root str_obj across gc_alloc which may trigger a collection.
        let mut roots: [*mut Obj; 1] = [str_obj];
        let mut frame = gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        // Allocate new string
        let size = crate::string::core::str_alloc_size(slice_len);
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = slice_len;
        (*new_str).char_len = char_count;

        // Copy slice data (re-derive src pointer after gc_alloc for clarity)
        if slice_len > 0 {
            let src = str_obj as *mut StrObj;
            ptr::copy_nonoverlapping(
                (*src).data.as_ptr().add(byte_start),
                (*new_str).data.as_mut_ptr(),
                slice_len,
            );
        }

        gc::gc_pop();
        obj
    }
}
#[export_name = "rt_str_slice"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_slice_abi(str_obj: Value, start: i64, end: i64) -> Value {
    Value::from_ptr(rt_str_slice(str_obj.unwrap_ptr(), start, end))
}

/// Slice a string with step: s[start:end:step]
/// Uses i64::MIN as sentinel for "default start" and i64::MAX for "default end"
/// Defaults depend on step direction:
///   - Positive step: start=0, end=len
///   - Negative step: start=len-1, end=-1 (before index 0)
///
/// Returns: pointer to new allocated StrObj
pub fn rt_str_slice_step(str_obj: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj {
    if step == 0 {
        unsafe {
            raise_exc!(
                exceptions::ExceptionType::ValueError,
                "slice step cannot be zero"
            );
        }
    }
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_slice_step");
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;
        let char_len = (*src).char_len;
        let src_data = (*src).data.as_ptr();

        // Step over CODEPOINTS, not bytes (CPython semantics — covers [::-1]
        // reversal of multi-byte text).
        let (start, end) = normalize_slice_indices(start, end, char_len as i64, step);
        let char_indices = crate::slice_utils::collect_step_indices(start, end, step);
        // Fix the codepoint count before the consuming loop below.
        let char_count = char_indices.len();

        // Pre-copy the selected codepoints' bytes before gc_alloc.
        let mut result_chars = Vec::new();
        if char_len == byte_len {
            // Proven ASCII: char index == byte index, no offsets Vec needed.
            for ci in char_indices {
                result_chars.push(*src_data.add(ci));
            }
        } else {
            // Bidirectional stepping needs random access to codepoint starts;
            // build the offsets Vec (offsets[i] = byte start of char i, with a
            // trailing byte_len entry so offsets[i+1] is the exclusive end).
            // Codepoint starts are non-continuation bytes — the same rule as
            // count_codepoints, so char_len entries are produced.
            let mut offsets = Vec::with_capacity(char_len + 1);
            for i in 0..byte_len {
                if (*src_data.add(i)) & 0xC0 != 0x80 {
                    offsets.push(i);
                }
            }
            offsets.push(byte_len);
            for ci in char_indices {
                let b0 = offsets[ci];
                let b1 = offsets[ci + 1];
                for b in b0..b1 {
                    result_chars.push(*src_data.add(b));
                }
            }
        }

        let result_len = result_chars.len();
        let size = crate::string::core::str_alloc_size(result_len);

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
        (*new_str).char_len = char_count;

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
#[export_name = "rt_str_slice_step"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_slice_step_abi(str_obj: Value, start: i64, end: i64, step: i64) -> Value {
    Value::from_ptr(rt_str_slice_step(str_obj.unwrap_ptr(), start, end, step))
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

/// Get single character at a byte index (for string iteration).
/// The `byte_index` must point to the start of a UTF-8 codepoint.
/// Returns: pointer to new allocated StrObj containing one full codepoint.
pub fn rt_str_getchar(str_obj: *mut Obj, byte_index: i64) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_getchar");
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len as i64;

        if byte_index < 0 || byte_index >= byte_len {
            // Index out of bounds - raise IndexError
            raise_exc!(
                exceptions::ExceptionType::IndexError,
                "string index out of range"
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
        let size = crate::string::core::str_alloc_size(copy_len);
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
        // One codepoint for well-formed UTF-8; recount for the malformed-clamp
        // case so the cache invariant holds unconditionally.
        (*new_str).char_len =
            crate::string::core::count_codepoints((*new_str).data.as_ptr(), copy_len);

        gc::gc_pop();
        obj
    }
}
#[export_name = "rt_str_getchar"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_getchar_abi(str_obj: Value, byte_index: i64) -> Value {
    Value::from_ptr(rt_str_getchar(str_obj.unwrap_ptr(), byte_index))
}

/// Python-level string subscript `s[char_index]`.
/// `char_index` is a Unicode codepoint index (may be negative).
/// Raises IndexError if out of range.
/// Returns: pointer to new allocated StrObj containing one full codepoint.
pub fn rt_str_subscript(str_obj: *mut Obj, char_index: i64) -> *mut Obj {
    if str_obj.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_subscript");
        let src = str_obj as *mut StrObj;
        let byte_len = (*src).len;
        let char_len = (*src).char_len;
        let data = (*src).data.as_ptr();

        let normalized = if char_index < 0 {
            char_len as i64 + char_index
        } else {
            char_index
        };
        if normalized < 0 || normalized >= char_len as i64 {
            raise_exc!(
                exceptions::ExceptionType::IndexError,
                "string index out of range"
            );
        }

        // Proven ASCII: char index == byte index, O(1). Otherwise one forward
        // walk to the n-th codepoint start (non-continuation byte — same rule
        // as count_codepoints).
        let byte_off = if char_len == byte_len {
            normalized as usize
        } else {
            let mut off = byte_len;
            let mut cp = 0i64;
            for i in 0..byte_len {
                if (*data.add(i)) & 0xC0 != 0x80 {
                    if cp == normalized {
                        off = i;
                        break;
                    }
                    cp += 1;
                }
            }
            off
        };
        rt_str_getchar(str_obj, byte_off as i64)
    }
}
#[export_name = "rt_str_subscript"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_subscript_abi(str_obj: Value, char_index: i64) -> Value {
    Value::from_ptr(rt_str_subscript(str_obj.unwrap_ptr(), char_index))
}
