//! File I/O operations for the Python AOT compiler
//!
//! This module provides file I/O support including open(), read(), write(), close(),
//! and context manager protocol (__enter__/__exit__).

use crate::gc::gc_alloc;
use crate::object::{FileEncoding, FileMode, FileObj, Obj, StrObj, TypeTagKind};
use pyaot_core_defs::Value;
use std::fs::OpenOptions;
use std::io::{Read, Seek, Write};
use std::ptr;

/// Parse an encoding string to FileEncoding enum.
/// Returns Utf8 for null/empty. Raises LookupError for unsupported encodings.
unsafe fn parse_encoding(encoding: *mut Obj) -> FileEncoding {
    if encoding.is_null() {
        return FileEncoding::Utf8;
    }
    if (*encoding).header.type_tag != TypeTagKind::Str {
        return FileEncoding::Utf8;
    }
    let enc_str = crate::utils::extract_str_unchecked(encoding);
    // Normalize: lowercase, strip hyphens/underscores (CPython does this)
    let normalized: String = enc_str
        .to_ascii_lowercase()
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .collect();
    match normalized.as_str() {
        "utf8" | "utf" => FileEncoding::Utf8,
        "ascii" | "usascii" => FileEncoding::Ascii,
        "latin1" | "latin" | "iso88591" | "iso885915" | "8859" => FileEncoding::Latin1,
        _ => {
            raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "unknown encoding: '{}'",
                enc_str
            );
        }
    }
}

/// Decode raw bytes to UTF-8 string according to encoding.
/// For UTF-8: validates (returns error on invalid).
/// For ASCII: validates 0-127 range.
/// For Latin-1: maps bytes 0-255 to Unicode codepoints (always succeeds).
fn decode_bytes(data: &[u8], encoding: FileEncoding) -> std::result::Result<String, String> {
    match encoding {
        FileEncoding::Utf8 => {
            String::from_utf8(data.to_vec()).map_err(|e| format!("invalid UTF-8: {}", e))
        }
        FileEncoding::Ascii => {
            if let Some(pos) = data.iter().position(|&b| b > 127) {
                Err(format!(
                    "'ascii' codec can't decode byte 0x{:02x} in position {}",
                    data[pos], pos
                ))
            } else {
                // ASCII is valid UTF-8
                Ok(String::from_utf8(data.to_vec()).unwrap())
            }
        }
        FileEncoding::Latin1 => {
            // Latin-1: each byte maps directly to a Unicode codepoint
            Ok(data.iter().map(|&b| b as char).collect())
        }
    }
}

/// Encode a UTF-8 string to bytes according to encoding.
/// For UTF-8: identity (internal representation is already UTF-8).
/// For ASCII: validates 0-127 range.
/// For Latin-1: maps Unicode codepoints 0-255 (fails for > 255).
fn encode_str(s: &str, encoding: FileEncoding) -> std::result::Result<Vec<u8>, String> {
    match encoding {
        FileEncoding::Utf8 => Ok(s.as_bytes().to_vec()),
        FileEncoding::Ascii => {
            if let Some(pos) = s.bytes().position(|b| b > 127) {
                Err(format!(
                    "'ascii' codec can't encode character '{}' in position {}",
                    s[pos..].chars().next().unwrap_or('?'),
                    pos
                ))
            } else {
                Ok(s.as_bytes().to_vec())
            }
        }
        FileEncoding::Latin1 => {
            let mut result = Vec::with_capacity(s.len());
            for (pos, ch) in s.chars().enumerate() {
                let cp = ch as u32;
                if cp > 255 {
                    return Err(format!(
                        "'latin-1' codec can't encode character '\\u{:04x}' in position {}",
                        cp, pos
                    ));
                }
                result.push(cp as u8);
            }
            Ok(result)
        }
    }
}

