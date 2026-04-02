//! Bytes conversion operations: decode, fromhex

use crate::exceptions;
use crate::object::Obj;

use super::core::{rt_make_bytes, rt_make_bytes_zero};

/// Decode bytes to string using specified encoding (utf-8 default)
/// encoding: pointer to StrObj for encoding name (null for utf-8)
/// Returns: pointer to allocated StrObj
#[no_mangle]
pub extern "C" fn rt_bytes_decode(bytes: *mut Obj, _encoding: *mut Obj) -> *mut Obj {
    use crate::object::BytesObj;
    use crate::string::rt_make_str;

    if bytes.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();

        // For now, only support UTF-8 encoding
        // In the future, could check encoding parameter and handle other encodings
        rt_make_str(data, len)
    }
}

/// Create bytes from hex string
/// hex_str: pointer to StrObj containing hex digits
/// Returns: pointer to new BytesObj
#[no_mangle]
pub extern "C" fn rt_bytes_fromhex(hex_str: *mut Obj) -> *mut Obj {
    use crate::object::StrObj;

    if hex_str.is_null() {
        return rt_make_bytes_zero(0);
    }

    unsafe {
        let str_obj = hex_str as *mut StrObj;
        let str_len = (*str_obj).len;
        let str_data = (*str_obj).data.as_ptr();

        // Skip whitespace and count hex digits
        let mut hex_chars = Vec::new();
        for i in 0..str_len {
            let c = *str_data.add(i);
            if c != b' ' && c != b'\t' && c != b'\n' && c != b'\r' {
                hex_chars.push(c);
            }
        }

        if hex_chars.len() % 2 != 0 {
            let msg = b"non-hexadecimal number found in fromhex() arg";
            exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
        }

        let byte_count = hex_chars.len() / 2;
        let mut bytes_vec = Vec::with_capacity(byte_count);

        for i in 0..byte_count {
            let high = hex_chars[i * 2];
            let low = hex_chars[i * 2 + 1];

            let high_val = match high {
                b'0'..=b'9' => high - b'0',
                b'a'..=b'f' => high - b'a' + 10,
                b'A'..=b'F' => high - b'A' + 10,
                _ => {
                    let msg = b"non-hexadecimal number found in fromhex() arg";
                    exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
                }
            };

            let low_val = match low {
                b'0'..=b'9' => low - b'0',
                b'a'..=b'f' => low - b'a' + 10,
                b'A'..=b'F' => low - b'A' + 10,
                _ => {
                    let msg = b"non-hexadecimal number found in fromhex() arg";
                    exceptions::rt_exc_raise_value_error(msg.as_ptr(), msg.len());
                }
            };

            bytes_vec.push((high_val << 4) | low_val);
        }

        rt_make_bytes(bytes_vec.as_ptr(), byte_count)
    }
}
