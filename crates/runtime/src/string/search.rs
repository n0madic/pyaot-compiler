//! Search and comparison operations: startswith, endswith, find, eq, contains, count
//!
//! Uses Boyer-Moore-Horspool algorithm for efficient O(n/m) substring search
//! when pattern length >= BMH_THRESHOLD.

use crate::object::{Obj, StrObj};
use pyaot_core_defs::Value;

/// Convert a byte offset to a character (codepoint) offset in a UTF-8 string
fn byte_offset_to_char_offset(bytes: &[u8], byte_offset: usize) -> usize {
    bytes[..byte_offset]
        .iter()
        .filter(|&&b| (b & 0xC0) != 0x80)
        .count()
}

/// Minimum pattern length to use Boyer-Moore-Horspool.
/// For shorter patterns, naive search has less overhead.
pub(crate) const BMH_THRESHOLD: usize = 4;

/// Build Boyer-Moore-Horspool bad character table.
/// Returns skip distances for each byte value (0-255).
///
/// For characters in the pattern: skip = pattern_len - 1 - rightmost_position
/// For characters not in pattern: skip = pattern_len
#[inline]
pub(crate) unsafe fn build_bad_char_table(pattern: *const u8, pattern_len: usize) -> [usize; 256] {
    let mut table = [pattern_len; 256];

    // For each character in pattern (except the last), compute skip distance
    // We process left-to-right so later positions overwrite earlier ones
    for i in 0..(pattern_len.saturating_sub(1)) {
        let c = *pattern.add(i) as usize;
        table[c] = pattern_len - 1 - i;
    }

    table
}

/// Boyer-Moore-Horspool substring search.
/// Returns the index of the first occurrence, or -1 if not found.
///
/// Time complexity: O(n/m) average case, O(n*m) worst case
/// Space complexity: O(256) = O(1) for the bad character table
#[inline]
pub(crate) unsafe fn bmh_find(
    haystack: *const u8,
    haystack_len: usize,
    needle: *const u8,
    needle_len: usize,
) -> i64 {
    if needle_len == 0 {
        return 0;
    }
    if needle_len > haystack_len {
        return -1;
    }

    // For short patterns, use naive search (less overhead)
    if needle_len < BMH_THRESHOLD {
        return naive_find(haystack, haystack_len, needle, needle_len);
    }

    let bad_char = build_bad_char_table(needle, needle_len);
    let last_idx = needle_len - 1;
    let mut i = 0;

    while i <= haystack_len - needle_len {
        // Compare from right to left
        let mut j = last_idx;
        loop {
            if *haystack.add(i + j) != *needle.add(j) {
                break;
            }
            if j == 0 {
                // Found match
                return i as i64;
            }
            j -= 1;
        }

        // Shift by bad character rule using the last character of the window
        let skip_char = *haystack.add(i + last_idx) as usize;
        i += bad_char[skip_char];
    }

    -1
}

/// Naive substring search for short patterns.
#[inline]
unsafe fn naive_find(
    haystack: *const u8,
    haystack_len: usize,
    needle: *const u8,
    needle_len: usize,
) -> i64 {
    let limit = haystack_len - needle_len + 1;

    'outer: for i in 0..limit {
        for j in 0..needle_len {
            if *haystack.add(i + j) != *needle.add(j) {
                continue 'outer;
            }
        }
        return i as i64;
    }

    -1
}

/// Boyer-Moore-Horspool search starting from a given position.
/// Used for counting non-overlapping occurrences.
#[inline]
pub(crate) unsafe fn bmh_find_from(
    haystack: *const u8,
    haystack_len: usize,
    needle: *const u8,
    needle_len: usize,
    start: usize,
    bad_char: &[usize; 256],
) -> i64 {
    if start + needle_len > haystack_len {
        return -1;
    }

    let last_idx = needle_len - 1;
    let mut i = start;

    while i <= haystack_len - needle_len {
        // Compare from right to left
        let mut j = last_idx;
        loop {
            if *haystack.add(i + j) != *needle.add(j) {
                break;
            }
            if j == 0 {
                // Found match
                return i as i64;
            }
            j -= 1;
        }

        // Shift by bad character rule
        let skip_char = *haystack.add(i + last_idx) as usize;
        i += bad_char[skip_char];
    }

    -1
}

