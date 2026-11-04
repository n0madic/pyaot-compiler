use crate::gc::gc_alloc;
use crate::object::{BytesObj, Obj, ObjHeader, StrObj};
use crate::string::rt_make_str;
use pyaot_core_defs::TypeTagKind;
use pyaot_core_defs::Value;
use std::alloc::{alloc, dealloc, realloc, Layout};

/// StringIO object - in-memory text stream.
///
/// The buffer holds one `u32` code point per character (CPython's UCS-4
/// model), so every position, size and length below counts CHARACTERS: `tell`,
/// `seek`, `read(n)` and `truncate(n)` agree with CPython on non-ASCII text,
/// and a read can never split a multi-byte UTF-8 sequence. UTF-8 is only
/// decoded on `write`/construction and re-encoded on `read`/`getvalue`.
#[repr(C)]
pub struct StringIOObj {
    pub header: ObjHeader,
    pub buffer: *mut u32, // Heap-allocated code-point buffer
    pub len: usize,       // Current content length (characters)
    pub capacity: usize,  // Buffer capacity (characters)
    pub position: usize,  // Current read/write position (characters)
    pub closed: bool,     // Whether the stream is closed
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

/// Decode a runtime str's UTF-8 bytes into code points. Tolerant of the
/// surrogate code points the runtime's str encoding can carry (3-byte
/// `ED A0..BF ..` sequences), which `std::str` would reject.
fn decode_code_points(bytes: &[u8], out: &mut Vec<u32>) {
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as u32;
        let (cp, width) = if b0 < 0x80 {
            (b0, 1)
        } else if b0 >> 5 == 0b110 && i + 1 < bytes.len() {
            (((b0 & 0x1F) << 6) | (bytes[i + 1] as u32 & 0x3F), 2)
        } else if b0 >> 4 == 0b1110 && i + 2 < bytes.len() {
            (
                ((b0 & 0x0F) << 12)
                    | ((bytes[i + 1] as u32 & 0x3F) << 6)
                    | (bytes[i + 2] as u32 & 0x3F),
                3,
            )
        } else if b0 >> 3 == 0b11110 && i + 3 < bytes.len() {
            (
                ((b0 & 0x07) << 18)
                    | ((bytes[i + 1] as u32 & 0x3F) << 12)
                    | ((bytes[i + 2] as u32 & 0x3F) << 6)
                    | (bytes[i + 3] as u32 & 0x3F),
                4,
            )
        } else {
            // Malformed / truncated sequence: keep the byte as U+FFFD
            // rather than reading past the end.
            (0xFFFD, 1)
        };
        out.push(cp);
        i += width;
    }
}

/// Encode code points back to the runtime's UTF-8 str bytes (surrogates as
/// their 3-byte form, mirroring `decode_code_points`).
fn encode_code_points(cps: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cps.len());
    for &cp in cps {
        if cp < 0x80 {
            out.push(cp as u8);
        } else if cp < 0x800 {
            out.push(0xC0 | (cp >> 6) as u8);
            out.push(0x80 | (cp & 0x3F) as u8);
        } else if cp < 0x10000 {
            out.push(0xE0 | (cp >> 12) as u8);
            out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
            out.push(0x80 | (cp & 0x3F) as u8);
        } else {
            out.push(0xF0 | (cp >> 18) as u8);
            out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
            out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
            out.push(0x80 | (cp & 0x3F) as u8);
        }
    }
    out
}

/// A runtime str holding the code points `cps`.
unsafe fn make_str_from_code_points(cps: &[u32]) -> *mut Obj {
    if cps.is_empty() {
        return rt_make_str(std::ptr::null(), 0);
    }
    let bytes = encode_code_points(cps);
    rt_make_str(bytes.as_ptr(), bytes.len())
}

