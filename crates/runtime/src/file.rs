//! File I/O operations for the Python AOT compiler
//!
//! This module provides file I/O support including open(), read(), write(), close(),
//! and context manager protocol (__enter__/__exit__).

use crate::gc::gc_alloc;
use crate::object::{FileMode, FileObj, Obj, StrObj, TypeTagKind};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::ptr;

/// Open a file with the given mode
/// Returns a FileObj pointer, or raises IOError on failure
///
/// # Safety
/// `filename` must be a valid pointer to a StrObj.
/// `mode` must be a valid pointer to a StrObj containing "r", "w", "a", "rb", "wb", or "ab".
#[no_mangle]
pub unsafe extern "C" fn rt_file_open(filename: *mut Obj, mode: *mut Obj) -> *mut Obj {
    if filename.is_null() {
        crate::utils::raise_io_error("filename cannot be None");
    }

    // Get filename string
    let filename_str = crate::utils::extract_str_unchecked(filename);

    // Get mode string (default to "r" if null)
    let mode_str = if mode.is_null() {
        String::from("r")
    } else {
        crate::utils::extract_str_unchecked(mode)
    };

    // Parse mode
    let (file_mode, binary) = match mode_str.as_str() {
        "r" => (FileMode::Read, false),
        "w" => (FileMode::Write, false),
        "a" => (FileMode::Append, false),
        "rb" => (FileMode::ReadBinary, true),
        "wb" => (FileMode::WriteBinary, true),
        "ab" => (FileMode::AppendBinary, true),
        _ => {
            let msg = format!("invalid mode: '{}'", mode_str);
            crate::utils::raise_io_error(&msg);
        }
    };

    // Open the file
    let file_result = match file_mode {
        FileMode::Read | FileMode::ReadBinary => OpenOptions::new().read(true).open(&filename_str),
        FileMode::Write | FileMode::WriteBinary => OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&filename_str),
        FileMode::Append | FileMode::AppendBinary => OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filename_str),
    };

    match file_result {
        Ok(file) => {
            // Allocate FileObj
            let size = std::mem::size_of::<FileObj>();
            let obj = gc_alloc(size, TypeTagKind::File as u8) as *mut FileObj;

            // Box the file handle and leak it
            let file_box = Box::new(file);
            let file_ptr = Box::into_raw(file_box);

            (*obj).handle = file_ptr;
            (*obj).mode = file_mode as u8;
            (*obj).closed = false;
            (*obj).binary = binary;
            (*obj).name = filename;

            obj as *mut Obj
        }
        Err(e) => {
            let msg = match e.kind() {
                std::io::ErrorKind::NotFound => {
                    format!("No such file or directory: '{}'", filename_str)
                }
                std::io::ErrorKind::PermissionDenied => {
                    format!("Permission denied: '{}'", filename_str)
                }
                _ => format!("{}: '{}'", e, filename_str),
            };
            crate::utils::raise_io_error(&msg);
        }
    }
}

/// Read the entire file contents as a string (text mode) or bytes (binary mode)
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_read(file: *mut Obj) -> *mut Obj {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    if !is_readable(file_obj) {
        crate::utils::raise_io_error("not readable");
    }

    let handle = &mut *(*file_obj).handle;
    let mut buffer = Vec::new();

    match handle.read_to_end(&mut buffer) {
        Ok(_) => {
            if (*file_obj).binary {
                // Return bytes object
                crate::bytes::rt_make_bytes(buffer.as_ptr(), buffer.len())
            } else {
                // Return string object
                crate::string::rt_make_str(buffer.as_ptr(), buffer.len())
            }
        }
        Err(e) => {
            let msg = format!("read error: {}", e);
            crate::utils::raise_io_error(&msg);
        }
    }
}

/// Read up to n bytes/characters from the file
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_read_n(file: *mut Obj, n: i64) -> *mut Obj {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    if !is_readable(file_obj) {
        crate::utils::raise_io_error("not readable");
    }

    if n < 0 {
        // Negative n means read all
        return rt_file_read(file);
    }

    let handle = &mut *(*file_obj).handle;
    let mut buffer = vec![0u8; n as usize];

    match handle.read(&mut buffer) {
        Ok(bytes_read) => {
            buffer.truncate(bytes_read);
            if (*file_obj).binary {
                crate::bytes::rt_make_bytes(buffer.as_ptr(), buffer.len())
            } else {
                crate::string::rt_make_str(buffer.as_ptr(), buffer.len())
            }
        }
        Err(e) => {
            let msg = format!("read error: {}", e);
            crate::utils::raise_io_error(&msg);
        }
    }
}

/// Read a single line from the file (including the newline character if present)
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_readline(file: *mut Obj) -> *mut Obj {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    if !is_readable(file_obj) {
        crate::utils::raise_io_error("not readable");
    }

    let handle = &mut *(*file_obj).handle;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];

    loop {
        match handle.read(&mut byte) {
            Ok(0) => break, // EOF
            Ok(_) => {
                line.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(e) => {
                let msg = format!("read error: {}", e);
                crate::utils::raise_io_error(&msg);
            }
        }
    }

    crate::string::rt_make_str(line.as_ptr(), line.len())
}