/// Open a file with the given mode and optional encoding
/// Returns a FileObj pointer, or raises IOError on failure
///
/// Supported modes: r, w, a, rb, wb, ab, r+, w+, a+, r+b/rb+, w+b/wb+, a+b/ab+
/// Supported encodings: utf-8 (default), ascii, latin-1/iso-8859-1
///
/// # Safety
/// `filename` must be a valid pointer to a StrObj.
/// `mode` must be a valid pointer to a StrObj containing a supported mode string.
/// `encoding` must be a valid pointer to a StrObj or null (defaults to utf-8).
pub unsafe fn rt_file_open(
    filename: *mut Obj,
    mode: *mut Obj,
    encoding: *mut Obj,
) -> *mut Obj {
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

    // Parse encoding (default to utf-8; ignored for binary modes)
    let file_encoding = parse_encoding(encoding);

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
            (*obj).encoding = file_encoding as u8;
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
            raise_exc_string!(crate::exceptions::ExceptionType::IOError, msg);
        }
    }
}
#[export_name = "rt_file_open"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_open_abi(
    filename: Value,
    mode: Value,
    encoding: Value,
) -> Value {
    Value::from_ptr(unsafe { rt_file_open(filename.unwrap_ptr(), mode.unwrap_ptr(), encoding.unwrap_ptr()) })
}


/// Read the entire file contents as a string (text mode) or bytes (binary mode)
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_read(file: *mut Obj) -> *mut Obj {
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
                raise_exc!(
                    pyaot_core_defs::BuiltinExceptionKind::ValueError,
                    "read(): file too large; use read(n) for files over 1 GB"
                );
            }
            if (*file_obj).binary {
                // Return bytes object
                crate::bytes::rt_make_bytes(buffer.as_ptr(), buffer.len())
            } else {
                // Text mode: decode bytes according to file encoding
                let enc = get_encoding(file_obj);
                match decode_bytes(&buffer, enc) {
                    Ok(s) => crate::string::rt_make_str(s.as_ptr(), s.len()),
                    Err(e) => {
                        raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "read error: {}",
                            e
                        );
                    }
                }
            }
        }
        Err(e) => {
            raise_exc!(
                crate::exceptions::ExceptionType::IOError,
                "read error: {}",
                e
            );
        }
    }
}
#[export_name = "rt_file_read"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_read_abi(file: Value) -> Value {
    Value::from_ptr(unsafe { rt_file_read(file.unwrap_ptr()) })
}


/// Read up to n bytes/characters from the file
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_read_n(file: *mut Obj, n: i64) -> *mut Obj {
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
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "read size too large"
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
                let enc = get_encoding(file_obj);
                match decode_bytes(&buffer, enc) {
                    Ok(s) => crate::string::rt_make_str(s.as_ptr(), s.len()),
                    Err(e) => {
                        raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "read error: {}",
                            e
                        );
                    }
                }
            }
        }
        Err(e) => {
            raise_exc!(
                crate::exceptions::ExceptionType::IOError,
                "read error: {}",
                e
            );
        }
    }
}
#[export_name = "rt_file_read_n"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_read_n_abi(file: Value, n: i64) -> Value {
    Value::from_ptr(unsafe { rt_file_read_n(file.unwrap_ptr(), n) })
}


/// Read a single line from the file (including the newline character if present)
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_readline(file: *mut Obj) -> *mut Obj {
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
                raise_exc!(
                    crate::exceptions::ExceptionType::IOError,
                    "read error: {}",
                    e
                );
            }
        }
    }

    if (*file_obj).binary {
        // Return bytes object for binary mode
        crate::bytes::rt_make_bytes(line.as_ptr(), line.len())
    } else {
        // Text mode: decode according to file encoding
        let enc = get_encoding(file_obj);
        match decode_bytes(&line, enc) {
            Ok(s) => crate::string::rt_make_str(s.as_ptr(), s.len()),
            Err(e) => {
                raise_exc!(
                    crate::exceptions::ExceptionType::IOError,
                    "read error: {}",
                    e
                );
            }
        }
    }
}
#[export_name = "rt_file_readline"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_readline_abi(file: Value) -> Value {
    Value::from_ptr(unsafe { rt_file_readline(file.unwrap_ptr()) })
}


