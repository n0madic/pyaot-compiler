use crate::gc::gc_alloc;
use crate::object::{BytesObj, Obj, ObjHeader, StrObj};
use crate::string::rt_make_str;
use pyaot_core_defs::TypeTagKind;
use pyaot_core_defs::Value;
use std::alloc::{alloc, dealloc, realloc, Layout};

/// StringIO object - in-memory text stream
#[repr(C)]
pub struct StringIOObj {
    pub header: ObjHeader,
    pub buffer: *mut u8, // Heap-allocated buffer
    pub len: usize,      // Current content length
    pub capacity: usize, // Buffer capacity
    pub position: usize, // Current read/write position
    pub closed: bool,    // Whether the stream is closed
}

/// BytesIO object - in-memory binary stream
#[repr(C)]
pub struct BytesIOObj {
    pub header: ObjHeader,
    pub buffer: *mut u8, // Heap-allocated buffer
    pub len: usize,      // Current content length
    pub capacity: usize, // Buffer capacity
    pub position: usize, // Current read/write position
    pub closed: bool,    // Whether the stream is closed
}

// Helper function to create bytes from slice
unsafe fn make_bytes_from_slice(data: &[u8]) -> *mut Obj {
    let len = data.len();
    let size = std::mem::size_of::<BytesObj>() + len;
    let obj = gc_alloc(size, TypeTagKind::Bytes.tag()) as *mut BytesObj;
    (*obj).len = len;
    if len > 0 {
        std::ptr::copy_nonoverlapping(data.as_ptr(), (*obj).data.as_mut_ptr(), len);
    }
    obj as *mut Obj
}

// Helper function to check if stream is closed
unsafe fn check_closed(closed: bool) {
    if closed {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "I/O operation on closed file"
        );
    }
}

// Helper function to ensure buffer capacity for StringIO/BytesIO
unsafe fn ensure_capacity(buffer: &mut *mut u8, capacity: &mut usize, needed: usize) {
    if needed > *capacity {
        let new_capacity = (needed.max(*capacity * 2)).max(16);
        if (*buffer).is_null() {
            let layout = Layout::from_size_align_unchecked(new_capacity, 1);
            *buffer = alloc(layout);
            if (*buffer).is_null() {
                raise_exc!(
                    pyaot_core_defs::BuiltinExceptionKind::MemoryError,
                    "StringIO/BytesIO allocation failed"
                );
            }
        } else {
            let old_layout = Layout::from_size_align_unchecked(*capacity, 1);
            let new_buf = realloc(*buffer, old_layout, new_capacity);
            if new_buf.is_null() {
                // realloc failure leaves the old buffer intact; raise without leaking it
                raise_exc!(
                    pyaot_core_defs::BuiltinExceptionKind::MemoryError,
                    "StringIO/BytesIO reallocation failed"
                );
            }
            *buffer = new_buf;
        }
        *capacity = new_capacity;
    }
}

// =============================================================================
// StringIO Implementation
// =============================================================================

/// Create a new StringIO object
pub unsafe fn rt_stringio_new(initial: *mut Obj) -> *mut Obj {
    let size = std::mem::size_of::<StringIOObj>();
    let obj = gc_alloc(size, TypeTagKind::StringIO.tag()) as *mut StringIOObj;

    (*obj).buffer = std::ptr::null_mut();
    (*obj).len = 0;
    (*obj).capacity = 0;
    (*obj).position = 0;
    (*obj).closed = false;

    // If initial string provided, copy its content
    if !initial.is_null() {
        if (*initial).header.type_tag != TypeTagKind::Str {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "initial_value must be str"
            );
        }
        let str_obj = initial as *const StrObj;
        let initial_len = (*str_obj).len;
        if initial_len > 0 {
            ensure_capacity(&mut (*obj).buffer, &mut (*obj).capacity, initial_len);
            std::ptr::copy_nonoverlapping((*str_obj).data.as_ptr(), (*obj).buffer, initial_len);
            (*obj).len = initial_len;
        }
    }

    obj as *mut Obj
}
#[export_name = "rt_stringio_new"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_new_abi(initial: Value) -> Value {
    Value::from_ptr(unsafe { rt_stringio_new(initial.unwrap_ptr()) })
}

