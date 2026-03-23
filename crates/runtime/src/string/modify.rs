//! String modification operations: replace, mul
//!
//! Uses Boyer-Moore-Horspool for efficient substring search in replace.

use crate::gc::{self, gc_pop, gc_push, ShadowFrame};
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};
use crate::string::search::{bmh_find_from, build_bad_char_table, BMH_THRESHOLD};

/// Replace all occurrences of old with new in string using Boyer-Moore-Horspool
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_replace(str_obj: *mut Obj, old: *mut Obj, new: *mut Obj) -> *mut Obj {
    if str_obj.is_null() || old.is_null() || new.is_null() {
        return str_obj; // Return original if any arg is null
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let old_str = old as *mut StrObj;
        let new_str = new as *mut StrObj;

        let src_len = (*src).len;
        let old_len = (*old_str).len;
        let new_len = (*new_str).len;

        if old_len == 0 {
            // CPython behavior: insert `new` before each character and at the end
            // "abc".replace("", "X") -> "XaXbXcX"
            let src_bytes = std::slice::from_raw_parts((*src).data.as_ptr(), (*src).len);
            let new_bytes = std::slice::from_raw_parts((*new_str).data.as_ptr(), (*new_str).len);

            let mut result =
                Vec::with_capacity((*src).len + (new_bytes.len() * (src_bytes.len() + 1)));

            // Iterate over characters (not bytes) for multi-byte safety
            let src_str = std::str::from_utf8_unchecked(src_bytes);
            for ch in src_str.chars() {
                result.extend_from_slice(new_bytes);
                let mut buf = [0u8; 4];
                let encoded = ch.encode_utf8(&mut buf);
                result.extend_from_slice(encoded.as_bytes());
            }
            result.extend_from_slice(new_bytes); // After the last character

            return crate::string::core::rt_make_str_impl(result.as_ptr(), result.len());
        }

        // Read all byte data from the StrObjs before calling gc_alloc.
        // gc_alloc may trigger a collection, which would invalidate raw pointers
        // derived from StrObj fields if those objects are not reachable.  Root
        // the three input objects on the shadow stack for the duration of the
        // allocation so the GC keeps them alive.
        let mut roots: [*mut Obj; 3] = [str_obj, old, new];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Re-derive byte pointers after rooting (pointers remain valid while roots
        // are on the shadow stack because the GC only collects, never moves).
        let src_data = (*src).data.as_ptr();
        let old_data = (*old_str).data.as_ptr();
        let new_data = (*new_str).data.as_ptr();

        // First pass: find all match positions
        let mut positions: Vec<usize> = Vec::new();

        if old_len < BMH_THRESHOLD {
            // Naive search for short patterns
            let mut i = 0;
            while i + old_len <= src_len {
                let mut matches = true;
                for j in 0..old_len {
                    if *src_data.add(i + j) != *old_data.add(j) {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    positions.push(i);
                    i += old_len; // Non-overlapping
                } else {
                    i += 1;
                }
            }
        } else {
            // Use BMH for longer patterns
            let bad_char = build_bad_char_table(old_data, old_len);
            let mut pos = 0;
            loop {
                let found = bmh_find_from(src_data, src_len, old_data, old_len, pos, &bad_char);
                if found < 0 {
                    break;
                }
                positions.push(found as usize);
                pos = (found as usize) + old_len; // Non-overlapping
            }
        }

        if positions.is_empty() {
            gc_pop();
            return str_obj; // No matches, return original
        }

        let count = positions.len();
        let result_len = src_len - (count * old_len) + (count * new_len);

        // Allocate new string (gc_alloc may collect; inputs stay alive via shadow frame)
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let result = obj as *mut StrObj;
        (*result).len = result_len;

        // Second pass: copy with replacements using stored positions
        let dst_data = (*result).data.as_mut_ptr();
        let mut src_idx = 0;
        let mut dst_idx = 0;
        let mut pos_idx = 0;

        while src_idx < src_len {
            // Check if we're at a match position
            if pos_idx < positions.len() && src_idx == positions[pos_idx] {
                // Copy new string instead of old
                std::ptr::copy_nonoverlapping(new_data, dst_data.add(dst_idx), new_len);
                src_idx += old_len;
                dst_idx += new_len;
                pos_idx += 1;
            } else {
                // Copy single character
                *dst_data.add(dst_idx) = *src_data.add(src_idx);
                src_idx += 1;
                dst_idx += 1;
            }
        }

        gc_pop();
        obj
    }
}

/// String multiplication: "abc" * 3 = "abcabcabc"
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_mul(str_obj: *mut Obj, count: i64) -> *mut Obj {
    if str_obj.is_null() || count <= 0 {
        // Return empty string
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>();
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        unsafe {
            let new_str = obj as *mut StrObj;
            (*new_str).len = 0;
        }
        return obj;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let len = (*src).len;
        let count = count as usize;
        let result_len = match len.checked_mul(count) {
            Some(l) => l,
            None => {
                let msg = b"repeated string is too long";
                crate::exceptions::rt_exc_raise_overflow_error(msg.as_ptr(), msg.len());
            }
        };

        // Allocate new string
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let new_str = obj as *mut StrObj;
        (*new_str).len = result_len;

        // Copy the string count times
        let src_data = (*src).data.as_ptr();
        let dst_data = (*new_str).data.as_mut_ptr();
        for i in 0..count {
            std::ptr::copy_nonoverlapping(src_data, dst_data.add(i * len), len);
        }

        obj
    }
}

/// str.removeprefix(prefix) - Return string with prefix removed if present
/// If string starts with prefix, returns string[len(prefix):], otherwise returns original.
/// Returns: pointer to new StrObj
#[no_mangle]
pub extern "C" fn rt_str_removeprefix(s: *mut Obj, prefix: *mut Obj) -> *mut Obj {
    if s.is_null() {
        return s;
    }

    if prefix.is_null() {
        return s;
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let prefix_obj = prefix as *mut StrObj;

        let str_len = (*str_obj).len;
        let prefix_len = (*prefix_obj).len;

        // If prefix is longer than string, can't match
        if prefix_len > str_len {
            return s;
        }

        let str_data = (*str_obj).data.as_ptr();
        let prefix_data = (*prefix_obj).data.as_ptr();

        // Check if string starts with prefix
        let mut matches = true;
        for i in 0..prefix_len {
            if *str_data.add(i) != *prefix_data.add(i) {
                matches = false;
                break;
            }
        }

        if matches {
            // Remove prefix - return substring from prefix_len onwards
            let result_len = str_len - prefix_len;
            let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
            let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

            let result = obj as *mut StrObj;
            (*result).len = result_len;

            if result_len > 0 {
                std::ptr::copy_nonoverlapping(
                    str_data.add(prefix_len),
                    (*result).data.as_mut_ptr(),
                    result_len,
                );
            }

            obj
        } else {
            // No match, return original
            s
        }
    }
}

/// str.removesuffix(suffix) - Return string with suffix removed if present
/// If string ends with suffix, returns string[:-len(suffix)], otherwise returns original.
/// Returns: pointer to new StrObj
#[no_mangle]
pub extern "C" fn rt_str_removesuffix(s: *mut Obj, suffix: *mut Obj) -> *mut Obj {
    if s.is_null() {
        return s;
    }

    if suffix.is_null() {
        return s;
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let suffix_obj = suffix as *mut StrObj;

        let str_len = (*str_obj).len;
        let suffix_len = (*suffix_obj).len;

        // If suffix is longer than string, can't match
        if suffix_len > str_len {
            return s;
        }

        let str_data = (*str_obj).data.as_ptr();
        let suffix_data = (*suffix_obj).data.as_ptr();

        // Check if string ends with suffix
        let start_pos = str_len - suffix_len;
        let mut matches = true;
        for i in 0..suffix_len {
            if *str_data.add(start_pos + i) != *suffix_data.add(i) {
                matches = false;
                break;
            }
        }

        if matches {
            // Remove suffix - return substring up to start_pos
            let result_len = str_len - suffix_len;
            let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
            let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

            let result = obj as *mut StrObj;
            (*result).len = result_len;

            if result_len > 0 {
                std::ptr::copy_nonoverlapping(str_data, (*result).data.as_mut_ptr(), result_len);
            }

            obj
        } else {
            // No match, return original
            s
        }
    }
}

/// str.expandtabs(tabsize) - Replace tabs with spaces
/// Each tab is replaced with spaces to reach the next tab stop (multiples of tabsize).
/// Default tabsize is 8. Negative tabsize is treated as 0.
/// Returns: pointer to new StrObj
#[no_mangle]
pub extern "C" fn rt_str_expandtabs(s: *mut Obj, tabsize: i64) -> *mut Obj {
    if s.is_null() {
        return s;
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let str_len = (*str_obj).len;
        let str_data = (*str_obj).data.as_ptr();

        let tabsize = if tabsize < 0 { 0 } else { tabsize as usize };

        // First pass: calculate result length
        let mut result_len = 0;
        let mut column = 0usize; // Current column position (character count, not byte count)

        for i in 0..str_len {
            let c = *str_data.add(i);
            if c == b'\t' {
                // Tab expands to spaces until next tab stop
                if tabsize > 0 {
                    let spaces = tabsize - (column % tabsize);
                    result_len += spaces;
                    column += spaces;
                }
                // If tabsize is 0, tab is removed (adds 0 spaces)
            } else if c == b'\n' || c == b'\r' {
                // Newlines reset column
                result_len += 1;
                column = 0;
            } else {
                result_len += 1;
                // Only count leading bytes of multi-byte sequences for column tracking
                if (c & 0xC0) != 0x80 {
                    column += 1;
                }
            }
        }

        // Allocate result string
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + result_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let result = obj as *mut StrObj;
        (*result).len = result_len;
        let dst_data = (*result).data.as_mut_ptr();

        // Second pass: copy with tab expansion
        let mut dst_idx = 0;
        let mut column = 0usize;

        for i in 0..str_len {
            let c = *str_data.add(i);
            if c == b'\t' {
                // Expand tab to spaces
                if tabsize > 0 {
                    let spaces = tabsize - (column % tabsize);
                    for _ in 0..spaces {
                        *dst_data.add(dst_idx) = b' ';
                        dst_idx += 1;
                    }
                    column += spaces;
                }
            } else if c == b'\n' || c == b'\r' {
                *dst_data.add(dst_idx) = c;
                dst_idx += 1;
                column = 0;
            } else {
                *dst_data.add(dst_idx) = c;
                dst_idx += 1;
                // Only count leading bytes of multi-byte sequences for column tracking
                if (c & 0xC0) != 0x80 {
                    column += 1;
                }
            }
        }

        obj
    }
}
