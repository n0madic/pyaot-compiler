//! Bytes search operations: find, rfind, index, rindex, count, contains, split, rsplit, join

use crate::gc;
use crate::object::Obj;
use pyaot_core_defs::Value;

use super::core::{rt_make_bytes, rt_make_bytes_zero};

/// Bounded sub-bytes search within the byte range `[start, end)` (§9). Bytes
/// index by byte, so `start`/`end` are byte indices with CPython slice clamping
/// (a negative adds the length; `start` is floored at 0, `end` capped at the
/// length). The returned index is the ABSOLUTE byte offset, or -1 if not found.
/// `backward` selects the last match (rfind) over the first (find).
fn bytes_find_bounded(bytes: *mut Obj, sub: *mut Obj, start: i64, end: i64, backward: bool) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || sub.is_null() {
        return -1;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let sub_obj = sub as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let sub_len = (*sub_obj).len;

        // CPython slice clamping (byte indices).
        let len_i = bytes_len as i64;
        let start_c: i64 = if start < 0 {
            (start + len_i).max(0)
        } else {
            start
        };
        let end_c: i64 = if end < 0 {
            (end + len_i).max(0)
        } else {
            end.min(len_i)
        };

        // Empty needle: a zero-length match exists at `start` (forward) / `end`
        // (backward) when `start <= end <= len`.
        if sub_len == 0 {
            if start_c > end_c || start_c > len_i {
                return -1;
            }
            return if backward { end_c } else { start_c };
        }
        if start_c > end_c {
            return -1;
        }
        let start_b = (start_c.min(len_i)) as usize; // <= len, safe for the guard
        let end_b = end_c as usize; // already in [0, len]
        if start_b + sub_len > end_b {
            return -1;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let sub_data = (*sub_obj).data.as_ptr();
        let matches_at = |i: usize| -> bool {
            for j in 0..sub_len {
                if *bytes_data.add(i + j) != *sub_data.add(j) {
                    return false;
                }
            }
            true
        };
        let last = end_b - sub_len; // >= start_b (guarded above)
        if backward {
            let mut i = last;
            loop {
                if matches_at(i) {
                    return i as i64;
                }
                if i == start_b {
                    break;
                }
                i -= 1;
            }
        } else {
            for i in start_b..=last {
                if matches_at(i) {
                    return i as i64;
                }
            }
        }
        -1
    }
}

/// Find sub-bytes in bytes within `[start, end)`.
/// Returns: index of first occurrence or -1 if not found
pub fn rt_bytes_find(bytes: *mut Obj, sub: *mut Obj, start: i64, end: i64) -> i64 {
    bytes_find_bounded(bytes, sub, start, end, false)
}
#[export_name = "rt_bytes_find"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_find_abi(bytes: Value, sub: Value, start: i64, end: i64) -> i64 {
    rt_bytes_find(bytes.unwrap_ptr(), sub.unwrap_ptr(), start, end)
}

/// Find sub-bytes searching from the right within `[start, end)`.
/// Returns: index of last occurrence or -1 if not found
pub fn rt_bytes_rfind(bytes: *mut Obj, sub: *mut Obj, start: i64, end: i64) -> i64 {
    bytes_find_bounded(bytes, sub, start, end, true)
}
#[export_name = "rt_bytes_rfind"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_rfind_abi(bytes: Value, sub: Value, start: i64, end: i64) -> i64 {
    rt_bytes_rfind(bytes.unwrap_ptr(), sub.unwrap_ptr(), start, end)
}

/// Generic bytes search with operation tag.
/// op_tag: 0=find, 1=rfind, 2=index, 3=rindex
pub fn rt_bytes_search(bytes: *mut Obj, sub: *mut Obj, op_tag: u8) -> i64 {
    let result = match op_tag {
        0 => rt_bytes_find(bytes, sub, 0, i64::MAX),
        1 => rt_bytes_rfind(bytes, sub, 0, i64::MAX),
        2 | 3 => {
            let r = if op_tag == 2 {
                rt_bytes_find(bytes, sub, 0, i64::MAX)
            } else {
                rt_bytes_rfind(bytes, sub, 0, i64::MAX)
            };
            if r < 0 {
                unsafe {
                    raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "subsection not found"
                    );
                }
            }
            r
        }
        _ => unreachable!("invalid search op_tag: {op_tag}"),
    };
    result
}
#[export_name = "rt_bytes_search"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_search_abi(bytes: Value, sub: Value, op_tag: u8) -> i64 {
    rt_bytes_search(bytes.unwrap_ptr(), sub.unwrap_ptr(), op_tag)
}

