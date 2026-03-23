//! Core string operations: creation, data access, length, concatenation

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::exceptions::{rt_exc_raise, ExceptionType};
use crate::gc;
use crate::object::{Obj, ObjHeader, StrObj, TypeTagKind};

/// Create a new string object on the heap (internal implementation)
/// This is the low-level implementation that always allocates.
/// Use rt_make_str() for the public API that uses interning for single chars.
///
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
pub unsafe fn rt_make_str_impl(data: *const u8, len: usize) -> *mut Obj {
    use std::ptr;

    // Calculate size: header + len field + data bytes
    // Use checked arithmetic to prevent overflow
    let size = std::mem::size_of::<ObjHeader>()
        .checked_add(std::mem::size_of::<usize>())
        .and_then(|s| s.checked_add(len))
        .unwrap_or_else(|| {
            let msg = b"MemoryError: string size overflow";
            rt_exc_raise(ExceptionType::MemoryError as u8, msg.as_ptr(), msg.len());
        });

    // Allocate using GC
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

    let str_obj = obj as *mut StrObj;
    (*str_obj).len = len;

    // Copy string data
    if len > 0 && !data.is_null() {
        ptr::copy_nonoverlapping(data, (*str_obj).data.as_mut_ptr(), len);
    }

    obj
}

/// Create a new string object on the heap
/// data: pointer to string bytes (not null-terminated)
/// len: length of the string in bytes
/// Returns: pointer to allocated StrObj
///
/// For single-byte strings, this will use the interned string pool
/// which is pre-populated with all 256 single-byte strings.
///
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rt_make_str(data: *const u8, len: usize) -> *mut Obj {
    // For single-byte strings, use the interned pool (pre-populated in init_string_pool)
    if len == 1 {
        use crate::string::rt_make_str_interned;
        return rt_make_str_interned(data, len);
    }

    rt_make_str_impl(data, len)
}

/// Get the data pointer from a StrObj
/// Returns pointer to the string's byte data
#[no_mangle]
pub extern "C" fn rt_str_data(str_obj: *mut Obj) -> *const u8 {
    if str_obj.is_null() {
        return std::ptr::null();
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_data");
        let str_obj = str_obj as *mut StrObj;
        (*str_obj).data.as_ptr()
    }
}

/// Get the length of a StrObj
#[no_mangle]
pub extern "C" fn rt_str_len(str_obj: *mut Obj) -> usize {
    if str_obj.is_null() {
        return 0;
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_len");
        let str_obj = str_obj as *mut StrObj;
        (*str_obj).len
    }
}

/// Get the length of a string (as i64 for Python's len())
#[no_mangle]
pub extern "C" fn rt_str_len_int(str_obj: *mut Obj) -> i64 {
    if str_obj.is_null() {
        return 0;
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_len_int");
        let str_obj = str_obj as *mut StrObj;
        (*str_obj).len as i64
    }
}

/// Concatenate two strings
/// Returns: pointer to new allocated StrObj
#[no_mangle]
pub extern "C" fn rt_str_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    use std::ptr;

    if a.is_null() || b.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Str, "rt_str_concat");
        debug_assert_type_tag!(b, TypeTagKind::Str, "rt_str_concat");
        let str_a = a as *mut StrObj;
        let str_b = b as *mut StrObj;

        let len_a = (*str_a).len;
        let len_b = (*str_b).len;
        let total_len = match len_a.checked_add(len_b) {
            Some(l) => l,
            None => {
                let msg = b"string concatenation result is too long";
                rt_exc_raise(ExceptionType::OverflowError as u8, msg.as_ptr(), msg.len());
            }
        };

        // Calculate size: header + len field + data bytes
        let size = std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + total_len;

        // Allocate using GC
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        let str_obj = obj as *mut StrObj;
        (*str_obj).len = total_len;

        // Copy data from both strings
        if len_a > 0 {
            ptr::copy_nonoverlapping((*str_a).data.as_ptr(), (*str_obj).data.as_mut_ptr(), len_a);
        }
        if len_b > 0 {
            ptr::copy_nonoverlapping(
                (*str_b).data.as_ptr(),
                (*str_obj).data.as_mut_ptr().add(len_a),
                len_b,
            );
        }

        obj
    }
}

/// Encode string to bytes
/// encoding: pointer to encoding string (utf-8 default if null)
/// Returns: pointer to allocated BytesObj
#[no_mangle]
pub extern "C" fn rt_str_encode(s: *mut Obj, _encoding: *mut Obj) -> *mut Obj {
    use crate::bytes::rt_make_bytes;

    if s.is_null() {
        return unsafe { rt_make_bytes(std::ptr::null(), 0) };
    }

    unsafe {
        debug_assert_type_tag!(s, TypeTagKind::Str, "rt_str_encode");
        let str_obj = s as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();

        // For now, only support UTF-8 encoding (which is identity for our internal representation)
        // If encoding is provided and not "utf-8", we could raise an error, but for simplicity
        // we'll just always use UTF-8
        rt_make_bytes(data, len)
    }
}
