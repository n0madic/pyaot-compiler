//! File I/O operations for the Python AOT compiler
//!
//! This module provides file I/O support including open(), read(), write(), close(),
//! and context manager protocol (__enter__/__exit__).

use crate::gc::gc_alloc;
use crate::object::{FileMode, FileObj, Obj, StrObj, TypeTagKind};
use std::fs::OpenOptions;
use std::io::{Read, Seek, Write};
use std::ptr;

/// Open a file with the given mode
/// Returns a FileObj pointer, or raises IOError on failure
///
/// Supported modes: r, w, a, rb, wb, ab, r+, w+, a+, r+b/rb+, w+b/wb+, a+b/ab+
///
/// # Safety
/// `filename` must be a valid pointer to a StrObj.
/// `mode` must be a valid pointer to a StrObj containing a supported mode string.
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

    // Parse mode — CPython accepts both "r+b" and "rb+" orderings
    let (file_mode, binary) = match mode_str.as_str() {
        "r" => (FileMode::Read, false),
        "w" => (FileMode::Write, false),
        "a" => (FileMode::Append, false),
        "rb" => (FileMode::ReadBinary, true),
        "wb" => (FileMode::WriteBinary, true),
        "ab" => (FileMode::AppendBinary, true),
        "r+" => (FileMode::ReadWrite, false),
        "w+" => (FileMode::WriteRead, false),
        "a+" => (FileMode::AppendRead, false),
        "r+b" | "rb+" => (FileMode::ReadWriteBinary, true),
        "w+b" | "wb+" => (FileMode::WriteReadBinary, true),
        "a+b" | "ab+" => (FileMode::AppendReadBinary, true),
        _ => {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "invalid mode: '{}'",
                mode_str
            );
        }
    };

    // Open the file with appropriate options
    let file_result = match file_mode {
        // Read-only
        FileMode::Read | FileMode::ReadBinary => OpenOptions::new().read(true).open(&filename_str),
        // Write-only (truncate)
        FileMode::Write | FileMode::WriteBinary => OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&filename_str),
        // Append-only
        FileMode::Append | FileMode::AppendBinary => OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filename_str),
        // Read+write (file must exist)
        FileMode::ReadWrite | FileMode::ReadWriteBinary => OpenOptions::new()
            .read(true)
            .write(true)
            .open(&filename_str),
        // Write+read (truncate, create)
        FileMode::WriteRead | FileMode::WriteReadBinary => OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&filename_str),
        // Append+read (create)
        FileMode::AppendRead | FileMode::AppendReadBinary => OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
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
            crate::utils::raise_io_error_owned(msg);
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

    // Cap unbounded reads at 1 GB to prevent OOM on unexpectedly large files.
    // Reads that need more than this should use read(n) with explicit sizes.
    const MAX_READ_ALL_SIZE: u64 = 1 << 30; // 1 GB
                                            // Read one byte beyond the limit so we can detect overflow without consuming
                                            // more memory than necessary.
    let mut limited = Read::take(handle, MAX_READ_ALL_SIZE + 1);
    let mut buffer = Vec::new();

    match limited.read_to_end(&mut buffer) {
        Ok(_) => {
            if buffer.len() as u64 > MAX_READ_ALL_SIZE {
                let msg = b"read(): file too large; use read(n) for files over 1 GB";
                crate::exceptions::rt_exc_raise(
                    pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
                    msg.as_ptr(),
                    msg.len(),
                );
            }
            if (*file_obj).binary {
                // Return bytes object
                crate::bytes::rt_make_bytes(buffer.as_ptr(), buffer.len())
            } else {
                // Return string object
                crate::string::rt_make_str(buffer.as_ptr(), buffer.len())
            }
        }
        Err(e) => {
            crate::utils::raise_io_error_owned(format!("read error: {}", e));
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

    let n = n as usize;
    // Cap allocation to prevent OOM from absurd sizes
    const MAX_READ_SIZE: usize = 1 << 30; // 1 GB
    if n > MAX_READ_SIZE {
        let msg = b"ValueError: read size too large";
        crate::exceptions::rt_exc_raise(
            pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            msg.as_ptr(),
            msg.len(),
        );
    }

    let handle = &mut *(*file_obj).handle;
    let mut buffer = vec![0u8; n];

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
            crate::utils::raise_io_error_owned(format!("read error: {}", e));
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

    // Read in chunks for efficiency instead of 1-byte-at-a-time syscalls.
    // After finding a newline, seek back so subsequent reads start correctly.
    const CHUNK_SIZE: usize = 4096;
    let mut buf = [0u8; CHUNK_SIZE];
    loop {
        match handle.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                if let Some(pos) = buf[..n].iter().position(|&b| b == b'\n') {
                    // Found newline — take up to and including it
                    line.extend_from_slice(&buf[..=pos]);
                    // Seek back to right after the newline
                    let rewind = (n - pos - 1) as i64;
                    if rewind > 0 {
                        #[cfg(debug_assertions)]
                        if let Err(e) = handle.seek(std::io::SeekFrom::Current(-rewind)) {
                            eprintln!("Warning: readline seek failed: {}", e);
                        }
                        #[cfg(not(debug_assertions))]
                        let _ = handle.seek(std::io::SeekFrom::Current(-rewind));
                    }
                    break;
                } else {
                    // No newline in this chunk — append all and keep reading
                    line.extend_from_slice(&buf[..n]);
                }
            }
            Err(e) => {
                crate::utils::raise_io_error_owned(format!("read error: {}", e));
            }
        }
    }

    if (*file_obj).binary {
        // Return bytes object for binary mode
        crate::bytes::rt_make_bytes(line.as_ptr(), line.len())
    } else {
        // Return string for text mode
        crate::string::rt_make_str(line.as_ptr(), line.len())
    }
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

    // Cap unbounded reads at 1 GB to prevent OOM on unexpectedly large files.
    const MAX_READ_ALL_SIZE: u64 = 1 << 30; // 1 GB
    let mut limited = Read::take(handle, MAX_READ_ALL_SIZE + 1);
    let mut raw = Vec::new();
    if let Err(e) = limited.read_to_end(&mut raw) {
        crate::utils::raise_io_error_owned(format!("read error: {}", e));
    }
    if raw.len() as u64 > MAX_READ_ALL_SIZE {
        let msg = b"readlines(): file too large; use read(n) for files over 1 GB";
        crate::exceptions::rt_exc_raise(
            pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            msg.as_ptr(),
            msg.len(),
        );
    }
    let content = match String::from_utf8(raw) {
        Ok(s) => s,
        Err(e) => {
            crate::utils::raise_io_error_owned(format!("read error: invalid UTF-8: {}", e));
        }
    };

    // Create a list to hold the lines
    let list = crate::list::rt_make_list(0, crate::object::ELEM_HEAP_OBJ);

    // Split on '\n' to preserve whether the file ended with a newline.
    // Splitting "a\nb\n" yields ["a", "b", ""] — the trailing empty
    // element signals that the content ended with '\n' and should not
    // become an extra empty line in the output.
    let lines: Vec<&str> = content.split('\n').collect();
    for (idx, line) in lines.iter().enumerate() {
        let is_last = idx == lines.len() - 1;
        if is_last && line.is_empty() {
            // Trailing '\n' produced an empty last element — skip it.
            break;
        }
        let line_str = if is_last && !content.ends_with('\n') {
            // Last line has no trailing newline — don't add one.
            line.to_string()
        } else {
            format!("{}\n", line)
        };
        let line_obj = crate::string::rt_make_str(line_str.as_ptr(), line_str.len());
        crate::list::rt_list_push(list, line_obj);
    }

    list
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
            crate::utils::raise_io_error_owned(format!("write error: {}", e));
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
        crate::utils::raise_io_error_owned(format!("flush error: {}", e));
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
    match FileMode::try_from((*file_obj).mode) {
        Ok(mode) => matches!(
            mode,
            FileMode::Read
                | FileMode::ReadBinary
                | FileMode::ReadWrite
                | FileMode::WriteRead
                | FileMode::AppendRead
                | FileMode::ReadWriteBinary
                | FileMode::WriteReadBinary
                | FileMode::AppendReadBinary
        ),
        Err(_) => false,
    }
}

/// Check if file is writable
unsafe fn is_writable(file_obj: *mut FileObj) -> bool {
    match FileMode::try_from((*file_obj).mode) {
        Ok(mode) => matches!(
            mode,
            FileMode::Write
                | FileMode::WriteBinary
                | FileMode::Append
                | FileMode::AppendBinary
                | FileMode::ReadWrite
                | FileMode::WriteRead
                | FileMode::AppendRead
                | FileMode::ReadWriteBinary
                | FileMode::WriteReadBinary
                | FileMode::AppendReadBinary
        ),
        Err(_) => false,
    }
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
