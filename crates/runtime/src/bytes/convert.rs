//! Bytes conversion operations: decode, fromhex

use crate::object::Obj;
use pyaot_core_defs::Value;

use super::core::{rt_make_bytes, rt_make_bytes_zero};

/// Decode bytes to string honoring the encoding (§9): `utf-8` (default)
/// validates UTF-8 and raises `UnicodeDecodeError` on invalid input (the former
/// code copied blindly — a latent bug); `ascii` raises `UnicodeDecodeError` on
/// any byte ≥ 0x80; `latin-1` maps each byte to its codepoint (always valid); an
/// unknown encoding name raises `LookupError`. `errors=` is not modeled.
/// encoding: pointer to StrObj for encoding name (null for utf-8)
/// Returns: pointer to allocated StrObj
pub fn rt_bytes_decode(bytes: *mut Obj, encoding: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use crate::object::BytesObj;
    use crate::string::{classify_encoding, rt_make_str, Encoding};

    if bytes.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();
        let slice = std::slice::from_raw_parts(data, len);

        match classify_encoding(encoding) {
            Encoding::Unknown => raise_exc!(
                crate::exceptions::ExceptionType::LookupError,
                "unknown encoding"
            ),
            Encoding::Utf8 => {
                if std::str::from_utf8(slice).is_err() {
                    raise_exc!(
                        crate::exceptions::ExceptionType::UnicodeDecodeError,
                        "'utf-8' codec can't decode bytes: invalid utf-8"
                    );
                }
            }
            Encoding::Ascii => {
                if slice.iter().any(|&b| b >= 0x80) {
                    raise_exc!(
                        crate::exceptions::ExceptionType::UnicodeDecodeError,
                        "'ascii' codec can't decode byte: ordinal not in range(128)"
                    );
                }
            }
            Encoding::Latin1 => {
                // Every byte maps to a codepoint U+0000..U+00FF (always valid).
                // Build the UTF-8 string in an owned buffer (GC-independent).
                let mut out = String::with_capacity(len);
                for &b in slice {
                    out.push(b as char);
                }
                return rt_make_str(out.as_ptr(), out.len());
            }
        }

        // utf-8 / validated-ascii: identity copy. Root `bytes` across rt_make_str
        // → gc_alloc (a collection could free the BytesObj and invalidate `data`).
        let mut roots: [*mut Obj; 1] = [bytes];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);
        let result = rt_make_str(data, len);
        gc_pop();
        result
    }
}
#[export_name = "rt_bytes_decode"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_decode_abi(bytes: Value, encoding: Value) -> Value {
    Value::from_ptr(rt_bytes_decode(bytes.unwrap_ptr(), encoding.unwrap_ptr()))
}

/// Create bytes from hex string
/// hex_str: pointer to StrObj containing hex digits
/// Returns: pointer to new BytesObj
pub fn rt_bytes_fromhex(hex_str: *mut Obj) -> *mut Obj {
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
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "non-hexadecimal number found in fromhex() arg"
            );
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
                    raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "non-hexadecimal number found in fromhex() arg"
                    );
                }
            };

            let low_val = match low {
                b'0'..=b'9' => low - b'0',
                b'a'..=b'f' => low - b'a' + 10,
                b'A'..=b'F' => low - b'A' + 10,
                _ => {
                    raise_exc!(
                        crate::exceptions::ExceptionType::ValueError,
                        "non-hexadecimal number found in fromhex() arg"
                    );
                }
            };

            bytes_vec.push((high_val << 4) | low_val);
        }

        rt_make_bytes(bytes_vec.as_ptr(), byte_count)
    }
}
#[export_name = "rt_bytes_fromhex"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_fromhex_abi(hex_str: Value) -> Value {
    Value::from_ptr(rt_bytes_fromhex(hex_str.unwrap_ptr()))
}
