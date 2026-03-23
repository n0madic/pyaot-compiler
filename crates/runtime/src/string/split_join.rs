//! Split and join operations: split, join
//!
//! Uses Boyer-Moore-Horspool for efficient substring search in split.

use crate::gc::{self, gc_pop, gc_push, ShadowFrame};
use crate::list::{rt_list_len, rt_list_push, rt_make_list};
use crate::object::{ListObj, Obj, ObjHeader, StrObj, TypeTagKind, ELEM_HEAP_OBJ};
use crate::string::search::{bmh_find_from, build_bad_char_table, BMH_THRESHOLD};

use super::core::rt_make_str;

/// Split string by separator using Boyer-Moore-Horspool
/// Returns: list of strings
#[no_mangle]
pub extern "C" fn rt_str_split(str_obj: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj {
    if str_obj.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let src_data = (*src).data.as_ptr();

        // Create result list (for string elements)
        let list = rt_make_list(0, ELEM_HEAP_OBJ);
        let max = if maxsplit < 0 { i64::MAX } else { maxsplit };

        // CRITICAL: Protect the list from GC during construction.
        // The list is not on any shadow stack, so GC could collect it
        // when rt_make_str triggers collection.
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Handle None separator (split on whitespace)
        if sep.is_null() {
            // Split on whitespace
            let mut splits: i64 = 0;
            let mut start = 0;
            let mut in_word = false;

            for i in 0..src_len {
                let c = *src_data.add(i);
                let is_ws = c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';

                if is_ws {
                    if in_word {
                        // End of word
                        if splits < max {
                            let word = rt_make_str(src_data.add(start), i - start);
                            rt_list_push(list, word);
                            splits += 1;
                        }
                        in_word = false;
                    }
                } else if !in_word {
                    // Start of word
                    start = i;
                    in_word = true;
                }
            }

            // Handle last word
            if in_word {
                let word = rt_make_str(src_data.add(start), src_len - start);
                rt_list_push(list, word);
            }
        } else {
            let sep_str = sep as *mut StrObj;
            let sep_len = (*sep_str).len;
            let sep_data = (*sep_str).data.as_ptr();

            if sep_len == 0 {
                // Empty separator is invalid, return original string in list
                rt_list_push(list, str_obj);
                gc_pop();
                return list;
            }

            let mut splits: i64 = 0;
            let mut start = 0;

            if sep_len < BMH_THRESHOLD {
                // Naive search for short separators
                let mut i = 0;
                while i + sep_len <= src_len {
                    // Check for separator match
                    let mut matches = true;
                    for j in 0..sep_len {
                        if *src_data.add(i + j) != *sep_data.add(j) {
                            matches = false;
                            break;
                        }
                    }

                    if matches && splits < max {
                        let part = rt_make_str(src_data.add(start), i - start);
                        rt_list_push(list, part);
                        splits += 1;
                        start = i + sep_len;
                        i = start;
                    } else {
                        i += 1;
                    }
                }
            } else {
                // Use BMH for longer separators
                let bad_char = build_bad_char_table(sep_data, sep_len);
                let mut pos = 0;

                loop {
                    if splits >= max {
                        break;
                    }

                    let found = bmh_find_from(src_data, src_len, sep_data, sep_len, pos, &bad_char);
                    if found < 0 {
                        break;
                    }

                    let found_pos = found as usize;
                    let part = rt_make_str(src_data.add(start), found_pos - start);
                    rt_list_push(list, part);
                    splits += 1;
                    start = found_pos + sep_len;
                    pos = start;
                }
            }

            // Add remaining part
            let part = rt_make_str(src_data.add(start), src_len - start);
            rt_list_push(list, part);
        }

        gc_pop();
        list
    }
}

/// Join list of strings with separator
/// Returns: concatenated string
#[no_mangle]
pub extern "C" fn rt_str_join(sep: *mut Obj, list_obj: *mut Obj) -> *mut Obj {
    if list_obj.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let sep_str = if sep.is_null() {
            std::ptr::null()
        } else {
            sep as *mut StrObj
        };
        let sep_len = if sep_str.is_null() { 0 } else { (*sep_str).len };

        let list = list_obj as *mut ListObj;
        let len = rt_list_len(list_obj);

        if len == 0 {
            return rt_make_str(std::ptr::null(), 0);
        }

        // Calculate total length
        let mut total_len = 0;
        for i in 0..len as usize {
            let item = *(*list).data.add(i);
            if !item.is_null() {
                let item_str = item as *mut StrObj;
                total_len += (*item_str).len;
            }
        }
        // Add separators between elements
        if len > 1 {
            total_len += sep_len * ((len - 1) as usize);
        }

        // Root sep and list_obj across gc_alloc: the collector may run during
        // allocation, and neither pointer is on any caller shadow stack.
        // The GC skips null entries in the roots array, so using null for a
        // missing sep is safe.
        let mut roots: [*mut Obj; 2] = [sep, list_obj];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Allocate result (may trigger GC; sep and list_obj stay alive via frame)
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + total_len;
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
        let result = obj as *mut StrObj;
        (*result).len = total_len;

        // Copy strings with separators
        let dst_data = (*result).data.as_mut_ptr();
        let mut dst_idx = 0;

        for i in 0..len as usize {
            if i > 0 && !sep_str.is_null() {
                // Copy separator
                std::ptr::copy_nonoverlapping(
                    (*sep_str).data.as_ptr(),
                    dst_data.add(dst_idx),
                    sep_len,
                );
                dst_idx += sep_len;
            }

            let item = *(*list).data.add(i);
            if !item.is_null() {
                let item_str = item as *mut StrObj;
                let item_len = (*item_str).len;
                std::ptr::copy_nonoverlapping(
                    (*item_str).data.as_ptr(),
                    dst_data.add(dst_idx),
                    item_len,
                );
                dst_idx += item_len;
            }
        }

        gc_pop();
        obj
    }
}

