//! Print and input operations for runtime objects

use crate::gc;
use crate::object::{BytesObj, Obj, ObjHeader, StrObj, TypeTagKind};
use std::sync::atomic::{AtomicU8, Ordering};

static PRINT_TARGET: AtomicU8 = AtomicU8::new(0); // 0=stdout, 1=stderr

pub fn is_stderr_target() -> bool {
    PRINT_TARGET.load(Ordering::Relaxed) == 1
}

#[no_mangle]
pub extern "C" fn rt_print_set_stderr() {
    PRINT_TARGET.store(1, Ordering::Relaxed);
}

#[no_mangle]
pub extern "C" fn rt_print_set_stdout() {
    PRINT_TARGET.store(0, Ordering::Relaxed);
}

#[no_mangle]
pub extern "C" fn rt_print_flush() {
    use std::io::Write;
    if is_stderr_target() {
        let _ = std::io::stderr().flush();
    } else {
        let _ = std::io::stdout().flush();
    }
}

/// Print a StrObj (heap-allocated string)
#[no_mangle]
pub extern "C" fn rt_print_str_obj(str_obj: *mut Obj) {
    if str_obj.is_null() {
        return;
    }
    unsafe {
        let str_obj = str_obj as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);
        if let Ok(s) = std::str::from_utf8(bytes) {
            if is_stderr_target() {
                eprint!("{}", s);
            } else {
                print!("{}", s);
            }
        }
    }
}

/// Format a bytes slice as a Python bytes literal (e.g. `b'hello\n'`).
fn format_bytes_repr(data: *const u8, len: usize) -> String {
    let mut s = String::with_capacity(len + 3); // b'' + escapes
    s.push_str("b'");
    for i in 0..len {
        let byte = unsafe { *data.add(i) };
        match byte {
            b'\\' => s.push_str("\\\\"),
            b'\'' => s.push_str("\\'"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7E => s.push(byte as char),
            _ => {
                use std::fmt::Write;
                let _ = write!(s, "\\x{:02x}", byte);
            }
        }
    }
    s.push('\'');
    s
}

/// Print a BytesObj (heap-allocated bytes)
#[no_mangle]
pub extern "C" fn rt_print_bytes_obj(bytes: *mut Obj) {
    if bytes.is_null() {
        if is_stderr_target() {
            eprint!("b''");
        } else {
            print!("b''");
        }
        return;
    }

    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let len = (*bytes_obj).len;
        let data = (*bytes_obj).data.as_ptr();
        let repr = format_bytes_repr(data, len);
        if is_stderr_target() {
            eprint!("{}", repr);
        } else {
            print!("{}", repr);
        }
    }
}

/// Read a line from stdin after printing the prompt
/// Returns: pointer to allocated StrObj
#[no_mangle]
pub extern "C" fn rt_input(prompt: *mut Obj) -> *mut Obj {
    use std::io::{self, BufRead, Write};

    // Print prompt if provided
    if !prompt.is_null() {
        unsafe {
            if (*prompt).header.type_tag != TypeTagKind::Str {
                raise_exc!(
                    crate::exceptions::ExceptionType::TypeError,
                    "prompt must be a string"
                );
            }
            let str_obj = prompt as *mut StrObj;
            let len = (*str_obj).len;
            let data = (*str_obj).data.as_ptr();
            let bytes = std::slice::from_raw_parts(data, len);
            if let Ok(s) = std::str::from_utf8(bytes) {
                print!("{}", s);
            }
            let _ = io::stdout().flush();
        }
    }

    // Read line from stdin
    let stdin = io::stdin();
    let mut line = String::new();
    match stdin.lock().read_line(&mut line) {
        Ok(0) => {
            // EOF — raise EOFError like CPython
            unsafe {
                raise_exc!(
                    crate::exceptions::ExceptionType::EOFError,
                    "EOF when reading a line"
                );
            }
        }
        Ok(_) => {
            // Remove trailing newline
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }

            // Allocate and return string
            let bytes = line.as_bytes();
            unsafe {
                let size =
                    std::mem::size_of::<ObjHeader>() + std::mem::size_of::<usize>() + bytes.len();
                let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);
                let str_obj = obj as *mut StrObj;
                (*str_obj).len = bytes.len();
                if !bytes.is_empty() {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        (*str_obj).data.as_mut_ptr(),
                        bytes.len(),
                    );
                }
                obj
            }
        }
        Err(e) => {
            // I/O error — raise IOError (CPython raises OSError for I/O errors, not EOFError)
            unsafe {
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::IOError,
                    "error reading from stdin: {}",
                    e
                );
            }
        }
    }
}