/// Write string to StringIO, return number of characters written
pub unsafe fn rt_stringio_write(sio: *mut Obj, s: *mut Obj) -> i64 {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let str_obj = s as *const StrObj;
    let str_len = (*str_obj).len;

    if str_len == 0 {
        return 0;
    }

    // Calculate required capacity (write at current position) with overflow check
    let end_pos = (*sio_obj).position.checked_add(str_len).unwrap_or_else(|| {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::OverflowError,
            "StringIO write position overflow"
        )
    });
    ensure_capacity(&mut (*sio_obj).buffer, &mut (*sio_obj).capacity, end_pos);

    // Write data at current position
    std::ptr::copy_nonoverlapping(
        (*str_obj).data.as_ptr(),
        (*sio_obj).buffer.add((*sio_obj).position),
        str_len,
    );

    // Update position and length
    (*sio_obj).position += str_len;
    if (*sio_obj).position > (*sio_obj).len {
        (*sio_obj).len = (*sio_obj).position;
    }

    str_len as i64
}
#[export_name = "rt_stringio_write"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_write_abi(sio: Value, s: Value) -> i64 {
    unsafe { rt_stringio_write(sio.unwrap_ptr(), s.unwrap_ptr()) }
}

/// Read from StringIO
pub unsafe fn rt_stringio_read(sio: *mut Obj, size: i64) -> *mut Obj {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let remaining = (*sio_obj).len.saturating_sub((*sio_obj).position);

    // Determine how many bytes to read
    let to_read = if size < 0 {
        remaining
    } else {
        (size as usize).min(remaining)
    };

    if to_read == 0 {
        return rt_make_str(std::ptr::null(), 0);
    }

    let result = rt_make_str((*sio_obj).buffer.add((*sio_obj).position), to_read);
    (*sio_obj).position += to_read;

    result
}
#[export_name = "rt_stringio_read"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_read_abi(sio: Value, size: i64) -> Value {
    Value::from_ptr(unsafe { rt_stringio_read(sio.unwrap_ptr(), size) })
}

/// Read a line from StringIO (until newline or end)
pub unsafe fn rt_stringio_readline(sio: *mut Obj) -> *mut Obj {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let remaining = (*sio_obj).len.saturating_sub((*sio_obj).position);

    if remaining == 0 {
        return rt_make_str(std::ptr::null(), 0);
    }

    // Find newline
    let buffer_slice =
        std::slice::from_raw_parts((*sio_obj).buffer.add((*sio_obj).position), remaining);

    let line_len = buffer_slice
        .iter()
        .position(|&b| b == b'\n')
        .map(|pos| pos + 1) // Include the newline
        .unwrap_or(remaining);

    let result = rt_make_str((*sio_obj).buffer.add((*sio_obj).position), line_len);
    (*sio_obj).position += line_len;

    result
}
#[export_name = "rt_stringio_readline"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_readline_abi(sio: Value) -> Value {
    Value::from_ptr(unsafe { rt_stringio_readline(sio.unwrap_ptr()) })
}

/// Get the entire value of StringIO as a string
pub unsafe fn rt_stringio_getvalue(sio: *mut Obj) -> *mut Obj {
    let sio_obj = sio as *const StringIOObj;
    check_closed((*sio_obj).closed);

    if (*sio_obj).len == 0 {
        return rt_make_str(std::ptr::null(), 0);
    }

    rt_make_str((*sio_obj).buffer, (*sio_obj).len)
}
#[export_name = "rt_stringio_getvalue"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_getvalue_abi(sio: Value) -> Value {
    Value::from_ptr(unsafe { rt_stringio_getvalue(sio.unwrap_ptr()) })
}

/// Seek to a position in StringIO
pub unsafe fn rt_stringio_seek(sio: *mut Obj, pos: i64) -> i64 {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    if pos < 0 {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "Negative seek position"
        );
    }

    let new_pos = pos as usize; // Allow seeking past end of content
    (*sio_obj).position = new_pos;
    new_pos as i64
}
#[export_name = "rt_stringio_seek"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_seek_abi(sio: Value, pos: i64) -> i64 {
    unsafe { rt_stringio_seek(sio.unwrap_ptr(), pos) }
}