/// Check if string starts with prefix
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_startswith(str_obj: *mut Obj, prefix: *mut Obj) -> i8 {
    if str_obj.is_null() || prefix.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let pre = prefix as *mut StrObj;

        let src_len = (*src).len;
        let pre_len = (*pre).len;

        if pre_len > src_len {
            return 0;
        }

        let src_data = (*src).data.as_ptr();
        let pre_data = (*pre).data.as_ptr();

        for i in 0..pre_len {
            if *src_data.add(i) != *pre_data.add(i) {
                return 0;
            }
        }

        1
    }
}
#[export_name = "rt_str_startswith"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_startswith_abi(str_obj: Value, prefix: Value) -> i8 {
    rt_str_startswith(str_obj.unwrap_ptr(), prefix.unwrap_ptr())
}

/// Check if string ends with suffix
/// Returns: 1 (true) or 0 (false)
pub fn rt_str_endswith(str_obj: *mut Obj, suffix: *mut Obj) -> i8 {
    if str_obj.is_null() || suffix.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let suf = suffix as *mut StrObj;

        let src_len = (*src).len;
        let suf_len = (*suf).len;

        if suf_len > src_len {
            return 0;
        }

        let src_data = (*src).data.as_ptr();
        let suf_data = (*suf).data.as_ptr();
        let offset = src_len - suf_len;

        for i in 0..suf_len {
            if *src_data.add(offset + i) != *suf_data.add(i) {
                return 0;
            }
        }

        1
    }
}
#[export_name = "rt_str_endswith"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_endswith_abi(str_obj: Value, suffix: Value) -> i8 {
    rt_str_endswith(str_obj.unwrap_ptr(), suffix.unwrap_ptr())
}

/// Find substring in string using Boyer-Moore-Horspool algorithm
/// Returns: index of first occurrence or -1 if not found
pub fn rt_str_find(str_obj: *mut Obj, sub: *mut Obj) -> i64 {
    if str_obj.is_null() || sub.is_null() {
        return -1;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let needle = sub as *mut StrObj;

        let src_len = (*src).len;
        let needle_len = (*needle).len;

        if needle_len == 0 {
            return 0;
        }
        if needle_len > src_len {
            return -1;
        }

        let src_data = (*src).data.as_ptr();
        let needle_data = (*needle).data.as_ptr();

        let byte_pos = bmh_find(src_data, src_len, needle_data, needle_len);
        if byte_pos < 0 {
            return -1;
        }
        // Convert byte offset to character offset for CPython compatibility
        let haystack_bytes = std::slice::from_raw_parts(src_data, src_len);
        let char_offset = byte_offset_to_char_offset(haystack_bytes, byte_pos as usize);
        char_offset as i64
    }
}
#[export_name = "rt_str_find"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_find_abi(str_obj: Value, sub: Value) -> i64 {
    rt_str_find(str_obj.unwrap_ptr(), sub.unwrap_ptr())
}

/// Compare two strings for equality
/// Returns: 1 if equal, 0 if not equal
pub fn rt_str_eq(a: *mut Obj, b: *mut Obj) -> i8 {
    if a.is_null() && b.is_null() {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }

    unsafe {
        let str_a = a as *mut StrObj;
        let str_b = b as *mut StrObj;

        let len_a = (*str_a).len;
        let len_b = (*str_b).len;

        if len_a != len_b {
            return 0;
        }

        if len_a == 0 {
            return 1; // Both empty strings
        }

        let data_a = (*str_a).data.as_ptr();
        let data_b = (*str_b).data.as_ptr();

        for i in 0..len_a {
            if *data_a.add(i) != *data_b.add(i) {
                return 0;
            }
        }

        1
    }
}
#[export_name = "rt_str_eq"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_eq_abi(a: Value, b: Value) -> i8 {
    rt_str_eq(a.unwrap_ptr(), b.unwrap_ptr())
}

/// Check if needle is a substring of haystack using Boyer-Moore-Horspool
/// Returns 1 if needle is found in haystack, 0 otherwise
pub fn rt_str_contains(needle: *mut Obj, haystack: *mut Obj) -> i8 {
    if needle.is_null() || haystack.is_null() {
        return 0;
    }

    unsafe {
        let needle_str = needle as *mut StrObj;
        let haystack_str = haystack as *mut StrObj;

        let needle_len = (*needle_str).len;
        let haystack_len = (*haystack_str).len;

        // Empty needle is always found
        if needle_len == 0 {
            return 1;
        }

        // Needle longer than haystack cannot be found
        if needle_len > haystack_len {
            return 0;
        }

        let needle_data = (*needle_str).data.as_ptr();
        let haystack_data = (*haystack_str).data.as_ptr();

        if bmh_find(haystack_data, haystack_len, needle_data, needle_len) >= 0 {
            1
        } else {
            0
        }
    }
}
#[export_name = "rt_str_contains"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_contains_abi(needle: Value, haystack: Value) -> i8 {
    rt_str_contains(needle.unwrap_ptr(), haystack.unwrap_ptr())
}

