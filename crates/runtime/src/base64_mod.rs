//! Base64 encoding and decoding for Python runtime
//!
//! This module provides base64 encoding/decoding operations compatible with
//! Python's base64 module, using the `base64` crate for the actual encoding.

use crate::exceptions::{rt_exc_raise, ExceptionType};
use crate::gc;
use crate::object::{BytesObj, Obj, ObjHeader, StrObj, TypeTagKind};
use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use base64::Engine;

/// Helper function to create a bytes object from a Vec<u8>
unsafe fn make_bytes_from_vec(data: Vec<u8>) -> *mut Obj {
    let len = data.len();

    // Calculate size: header + len field + data bytes
    let size = std::mem::size_of::<ObjHeader>()
        .checked_add(std::mem::size_of::<usize>())
        .and_then(|s| s.checked_add(len))
        .unwrap_or_else(|| {
            let msg = b"MemoryError: bytes size overflow";
            rt_exc_raise(ExceptionType::MemoryError as u8, msg.as_ptr(), msg.len());
        });

    // Allocate using GC
    let obj = gc::gc_alloc(size, TypeTagKind::Bytes.tag());
    let bytes_obj = obj as *mut BytesObj;
    (*bytes_obj).len = len;

    // Copy data
    if len > 0 {
        std::ptr::copy_nonoverlapping(data.as_ptr(), (*bytes_obj).data.as_mut_ptr(), len);
    }

    obj
}

/// Helper function to get bytes slice from either StrObj or BytesObj
unsafe fn get_bytes_slice(obj: *mut Obj) -> (&'static [u8], TypeTagKind) {
    if obj.is_null() {
        let msg = b"TypeError: expected str or bytes";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    let header = &(*(obj as *const Obj)).header;
    let type_tag = header.type_tag;

    match type_tag {
        TypeTagKind::Str => {
            let str_obj = obj as *const StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            (std::slice::from_raw_parts(data, len), type_tag)
        }
        TypeTagKind::Bytes => {
            let bytes_obj = obj as *const BytesObj;
            let len = (*bytes_obj).len;
            let data = (*bytes_obj).data.as_ptr();
            (std::slice::from_raw_parts(data, len), type_tag)
        }
        _ => {
            let msg = b"TypeError: expected str or bytes";
            rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
        }
    }
}

/// Encode bytes to standard base64 bytes
/// data: pointer to BytesObj to encode
/// Returns: pointer to BytesObj containing base64-encoded data
///
/// # Safety
/// data must be a valid BytesObj pointer
#[no_mangle]
pub unsafe extern "C" fn rt_base64_b64encode(data: *mut Obj) -> *mut Obj {
    if data.is_null() {
        let msg = b"TypeError: expected bytes";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    let header = &(*(data as *const Obj)).header;
    if header.type_tag != TypeTagKind::Bytes {
        let msg = b"TypeError: expected bytes";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    let bytes_obj = data as *const BytesObj;
    let len = (*bytes_obj).len;
    let input_data = (*bytes_obj).data.as_ptr();
    let input_slice = std::slice::from_raw_parts(input_data, len);

    // Encode to base64 string
    let encoded = STANDARD.encode(input_slice);

    // Convert string to bytes
    make_bytes_from_vec(encoded.into_bytes())
}

/// Decode standard base64 (str or bytes) to bytes
/// data: pointer to StrObj or BytesObj containing base64 data
/// Returns: pointer to BytesObj containing decoded data
///
/// # Safety
/// data must be a valid StrObj or BytesObj pointer with valid base64 content
#[no_mangle]
pub unsafe extern "C" fn rt_base64_b64decode(data: *mut Obj) -> *mut Obj {
    let (input_slice, _type_tag) = get_bytes_slice(data);

    // Decode from base64
    match STANDARD.decode(input_slice) {
        Ok(decoded) => make_bytes_from_vec(decoded),
        Err(_) => {
            let msg = b"ValueError: invalid base64 data";
            rt_exc_raise(ExceptionType::ValueError as u8, msg.as_ptr(), msg.len());
        }
    }
}

/// Encode bytes to URL-safe base64 bytes
/// data: pointer to BytesObj to encode
/// Returns: pointer to BytesObj containing URL-safe base64-encoded data
///
/// # Safety
/// data must be a valid BytesObj pointer
#[no_mangle]
pub unsafe extern "C" fn rt_base64_urlsafe_b64encode(data: *mut Obj) -> *mut Obj {
    if data.is_null() {
        let msg = b"TypeError: expected bytes";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    let header = &(*(data as *const Obj)).header;
    if header.type_tag != TypeTagKind::Bytes {
        let msg = b"TypeError: expected bytes";
        rt_exc_raise(ExceptionType::TypeError as u8, msg.as_ptr(), msg.len());
    }

    let bytes_obj = data as *const BytesObj;
    let len = (*bytes_obj).len;
    let input_data = (*bytes_obj).data.as_ptr();
    let input_slice = std::slice::from_raw_parts(input_data, len);

    // Encode to URL-safe base64 string
    let encoded = URL_SAFE.encode(input_slice);

    // Convert string to bytes
    make_bytes_from_vec(encoded.into_bytes())
}

/// Decode URL-safe base64 (str or bytes) to bytes
/// data: pointer to StrObj or BytesObj containing URL-safe base64 data
/// Returns: pointer to BytesObj containing decoded data
///
/// # Safety
/// data must be a valid StrObj or BytesObj pointer with valid URL-safe base64 content
#[no_mangle]
pub unsafe extern "C" fn rt_base64_urlsafe_b64decode(data: *mut Obj) -> *mut Obj {
    let (input_slice, _type_tag) = get_bytes_slice(data);

    // Decode from URL-safe base64
    match URL_SAFE.decode(input_slice) {
        Ok(decoded) => make_bytes_from_vec(decoded),
        Err(_) => {
            let msg = b"ValueError: invalid base64 data";
            rt_exc_raise(ExceptionType::ValueError as u8, msg.as_ptr(), msg.len());
        }
    }
}