/// Get current position in StringIO
pub unsafe fn rt_stringio_tell(sio: *mut Obj) -> i64 {
    let sio_obj = sio as *const StringIOObj;
    check_closed((*sio_obj).closed);
    (*sio_obj).position as i64
}
#[export_name = "rt_stringio_tell"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_tell_abi(sio: Value) -> i64 {
    unsafe { rt_stringio_tell(sio.unwrap_ptr()) }
}

/// Close StringIO
pub unsafe fn rt_stringio_close(sio: *mut Obj) {
    let sio_obj = sio as *mut StringIOObj;
    (*sio_obj).closed = true;
}
#[export_name = "rt_stringio_close"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_close_abi(sio: Value) {
    unsafe { rt_stringio_close(sio.unwrap_ptr()) }
}

/// Truncate StringIO at given size (or current position if size=-1)
pub unsafe fn rt_stringio_truncate(sio: *mut Obj, size: i64) -> i64 {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let new_len = if size < 0 {
        (*sio_obj).position
    } else {
        // Clamp to current length: truncate cannot extend beyond existing content
        (size as usize).min((*sio_obj).len)
    };

    (*sio_obj).len = new_len;
    // CPython's truncate() does NOT change the stream position

    new_len as i64
}
#[export_name = "rt_stringio_truncate"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_truncate_abi(sio: Value, size: i64) -> i64 {
    unsafe { rt_stringio_truncate(sio.unwrap_ptr(), size) }
}

/// Finalize StringIO (called by GC)
pub unsafe fn stringio_finalize(obj: *mut Obj) {
    let sio_obj = obj as *mut StringIOObj;
    if !(*sio_obj).buffer.is_null() && (*sio_obj).capacity > 0 {
        let layout = Layout::from_size_align_unchecked((*sio_obj).capacity, 1);
        dealloc((*sio_obj).buffer, layout);
        (*sio_obj).buffer = std::ptr::null_mut();
    }
}

// =============================================================================
// BytesIO Implementation
// =============================================================================

/// Create a new BytesIO object
pub unsafe fn rt_bytesio_new(initial: *mut Obj) -> *mut Obj {
    let size = std::mem::size_of::<BytesIOObj>();
    let obj = gc_alloc(size, TypeTagKind::BytesIO.tag()) as *mut BytesIOObj;

    (*obj).buffer = std::ptr::null_mut();
    (*obj).len = 0;
    (*obj).capacity = 0;
    (*obj).position = 0;
    (*obj).closed = false;

    // If initial bytes provided, copy its content
    if !initial.is_null() {
        if (*initial).header.type_tag != TypeTagKind::Bytes {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::TypeError,
                "initial_bytes must be bytes"
            );
        }
        let bytes_obj = initial as *const BytesObj;
        let initial_len = (*bytes_obj).len;
        if initial_len > 0 {
            ensure_capacity(&mut (*obj).buffer, &mut (*obj).capacity, initial_len);
            std::ptr::copy_nonoverlapping((*bytes_obj).data.as_ptr(), (*obj).buffer, initial_len);
            (*obj).len = initial_len;
        }
    }

    obj as *mut Obj
}
#[export_name = "rt_bytesio_new"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytesio_new_abi(initial: Value) -> Value {
    Value::from_ptr(unsafe { rt_bytesio_new(initial.unwrap_ptr()) })
}

/// Write bytes to BytesIO, return number of bytes written
pub unsafe fn rt_bytesio_write(bio: *mut Obj, b: *mut Obj) -> i64 {
    let bio_obj = bio as *mut BytesIOObj;
    check_closed((*bio_obj).closed);

    let bytes_obj = b as *const BytesObj;
    let bytes_len = (*bytes_obj).len;

    if bytes_len == 0 {
        return 0;
    }

    // Calculate required capacity (write at current position) with overflow check
    let end_pos = (*bio_obj)
        .position
        .checked_add(bytes_len)
        .unwrap_or_else(|| {
            raise_exc!(
                pyaot_core_defs::BuiltinExceptionKind::OverflowError,
                "BytesIO write position overflow"
            )
        });
    ensure_capacity(&mut (*bio_obj).buffer, &mut (*bio_obj).capacity, end_pos);

    // Write data at current position
    std::ptr::copy_nonoverlapping(
        (*bytes_obj).data.as_ptr(),
        (*bio_obj).buffer.add((*bio_obj).position),
        bytes_len,
    );

    // Update position and length
    (*bio_obj).position += bytes_len;
    if (*bio_obj).position > (*bio_obj).len {
        (*bio_obj).len = (*bio_obj).position;
    }

    bytes_len as i64
}
#[export_name = "rt_bytesio_write"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytesio_write_abi(bio: Value, b: Value) -> i64 {
    unsafe { rt_bytesio_write(bio.unwrap_ptr(), b.unwrap_ptr()) }
}