/// str.splitlines() - Split string at line boundaries
/// Returns list of lines in the string, breaking at line boundaries.
/// Line breaks are not included in the resulting list.
/// Recognizes: \n, \r, \r\n
/// Returns: pointer to new ListObj containing StrObj elements
#[no_mangle]
pub extern "C" fn rt_str_splitlines(s: *mut Obj) -> *mut Obj {
    if s.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let str_len = (*str_obj).len;
        let str_data = (*str_obj).data.as_ptr();

        // Create result list
        let list = rt_make_list(0, ELEM_HEAP_OBJ);

        // Protect list from GC during string allocations
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let mut line_start = 0;
        let mut i = 0;

        while i < str_len {
            let c = *str_data.add(i);

            if c == b'\n' {
                // Add line (without newline)
                let line = rt_make_str(str_data.add(line_start), i - line_start);
                rt_list_push(list, line);
                i += 1;
                line_start = i;
            } else if c == b'\r' {
                // Add line (without carriage return)
                let line = rt_make_str(str_data.add(line_start), i - line_start);
                rt_list_push(list, line);
                i += 1;
                // Check for \r\n
                if i < str_len && *str_data.add(i) == b'\n' {
                    i += 1;
                }
                line_start = i;
            } else {
                i += 1;
            }
        }

        // Add remaining line if any
        if line_start < str_len {
            let line = rt_make_str(str_data.add(line_start), str_len - line_start);
            rt_list_push(list, line);
        }

        gc_pop();
        list
    }
}