/// Read all lines from the file as a list of strings (text mode) or list of bytes (binary mode)
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_readlines(file: *mut Obj) -> *mut Obj {
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
        raise_exc!(
            crate::exceptions::ExceptionType::IOError,
            "read error: {}",
            e
        );
    }
    if raw.len() as u64 > MAX_READ_ALL_SIZE {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "readlines(): file too large; use read(n) for files over 1 GB"
        );
    }

    let list = crate::list::rt_make_list(0);

    // Root [list, line_obj_slot] across all allocating calls (rt_make_str,
    // rt_make_bytes, rt_list_push).  Without rooting, a GC triggered by any
    // of those functions sweeps `list` while we still hold a pointer to it.
    let mut roots: [*mut Obj; 2] = [list, std::ptr::null_mut()];
    let mut frame = crate::gc::ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 2,
        roots: roots.as_mut_ptr(),
    };
    crate::gc::gc_push(&mut frame);

    if (*file_obj).binary {
        // Binary mode: split on b'\n', return list[bytes]
        // Each line includes the trailing b'\n' (except possibly the last)
        let mut start = 0;
        for i in 0..raw.len() {
            if raw[i] == b'\n' {
                let line = &raw[start..=i]; // include the newline
                let line_obj = crate::bytes::rt_make_bytes(line.as_ptr(), line.len());
                roots[1] = line_obj; // keep line_obj alive across rt_list_push
                crate::list::rt_list_push(roots[0], roots[1]);
                roots[1] = std::ptr::null_mut();
                start = i + 1;
            }
        }
        // Remaining bytes after last newline (if any)
        if start < raw.len() {
            let line = &raw[start..];
            let line_obj = crate::bytes::rt_make_bytes(line.as_ptr(), line.len());
            roots[1] = line_obj;
            crate::list::rt_list_push(roots[0], roots[1]);
        }
    } else {
        // Text mode: decode according to file encoding, then split on '\n'
        let enc = get_encoding(file_obj);
        let content = match decode_bytes(&raw, enc) {
            Ok(s) => s,
            Err(e) => {
                crate::gc::gc_pop();
                raise_exc!(
                    crate::exceptions::ExceptionType::IOError,
                    "read error: {}",
                    e
                );
            }
        };

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
            roots[1] = line_obj; // keep line_obj alive across rt_list_push
            crate::list::rt_list_push(roots[0], roots[1]);
            roots[1] = std::ptr::null_mut();
        }
    }

    crate::gc::gc_pop();
    roots[0] // live list pointer
}
#[export_name = "rt_file_readlines"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_readlines_abi(file: Value) -> Value {
    Value::from_ptr(unsafe { rt_file_readlines(file.unwrap_ptr()) })
}


