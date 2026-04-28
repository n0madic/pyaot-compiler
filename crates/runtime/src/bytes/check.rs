//! Bytes check operations: startswith, endswith

use crate::object::Obj;
use pyaot_core_defs::Value;

/// Check if bytes starts with prefix
/// Returns: 1 (true) or 0 (false)
pub fn rt_bytes_startswith(bytes: *mut Obj, prefix: *mut Obj) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || prefix.is_null() {
        return 0;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let prefix_obj = prefix as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let prefix_len = (*prefix_obj).len;

        if prefix_len > bytes_len {
            return 0;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let prefix_data = (*prefix_obj).data.as_ptr();

        for i in 0..prefix_len {
            if *bytes_data.add(i) != *prefix_data.add(i) {
                return 0;
            }
        }

        1
    }
}
#[export_name = "rt_bytes_startswith"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_startswith_abi(bytes: Value, prefix: Value) -> i64 {
    rt_bytes_startswith(bytes.unwrap_ptr(), prefix.unwrap_ptr())
}


/// Check if bytes ends with suffix
/// Returns: 1 (true) or 0 (false)
pub fn rt_bytes_endswith(bytes: *mut Obj, suffix: *mut Obj) -> i64 {
    use crate::object::BytesObj;

    if bytes.is_null() || suffix.is_null() {
        return 0;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let suffix_obj = suffix as *mut BytesObj;

        let bytes_len = (*bytes_obj).len;
        let suffix_len = (*suffix_obj).len;

        if suffix_len > bytes_len {
            return 0;
        }

        let bytes_data = (*bytes_obj).data.as_ptr();
        let suffix_data = (*suffix_obj).data.as_ptr();
        let offset = bytes_len - suffix_len;

        for i in 0..suffix_len {
            if *bytes_data.add(offset + i) != *suffix_data.add(i) {
                return 0;
            }
        }

        1
    }
}
#[export_name = "rt_bytes_endswith"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_endswith_abi(bytes: Value, suffix: Value) -> i64 {
    rt_bytes_endswith(bytes.unwrap_ptr(), suffix.unwrap_ptr())
}