/// Count occurrences of substring using Boyer-Moore-Horspool
/// Returns: count of non-overlapping occurrences
pub fn rt_str_count(str_obj: *mut Obj, sub: *mut Obj) -> i64 {
    if str_obj.is_null() || sub.is_null() {
        return 0;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let needle = sub as *mut StrObj;

        let src_len = (*src).len;
        let needle_len = (*needle).len;

        if needle_len == 0 {
            // Empty string count = number of characters + 1 (matching CPython)
            let src_bytes = std::slice::from_raw_parts((*src).data.as_ptr(), src_len);
            let char_count = src_bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count();
            return (char_count + 1) as i64;
        }
        if needle_len > src_len {
            return 0;
        }

        let src_data = (*src).data.as_ptr();
        let needle_data = (*needle).data.as_ptr();

        // For short patterns, use naive counting (less overhead)
        if needle_len < BMH_THRESHOLD {
            let mut count: i64 = 0;
            let mut i = 0;
            while i + needle_len <= src_len {
                let mut matches = true;
                for j in 0..needle_len {
                    if *src_data.add(i + j) != *needle_data.add(j) {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    count += 1;
                    i += needle_len; // Non-overlapping
                } else {
                    i += 1;
                }
            }
            return count;
        }

        // Use BMH for longer patterns
        let bad_char = build_bad_char_table(needle_data, needle_len);
        let mut count: i64 = 0;
        let mut pos = 0;

        loop {
            let found = bmh_find_from(src_data, src_len, needle_data, needle_len, pos, &bad_char);
            if found < 0 {
                break;
            }
            count += 1;
            pos = (found as usize) + needle_len; // Non-overlapping
        }

        count
    }
}
#[export_name = "rt_str_count"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_count_abi(str_obj: Value, sub: Value) -> i64 {
    rt_str_count(str_obj.unwrap_ptr(), sub.unwrap_ptr())
}

/// Find substring in string searching from the right using Boyer-Moore-Horspool algorithm
/// Returns: index of last occurrence or -1 if not found
pub fn rt_str_rfind(str_obj: *mut Obj, sub: *mut Obj) -> i64 {
    if str_obj.is_null() || sub.is_null() {
        return -1;
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let needle = sub as *mut StrObj;

        let src_len = (*src).len;
        let needle_len = (*needle).len;

        if needle_len == 0 {
            // Return character count (not byte count) for CPython compatibility
            let src_data = (*src).data.as_ptr();
            let src_bytes = std::slice::from_raw_parts(src_data, src_len);
            let char_count = src_bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count();
            return char_count as i64;
        }
        if needle_len > src_len {
            return -1;
        }

        let src_data = (*src).data.as_ptr();
        let needle_data = (*needle).data.as_ptr();

        // Search backwards - try each position from right to left
        let mut i = src_len - needle_len;
        loop {
            // Compare at position i
            let mut matches = true;
            for j in 0..needle_len {
                if *src_data.add(i + j) != *needle_data.add(j) {
                    matches = false;
                    break;
                }
            }
            if matches {
                // Convert byte offset to character offset for CPython compatibility
                let haystack_bytes = std::slice::from_raw_parts(src_data, src_len);
                let char_offset = byte_offset_to_char_offset(haystack_bytes, i);
                return char_offset as i64;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }

        -1
    }
}
#[export_name = "rt_str_rfind"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_rfind_abi(str_obj: Value, sub: Value) -> i64 {
    rt_str_rfind(str_obj.unwrap_ptr(), sub.unwrap_ptr())
}

/// Generic string search with operation tag.
/// op_tag: 0=find, 1=rfind, 2=index, 3=rindex
pub fn rt_str_search(str_obj: *mut Obj, sub: *mut Obj, op_tag: u8) -> i64 {
    let result = match op_tag {
        0 => rt_str_find(str_obj, sub),
        1 => rt_str_rfind(str_obj, sub),
        2 | 3 => {
            let r = if op_tag == 2 {
                rt_str_find(str_obj, sub)
            } else {
                rt_str_rfind(str_obj, sub)
            };
            if r < 0 {
                unsafe {
                    raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "substring not found"
                    );
                }
            }
            r
        }
        _ => unreachable!("invalid search op_tag: {op_tag}"),
    };
    result
}
#[export_name = "rt_str_search"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_search_abi(str_obj: Value, sub: Value, op_tag: u8) -> i64 {
    rt_str_search(str_obj.unwrap_ptr(), sub.unwrap_ptr(), op_tag)
}
