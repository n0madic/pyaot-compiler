//! Bytes search operations: find, rfind, index, rindex, count, contains, split, rsplit, join

use crate::gc;
use crate::object::Obj;

use super::core::{rt_make_bytes, rt_make_bytes_zero};

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
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
            crate::exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
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