/// Count occurrences of sub-bytes
/// Returns: count of non-overlapping occurrences
pub fn rt_bytes_count(bytes: *mut Obj, sub: *mut Obj) -> i64 {
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
#[export_name = "rt_bytes_count"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_count_abi(bytes: Value, sub: Value) -> i64 {
    rt_bytes_count(bytes.unwrap_ptr(), sub.unwrap_ptr())
}

/// Check if sub-bytes is contained in bytes
/// Returns: 1 (true) or 0 (false)
pub fn rt_bytes_contains(bytes: *mut Obj, sub: *mut Obj) -> i64 {
    if rt_bytes_find(bytes, sub, 0, i64::MAX) >= 0 {
        1
    } else {
        0
    }
}
#[export_name = "rt_bytes_contains"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_contains_abi(bytes: Value, sub: Value) -> i64 {
    rt_bytes_contains(bytes.unwrap_ptr(), sub.unwrap_ptr())
}

/// Split bytes by separator
/// Returns: list of BytesObj
pub fn rt_bytes_split(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::list::{rt_list_push, rt_make_list};
    use crate::object::BytesObj;

    if bytes.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let bytes_len = (*bytes_obj).len;
        let bytes_data = (*bytes_obj).data.as_ptr();

        let list = rt_make_list(0);
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
            // Split on whitespace — shared field-range helper (handles the
            // maxsplit remainder correctly; see slice_utils).
            let data = std::slice::from_raw_parts(bytes_data, bytes_len);
            for (start, end) in crate::slice_utils::whitespace_field_ranges(data, maxsplit, false) {
                let part = rt_make_bytes(bytes_data.add(start), end - start);
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
#[export_name = "rt_bytes_split"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_split_abi(bytes: Value, sep: Value, maxsplit: i64) -> Value {
    Value::from_ptr(rt_bytes_split(
        bytes.unwrap_ptr(),
        sep.unwrap_ptr(),
        maxsplit,
    ))
}

/// Split bytes from the right
/// Returns: list of BytesObj
pub fn rt_bytes_rsplit(bytes: *mut Obj, sep: *mut Obj, maxsplit: i64) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::list::{rt_list_push, rt_make_list};
    use crate::object::{BytesObj, ListObj};

    if bytes.is_null() {
        return rt_make_list(0);
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let bytes_len = (*bytes_obj).len;
        let bytes_data = (*bytes_obj).data.as_ptr();

        let list = rt_make_list(0);
        let max = if maxsplit < 0 { i64::MAX } else { maxsplit };

        let mut roots: [*mut Obj; 1] = [list];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        if sep.is_null() {
            // Split on whitespace from the right — shared field-range helper
            // returns fields already in left-to-right order (no reversal).
            let data = std::slice::from_raw_parts(bytes_data, bytes_len);
            for (start, end) in crate::slice_utils::whitespace_field_ranges(data, maxsplit, true) {
                let part = rt_make_bytes(bytes_data.add(start), end - start);
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
#[export_name = "rt_bytes_rsplit"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_rsplit_abi(bytes: Value, sep: Value, maxsplit: i64) -> Value {
    Value::from_ptr(rt_bytes_rsplit(
        bytes.unwrap_ptr(),
        sep.unwrap_ptr(),
        maxsplit,
    ))
}

/// Join bytes with separator
/// sep: separator bytes
/// iterable: list of bytes objects
/// Returns: pointer to new BytesObj
pub fn rt_bytes_join(sep: *mut Obj, iterable: *mut Obj) -> *mut Obj {
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
            let item = (*(*list).data.add(i)).0 as *mut crate::object::Obj;
            if !item.is_null() {
                let item_bytes = item as *mut BytesObj;
                total_len += (*item_bytes).len;
            }
        }
        if len > 1 {
            total_len += sep_len * ((len - 1) as usize);
        }

        // Allocate result. gc_alloc may collect; root `sep` and `iterable`
        // (the list — which transitively keeps its item bytes marked) so the
        // separator/source buffers we copy from below survive the collection.
        let mut roots: [*mut Obj; 2] = [sep, iterable];
        let mut frame = crate::gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        crate::gc::gc_push(&mut frame);

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

            let item = (*(*list).data.add(i)).0 as *mut crate::object::Obj;
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

        crate::gc::gc_pop();
        obj
    }
}
#[export_name = "rt_bytes_join"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_join_abi(sep: Value, iterable: Value) -> Value {
    Value::from_ptr(rt_bytes_join(sep.unwrap_ptr(), iterable.unwrap_ptr()))
}