/// str.partition(sep) - Split at first occurrence of separator
/// Returns (before, sep, after) tuple. If sep not found, returns (str, '', '').
/// Returns: pointer to new 3-tuple
#[no_mangle]
pub extern "C" fn rt_str_partition(s: *mut Obj, sep: *mut Obj) -> *mut Obj {
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if s.is_null() || sep.is_null() {
        // Return (s, '', '')
        unsafe {
            let empty = rt_make_str(std::ptr::null(), 0);
            let tuple = rt_make_tuple(3, ELEM_HEAP_OBJ);
            rt_tuple_set(tuple, 0, s);
            rt_tuple_set(tuple, 1, empty);
            rt_tuple_set(tuple, 2, empty);
            return tuple;
        }
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let sep_obj = sep as *mut StrObj;

        let str_len = (*str_obj).len;
        let sep_len = (*sep_obj).len;
        let str_data = (*str_obj).data.as_ptr();
        let sep_data = (*sep_obj).data.as_ptr();

        // Find first occurrence of separator
        let mut found_pos: Option<usize> = None;

        if sep_len > 0 && sep_len <= str_len {
            for i in 0..=(str_len - sep_len) {
                let mut matches = true;
                for j in 0..sep_len {
                    if *str_data.add(i + j) != *sep_data.add(j) {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    found_pos = Some(i);
                    break;
                }
            }
        }

        let tuple = rt_make_tuple(3, ELEM_HEAP_OBJ);

        if let Some(pos) = found_pos {
            // Found separator: (before, sep, after)
            let before = rt_make_str(str_data, pos);
            let after = rt_make_str(str_data.add(pos + sep_len), str_len - pos - sep_len);

            rt_tuple_set(tuple, 0, before);
            rt_tuple_set(tuple, 1, sep);
            rt_tuple_set(tuple, 2, after);
        } else {
            // Not found: (str, '', '')
            let empty = rt_make_str(std::ptr::null(), 0);
            rt_tuple_set(tuple, 0, s);
            rt_tuple_set(tuple, 1, empty);
            rt_tuple_set(tuple, 2, empty);
        }

        tuple
    }
}

/// str.rpartition(sep) - Split at last occurrence of separator
/// Returns (before, sep, after) tuple. If sep not found, returns ('', '', str).
/// Returns: pointer to new 3-tuple
#[no_mangle]
pub extern "C" fn rt_str_rpartition(s: *mut Obj, sep: *mut Obj) -> *mut Obj {
    use crate::tuple::{rt_make_tuple, rt_tuple_set};

    if s.is_null() || sep.is_null() {
        // Return ('', '', s)
        unsafe {
            let empty = rt_make_str(std::ptr::null(), 0);
            let tuple = rt_make_tuple(3, ELEM_HEAP_OBJ);
            rt_tuple_set(tuple, 0, empty);
            rt_tuple_set(tuple, 1, empty);
            rt_tuple_set(tuple, 2, s);
            return tuple;
        }
    }

    unsafe {
        let str_obj = s as *mut StrObj;
        let sep_obj = sep as *mut StrObj;

        let str_len = (*str_obj).len;
        let sep_len = (*sep_obj).len;
        let str_data = (*str_obj).data.as_ptr();
        let sep_data = (*sep_obj).data.as_ptr();

        // Find last occurrence of separator (search backwards)
        let mut found_pos: Option<usize> = None;

        if sep_len > 0 && sep_len <= str_len {
            for i in (0..=(str_len - sep_len)).rev() {
                let mut matches = true;
                for j in 0..sep_len {
                    if *str_data.add(i + j) != *sep_data.add(j) {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    found_pos = Some(i);
                    break;
                }
            }
        }

        let tuple = rt_make_tuple(3, ELEM_HEAP_OBJ);

        if let Some(pos) = found_pos {
            // Found separator: (before, sep, after)
            let before = rt_make_str(str_data, pos);
            let after = rt_make_str(str_data.add(pos + sep_len), str_len - pos - sep_len);

            rt_tuple_set(tuple, 0, before);
            rt_tuple_set(tuple, 1, sep);
            rt_tuple_set(tuple, 2, after);
        } else {
            // Not found: ('', '', str)
            let empty = rt_make_str(std::ptr::null(), 0);
            rt_tuple_set(tuple, 0, empty);
            rt_tuple_set(tuple, 1, empty);
            rt_tuple_set(tuple, 2, s);
        }

        tuple
    }
}

/// Split string by separator from the right using Boyer-Moore-Horspool
/// Returns: list of strings
#[no_mangle]
pub extern "C" fn rt_str_rsplit(str_obj: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj {
    if str_obj.is_null() {
        return rt_make_list(0, ELEM_HEAP_OBJ);
    }

    unsafe {
        let src = str_obj as *mut StrObj;
        let src_len = (*src).len;
        let src_data = (*src).data.as_ptr();

        // Create result list (for string elements)
        let list = rt_make_list(0, ELEM_HEAP_OBJ);
        let max = if maxsplit < 0 { i64::MAX } else { maxsplit };

        // CRITICAL: Protect the list from GC during construction
        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Handle None separator (split on whitespace from the right)
        if sep.is_null() {
            // Split on whitespace from the right
            let mut splits: i64 = 0;
            let mut end = src_len;
            let mut in_word = false;

            // Scan from right to left
            for i in (0..src_len).rev() {
                let c = *src_data.add(i);
                let is_ws = c == b' ' || c == b'\t' || c == b'\n' || c == b'\r';

                if is_ws {
                    if in_word {
                        // End of word (scanning from right)
                        if splits < max {
                            let word = rt_make_str(src_data.add(i + 1), end - i - 1);
                            rt_list_push(list, word);
                            splits += 1;
                        }
                        in_word = false;
                    }
                } else if !in_word {
                    // Start of word (scanning from right)
                    end = i + 1;
                    in_word = true;
                }
            }

            // Handle first word (leftmost)
            if in_word {
                let word = rt_make_str(src_data, end);
                rt_list_push(list, word);
            }

            // Reverse the list since we built it backwards
            let list_obj = list as *mut ListObj;
            let len = (*list_obj).len;
            for i in 0..(len / 2) {
                let temp = *(*list_obj).data.add(i);
                *(*list_obj).data.add(i) = *(*list_obj).data.add(len - 1 - i);
                *(*list_obj).data.add(len - 1 - i) = temp;
            }
        } else {
            let sep_str = sep as *mut StrObj;
            let sep_len = (*sep_str).len;
            let sep_data = (*sep_str).data.as_ptr();

            if sep_len == 0 {
                // Empty separator is invalid, return original string in list
                rt_list_push(list, str_obj);
                gc_pop();
                return list;
            }

            let mut splits: i64 = 0;
            let mut end = src_len;

            // Search from right to left
            if src_len >= sep_len {
                let mut i = src_len - sep_len;
                loop {
                    // Check for separator match
                    let mut matches = true;
                    for j in 0..sep_len {
                        if *src_data.add(i + j) != *sep_data.add(j) {
                            matches = false;
                            break;
                        }
                    }

                    if matches && splits < max {
                        let part = rt_make_str(src_data.add(i + sep_len), end - i - sep_len);
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

            // Add remaining part (left side)
            let part = rt_make_str(src_data, end);
            rt_list_push(list, part);

            // Reverse the list since we built it backwards
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