/// The code points of the str object `s` (raises TypeError on a non-str).
unsafe fn str_code_points(s: *mut Obj, what: &str) -> Vec<u32> {
    if s.is_null() || !Value(s as u64).is_ptr() || (*s).header.type_tag != TypeTagKind::Str {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::TypeError,
            "{} must be str",
            what
        );
    }
    let str_obj = s as *const StrObj;
    let bytes = std::slice::from_raw_parts((*str_obj).data.as_ptr(), (*str_obj).len);
    let mut cps = Vec::with_capacity(bytes.len());
    decode_code_points(bytes, &mut cps);
    cps
}

/// Grow the code-point buffer to hold at least `needed` characters.
unsafe fn ensure_char_capacity(sio: *mut StringIOObj, needed: usize) {
    if needed <= (*sio).capacity {
        return;
    }
    let new_capacity = needed.max((*sio).capacity.saturating_mul(2)).max(16);
    let new_layout = match Layout::array::<u32>(new_capacity) {
        Ok(l) => l,
        Err(_) => raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::MemoryError,
            "StringIO size overflow"
        ),
    };
    let new_buf = if (*sio).buffer.is_null() {
        alloc(new_layout)
    } else {
        // The old layout was valid when it was allocated.
        let old_layout = Layout::array::<u32>((*sio).capacity).unwrap_unchecked();
        realloc((*sio).buffer as *mut u8, old_layout, new_layout.size())
    } as *mut u32;
    if new_buf.is_null() {
        // realloc failure leaves the old buffer intact; raise without leaking it
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::MemoryError,
            "StringIO allocation failed"
        );
    }
    (*sio).buffer = new_buf;
    (*sio).capacity = new_capacity;
}

/// The live content `[0, len)` as a slice.
unsafe fn content<'a>(sio: *const StringIOObj) -> &'a [u32] {
    if (*sio).len == 0 {
        return &[];
    }
    std::slice::from_raw_parts((*sio).buffer, (*sio).len)
}

/// Create a new StringIO object
pub unsafe fn rt_stringio_new(initial: *mut Obj) -> *mut Obj {
    // Decode the initial value BEFORE allocating the object: decoding cannot
    // collect, but keeping all fallible work first keeps `initial` the only
    // object that must survive the allocation (the caller roots it).
    let initial_cps = if initial.is_null() || Value(initial as u64).is_none() {
        Vec::new()
    } else {
        str_code_points(initial, "initial_value")
    };

    let size = std::mem::size_of::<StringIOObj>();
    let obj = gc_alloc(size, TypeTagKind::StringIO.tag()) as *mut StringIOObj;

    (*obj).buffer = std::ptr::null_mut();
    (*obj).len = 0;
    (*obj).capacity = 0;
    (*obj).position = 0;
    (*obj).closed = false;

    if !initial_cps.is_empty() {
        ensure_char_capacity(obj, initial_cps.len());
        std::ptr::copy_nonoverlapping(initial_cps.as_ptr(), (*obj).buffer, initial_cps.len());
        (*obj).len = initial_cps.len();
    }

    obj as *mut Obj
}
#[export_name = "rt_stringio_new"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_new_abi(initial: Value) -> Value {
    Value::from_ptr(unsafe { rt_stringio_new(initial.0 as *mut Obj) })
}

/// Write string to StringIO, return number of characters written
pub unsafe fn rt_stringio_write(sio: *mut Obj, s: *mut Obj) -> i64 {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let cps = str_code_points(s, "write() argument");
    if cps.is_empty() {
        return 0;
    }

    // Calculate required capacity (write at current position) with overflow check
    let pos = (*sio_obj).position;
    let end_pos = pos.checked_add(cps.len()).unwrap_or_else(|| {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::OverflowError,
            "StringIO write position overflow"
        )
    });
    ensure_char_capacity(sio_obj, end_pos);

    // A prior seek past the end leaves a gap [len..position): CPython fills
    // it with NUL characters (and it must never expose uninitialized memory).
    if pos > (*sio_obj).len {
        std::ptr::write_bytes((*sio_obj).buffer.add((*sio_obj).len), 0, pos - (*sio_obj).len);
    }

    std::ptr::copy_nonoverlapping(cps.as_ptr(), (*sio_obj).buffer.add(pos), cps.len());

    (*sio_obj).position = end_pos;
    if end_pos > (*sio_obj).len {
        (*sio_obj).len = end_pos;
    }

    cps.len() as i64
}
#[export_name = "rt_stringio_write"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_write_abi(sio: Value, s: Value) -> i64 {
    unsafe { rt_stringio_write(sio.unwrap_ptr(), s.0 as *mut Obj) }
}