/// Write data to the file
/// Returns the number of bytes/characters written
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
/// `data` must be a valid pointer to a StrObj or BytesObj.
pub unsafe fn rt_file_write(file: *mut Obj, data: *mut Obj) -> i64 {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    if !is_writable(file_obj) {
        crate::utils::raise_io_error("not writable");
    }

    if data.is_null() {
        crate::utils::raise_value_error("write argument must be str or bytes, not None");
    }

    let handle = &mut *(*file_obj).handle;

    // Get data bytes, encoding strings according to file encoding
    let encoded_buf: Vec<u8>;
    let bytes = match (*data).header.type_tag {
        TypeTagKind::Str => {
            let str_obj = data as *const StrObj;
            let data_ptr = (*str_obj).data.as_ptr();
            let data_len = (*str_obj).len;
            let raw = std::slice::from_raw_parts(data_ptr, data_len);
            let enc = get_encoding(file_obj);
            if matches!(enc, FileEncoding::Utf8) {
                // UTF-8 is our internal representation — write directly
                raw
            } else {
                // Encode the UTF-8 string to the target encoding
                let s = std::str::from_utf8(raw).unwrap_or("");
                encoded_buf = match encode_str(s, enc) {
                    Ok(b) => b,
                    Err(e) => {
                        raise_exc!(
                            crate::exceptions::ExceptionType::IOError,
                            "write error: {}",
                            e
                        );
                    }
                };
                &encoded_buf
            }
        }
        TypeTagKind::Bytes => {
            let bytes_obj = data as *const crate::object::BytesObj;
            let data_ptr = (*bytes_obj).data.as_ptr();
            let data_len = (*bytes_obj).len;
            std::slice::from_raw_parts(data_ptr, data_len)
        }
        _ => {
            crate::utils::raise_value_error("write argument must be str or bytes");
        }
    };

    match handle.write(bytes) {
        Ok(written) => written as i64,
        Err(e) => {
            raise_exc!(
                crate::exceptions::ExceptionType::IOError,
                "write error: {}",
                e
            );
        }
    }
}
#[export_name = "rt_file_write"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_write_abi(file: Value, data: Value) -> i64 {
    unsafe { rt_file_write(file.unwrap_ptr(), data.unwrap_ptr()) }
}


/// Close the file
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_close(file: *mut Obj) {
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
#[export_name = "rt_file_close"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_close_abi(file: Value) {
    unsafe { rt_file_close(file.unwrap_ptr()) }
}


/// Flush the file buffer
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_flush(file: *mut Obj) {
    check_file_valid(file);
    let file_obj = file as *mut FileObj;

    let handle = &mut *(*file_obj).handle;

    if let Err(e) = handle.flush() {
        raise_exc!(
            crate::exceptions::ExceptionType::IOError,
            "flush error: {}",
            e
        );
    }
}
#[export_name = "rt_file_flush"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_flush_abi(file: Value) {
    unsafe { rt_file_flush(file.unwrap_ptr()) }
}


/// Context manager __enter__ - returns self
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_enter(file: *mut Obj) -> *mut Obj {
    check_file_valid(file);
    file
}
#[export_name = "rt_file_enter"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_enter_abi(file: Value) -> Value {
    Value::from_ptr(unsafe { rt_file_enter(file.unwrap_ptr()) })
}


/// Context manager __exit__ - closes the file and returns False
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_exit(file: *mut Obj) -> i8 {
    rt_file_close(file);
    0 // Return False (don't suppress exceptions)
}
#[export_name = "rt_file_exit"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_exit_abi(file: Value) -> i8 {
    unsafe { rt_file_exit(file.unwrap_ptr()) }
}


/// Check if the file is closed
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_is_closed(file: *mut Obj) -> i8 {
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
#[export_name = "rt_file_is_closed"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_is_closed_abi(file: Value) -> i8 {
    unsafe { rt_file_is_closed(file.unwrap_ptr()) }
}


/// Get the filename of the file
///
/// # Safety
/// `file` must be a valid pointer to a FileObj.
pub unsafe fn rt_file_name(file: *mut Obj) -> *mut Obj {
    if file.is_null() {
        return crate::string::rt_make_str(ptr::null(), 0);
    }
    let file_obj = file as *mut FileObj;
    (*file_obj).name
}
#[export_name = "rt_file_name"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_file_name_abi(file: Value) -> Value {
    Value::from_ptr(unsafe { rt_file_name(file.unwrap_ptr()) })
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

/// Get the file's encoding as a FileEncoding enum
unsafe fn get_encoding(file_obj: *mut FileObj) -> FileEncoding {
    match (*file_obj).encoding {
        0 => FileEncoding::Utf8,
        1 => FileEncoding::Ascii,
        2 => FileEncoding::Latin1,
        _ => FileEncoding::Utf8, // fallback
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