/// Read all lines from the file as a list of strings
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_readlines(file: *mut Obj) -> *mut Obj {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    if !is_readable(file_obj) {
        crate::utils::raise_io_error("not readable");
    }

    let handle = &mut *(*file_obj).handle;
    let mut content = String::new();

    match handle.read_to_string(&mut content) {
        Ok(_) => {
            // Create a list to hold the lines
            let list = crate::list::rt_make_list(0, crate::object::ELEM_HEAP_OBJ);

            for line in content.lines() {
                // Add newline back (except for last line if original didn't have it)
                let line_with_newline = format!("{}\n", line);
                let line_obj =
                    crate::string::rt_make_str(line_with_newline.as_ptr(), line_with_newline.len());
                crate::list::rt_list_push(list, line_obj);
            }

            list
        }
        Err(e) => {
            let msg = format!("read error: {}", e);
            crate::utils::raise_io_error(&msg);
        }
    }
}

/// Write data to the file
/// Returns the number of bytes/characters written
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
/// `data` must be a valid pointer to a StrObj or BytesObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_write(file: *mut Obj, data: *mut Obj) -> i64 {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    if !is_writable(file_obj) {
        crate::utils::raise_io_error("not writable");
    }

    if data.is_null() {
        crate::utils::raise_value_error("write argument must be str or bytes, not None");
    }

    let handle = &mut *(*file_obj).handle;

    // Get data bytes
    let (data_ptr, data_len) = match (*data).header.type_tag {
        TypeTagKind::Str => {
            let str_obj = data as *const StrObj;
            ((*str_obj).data.as_ptr(), (*str_obj).len)
        }
        TypeTagKind::Bytes => {
            let bytes_obj = data as *const crate::object::BytesObj;
            ((*bytes_obj).data.as_ptr(), (*bytes_obj).len)
        }
        _ => {
            crate::utils::raise_value_error("write argument must be str or bytes");
        }
    };

    let bytes = std::slice::from_raw_parts(data_ptr, data_len);

    match handle.write(bytes) {
        Ok(written) => written as i64,
        Err(e) => {
            let msg = format!("write error: {}", e);
            crate::utils::raise_io_error(&msg);
        }
    }
}

/// Close the file
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_close(file: *mut Obj) {
    if file.is_null() {
        return;
    }

    let file_obj = file as *mut FileObj;

    if (*file_obj).closed {
        return; // Already closed
    }

    // Drop the file handle to close it
    if !(*file_obj).handle.is_null() {
        let _ = Box::from_raw((*file_obj).handle);
        (*file_obj).handle = ptr::null_mut();
    }

    (*file_obj).closed = true;
}

/// Flush the file buffer
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_flush(file: *mut Obj) {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    let handle = &mut *(*file_obj).handle;

    if let Err(e) = handle.flush() {
        let msg = format!("flush error: {}", e);
        crate::utils::raise_io_error(&msg);
    }
}

/// Context manager __enter__ - returns self
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_enter(file: *mut Obj) -> *mut Obj {
    check_file_valid(file);
    file
}

/// Context manager __exit__ - closes the file and returns False
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_exit(file: *mut Obj) -> i8 {
    rt_file_close(file);
    0 // Return False (don't suppress exceptions)
}

/// Check if the file is closed
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_is_closed(file: *mut Obj) -> i8 {
    if file.is_null() {
        return 1;
    }
    let file_obj = file as *mut FileObj;
    if (*file_obj).closed {
        1
    } else {
        0
    }
}

/// Get the filename of the file
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
#[no_mangle]
pub unsafe extern "C" fn rt_file_name(file: *mut Obj) -> *mut Obj {
    if file.is_null() {
        return crate::string::rt_make_str(ptr::null(), 0);
    }
    let file_obj = file as *mut FileObj;
    (*file_obj).name
}

// ==================== Helper functions ====================

/// Check if file is valid (not null and not closed)
unsafe fn check_file_valid(file: *mut Obj) {
    if file.is_null() {
        crate::utils::raise_value_error("I/O operation on closed file");
    }
    let file_obj = file as *mut FileObj;
    if (*file_obj).closed {
        crate::utils::raise_value_error("I/O operation on closed file");
    }
    if (*file_obj).handle.is_null() {
        crate::utils::raise_value_error("I/O operation on closed file");
    }
}

/// Check if file is readable
unsafe fn is_readable(file_obj: *mut FileObj) -> bool {
    let mode = FileMode::try_from((*file_obj).mode).expect("is_readable: invalid file mode");
    matches!(mode, FileMode::Read | FileMode::ReadBinary)
}

/// Check if file is writable
unsafe fn is_writable(file_obj: *mut FileObj) -> bool {
    let mode = FileMode::try_from((*file_obj).mode).expect("is_writable: invalid file mode");
    matches!(
        mode,
        FileMode::Write | FileMode::WriteBinary | FileMode::Append | FileMode::AppendBinary
    )
}

/// Close file during GC sweep if still open (safety net)
/// Called from gc.rs when sweeping FileObj
///
/// # Safety
/// `file` must be a valid pointer to a FileObj, or null.
pub unsafe fn file_finalize(file: *mut Obj) {
    if file.is_null() {
        return;
    }
    let file_obj = file as *mut FileObj;
    if !(*file_obj).closed && !(*file_obj).handle.is_null() {
        let _ = Box::from_raw((*file_obj).handle);
        (*file_obj).handle = ptr::null_mut();
        (*file_obj).closed = true;
    }
}