/// Read up to `size` characters (all remaining when `size < 0`).
pub unsafe fn rt_stringio_read(sio: *mut Obj, size: i64) -> *mut Obj {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let data = content(sio_obj);
    let start = (*sio_obj).position.min(data.len());
    let remaining = data.len() - start;
    let to_read = if size < 0 {
        remaining
    } else {
        (size as usize).min(remaining)
    };
    // Position only advances over what was actually read (a position past the
    // end stays where it is, as in CPython).
    if to_read > 0 {
        (*sio_obj).position = start + to_read;
    }
    make_str_from_code_points(&data[start..start + to_read])
}
#[export_name = "rt_stringio_read"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_read_abi(sio: Value, size: i64) -> Value {
    Value::from_ptr(unsafe { rt_stringio_read(sio.unwrap_ptr(), size) })
}

/// Read a line from StringIO (until and including `\n`, or to the end)
pub unsafe fn rt_stringio_readline(sio: *mut Obj) -> *mut Obj {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let data = content(sio_obj);
    let start = (*sio_obj).position.min(data.len());
    let rest = &data[start..];
    let line_len = rest
        .iter()
        .position(|&c| c == '\n' as u32)
        .map(|i| i + 1) // Include the newline
        .unwrap_or(rest.len());
    if line_len > 0 {
        (*sio_obj).position = start + line_len;
    }
    make_str_from_code_points(&rest[..line_len])
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
    make_str_from_code_points(content(sio_obj))
}
#[export_name = "rt_stringio_getvalue"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_getvalue_abi(sio: Value) -> Value {
    Value::from_ptr(unsafe { rt_stringio_getvalue(sio.unwrap_ptr()) })
}

/// Seek to a character position in StringIO (past the end is allowed)
pub unsafe fn rt_stringio_seek(sio: *mut Obj, pos: i64) -> i64 {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    if pos < 0 {
        raise_exc!(
            pyaot_core_defs::BuiltinExceptionKind::ValueError,
            "Negative seek position {}",
            pos
        );
    }

    (*sio_obj).position = pos as usize;
    pos
}
#[export_name = "rt_stringio_seek"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_stringio_seek_abi(sio: Value, pos: i64) -> i64 {
    unsafe { rt_stringio_seek(sio.unwrap_ptr(), pos) }
}

/// Get the current character position in StringIO
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

/// Truncate StringIO at `size` characters (or the current position if
/// `size < 0`)
pub unsafe fn rt_stringio_truncate(sio: *mut Obj, size: i64) -> i64 {
    let sio_obj = sio as *mut StringIOObj;
    check_closed((*sio_obj).closed);

    let requested = if size < 0 {
        (*sio_obj).position
    } else {
        size as usize
    };
    // Truncate never extends: a position/size past the end (e.g. after
    // `seek(100000)`) leaves the content alone — growing `len` past the
    // written characters would expose memory beyond the buffer.
    (*sio_obj).len = requested.min((*sio_obj).len);
    // CPython's truncate() does NOT change the stream position, and returns
    // the requested size, not the resulting length.

    requested as i64
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
        let layout = Layout::array::<u32>((*sio_obj).capacity).unwrap_unchecked();
        dealloc((*sio_obj).buffer as *mut u8, layout);
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

    // If a prior seek moved position past the valid end, the gap
    // [len..position] is uninitialized heap memory (alloc/realloc is not
    // zeroed). CPython fills such gaps with NUL; zero it before the write so
    // getvalue()/read() never expose uninitialized bytes (Rust UB).
    if (*bio_obj).position > (*bio_obj).len {
        std::ptr::write_bytes(
            (*bio_obj).buffer.add((*bio_obj).len),
            0,
            (*bio_obj).position - (*bio_obj).len,
        );
    }

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