/// Read from BytesIO
pub unsafe fn rt_bytesio_read(bio: *mut Obj, size: i64) -> *mut Obj {
    let bio_obj = bio as *mut BytesIOObj;
    check_closed((*bio_obj).closed);

    let remaining = (*bio_obj).len.saturating_sub((*bio_obj).position);

    // Determine how many bytes to read
    let to_read = if size < 0 {
        remaining
    } else {
        (size as usize).min(remaining)
    };

    if to_read == 0 {
        return make_bytes_from_slice(&[]);
    }

    let data_slice =
        std::slice::from_raw_parts((*bio_obj).buffer.add((*bio_obj).position), to_read);
    let result = make_bytes_from_slice(data_slice);
    (*bio_obj).position += to_read;

    result
}
#[export_name = "rt_bytesio_read"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytesio_read_abi(bio: Value, size: i64) -> Value {
    Value::from_ptr(unsafe { rt_bytesio_read(bio.unwrap_ptr(), size) })
}

/// Get the entire value of BytesIO as bytes
pub unsafe fn rt_bytesio_getvalue(bio: *mut Obj) -> *mut Obj {
    let bio_obj = bio as *const BytesIOObj;
    check_closed((*bio_obj).closed);

    if (*bio_obj).len == 0 {
        return make_bytes_from_slice(&[]);
    }

    let data_slice = std::slice::from_raw_parts((*bio_obj).buffer, (*bio_obj).len);
    make_bytes_from_slice(data_slice)
}
#[export_name = "rt_bytesio_getvalue"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytesio_getvalue_abi(bio: Value) -> Value {
    Value::from_ptr(unsafe { rt_bytesio_getvalue(bio.unwrap_ptr()) })
}

/// Seek to a position in BytesIO
pub unsafe fn rt_bytesio_seek(bio: *mut Obj, pos: i64) -> i64 {
    let bio_obj = bio as *mut BytesIOObj;
    check_closed((*bio_obj).closed);

    if pos < 0 {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "Negative seek position"
        );
    }

    let new_pos = pos as usize; // Allow seeking past end (matches CPython BytesIO)
    (*bio_obj).position = new_pos;
    new_pos as i64
}
#[export_name = "rt_bytesio_seek"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytesio_seek_abi(bio: Value, pos: i64) -> i64 {
    unsafe { rt_bytesio_seek(bio.unwrap_ptr(), pos) }
}

/// Get current position in BytesIO
pub unsafe fn rt_bytesio_tell(bio: *mut Obj) -> i64 {
    let bio_obj = bio as *const BytesIOObj;
    check_closed((*bio_obj).closed);
    (*bio_obj).position as i64
}
#[export_name = "rt_bytesio_tell"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytesio_tell_abi(bio: Value) -> i64 {
    unsafe { rt_bytesio_tell(bio.unwrap_ptr()) }
}

/// Close BytesIO
pub unsafe fn rt_bytesio_close(bio: *mut Obj) {
    let bio_obj = bio as *mut BytesIOObj;
    (*bio_obj).closed = true;
}
#[export_name = "rt_bytesio_close"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytesio_close_abi(bio: Value) {
    unsafe { rt_bytesio_close(bio.unwrap_ptr()) }
}

/// Finalize BytesIO (called by GC)
pub unsafe fn bytesio_finalize(obj: *mut Obj) {
    let bio_obj = obj as *mut BytesIOObj;
    if !(*bio_obj).buffer.is_null() && (*bio_obj).capacity > 0 {
        let layout = Layout::from_size_align_unchecked((*bio_obj).capacity, 1);
        dealloc((*bio_obj).buffer, layout);
        (*bio_obj).buffer = std::ptr::null_mut();
    }
}
