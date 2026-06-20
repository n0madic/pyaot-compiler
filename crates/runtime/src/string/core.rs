//! Core string operations: creation, data access, length, concatenation

#[allow(unused_imports)]
use crate::debug_assert_type_tag;
use crate::exceptions::ExceptionType;
use crate::gc;
use crate::object::{Obj, StrObj, TypeTagKind};
use pyaot_core_defs::Value;

/// Count codepoints in `data[..len]` using the non-continuation-byte rule
/// `(b & 0xC0) != 0x80`. This is the single source of truth for the
/// `StrObj::char_len` invariant — every allocation site either calls this or
/// derives the value arithmetically from inputs whose `char_len` was itself
/// produced by this rule (so even malformed UTF-8 stays self-consistent).
///
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
#[inline]
pub(crate) unsafe fn count_codepoints(data: *const u8, len: usize) -> usize {
    let mut count = 0usize;
    for i in 0..len {
        if (*data.add(i)) & 0xC0 != 0x80 {
            count += 1;
        }
    }
    count
}

/// Allocation size for a `StrObj` holding `byte_len` payload bytes.
/// Mandatory at every StrObj allocation site: the header/field part is
/// `size_of::<StrObj>()` (asserted equal to `offset_of!(StrObj, data)`),
/// so adding a field to `StrObj` can never silently under-allocate.
///
/// # Safety
/// Raises MemoryError on overflow (caller must be in a context where
/// `raise_exc!` is valid).
#[inline]
pub(crate) unsafe fn str_alloc_size(byte_len: usize) -> usize {
    std::mem::size_of::<StrObj>()
        .checked_add(byte_len)
        .unwrap_or_else(|| {
            raise_exc!(ExceptionType::MemoryError, "string size overflow");
        })
}

/// Create a new string object on the heap (internal implementation)
/// This is the low-level implementation that always allocates.
/// Use rt_make_str() for the public API that uses interning for single chars.
///
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
pub unsafe fn rt_make_str_impl(data: *const u8, len: usize) -> *mut Obj {
    use std::ptr;

    let raw_size = str_alloc_size(len);

    // Round up to slab size class for small strings to benefit from
    // O(1) bump allocation instead of system malloc. The minimum class is 32
    // because size_of::<StrObj>() is already 32 (header + len + char_len).
    let size = if raw_size <= 32 {
        32
    } else if raw_size <= 48 {
        48
    } else if raw_size <= 64 {
        64
    } else {
        raw_size
    };

    // Allocate using GC
    let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

    let str_obj = obj as *mut StrObj;
    (*str_obj).len = len;
    (*str_obj).char_len = if len > 0 && !data.is_null() {
        count_codepoints(data, len)
    } else {
        0
    };

    // Copy string data
    if len > 0 && !data.is_null() {
        ptr::copy_nonoverlapping(data, (*str_obj).data.as_mut_ptr(), len);
    }

    obj
}

/// Create a new string object on the heap
/// data: pointer to string bytes (not null-terminated)
/// len: length of the string in bytes
/// Returns: pointer to allocated StrObj
///
/// For single-byte strings, this will use the interned string pool
/// (populated lazily on first use of each byte value).
///
/// # Safety
/// If `len > 0`, `data` must be a valid pointer to at least `len` bytes.
pub unsafe fn rt_make_str(data: *const u8, len: usize) -> *mut Obj {
    // For single-byte strings, use the lazily-populated interned pool
    if len == 1 {
        use crate::string::rt_make_str_interned;
        return rt_make_str_interned(data, len);
    }

    rt_make_str_impl(data, len)
}
#[export_name = "rt_make_str"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_make_str_abi(data: *const u8, len: usize) -> Value {
    Value::from_ptr(unsafe { rt_make_str(data, len) })
}

/// Get the data pointer from a StrObj
/// Returns pointer to the string's byte data
pub fn rt_str_data(str_obj: *mut Obj) -> *const u8 {
    if str_obj.is_null() {
        return std::ptr::null();
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_data");
        let str_obj = str_obj as *mut StrObj;
        (*str_obj).data.as_ptr()
    }
}
#[export_name = "rt_str_data"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_data_abi(str_obj: Value) -> *const u8 {
    rt_str_data(str_obj.unwrap_ptr())
}

/// Get the length of a StrObj
pub fn rt_str_len(str_obj: *mut Obj) -> usize {
    if str_obj.is_null() {
        return 0;
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_len");
        let str_obj = str_obj as *mut StrObj;
        (*str_obj).len
    }
}
#[export_name = "rt_str_len"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_len_abi(str_obj: Value) -> usize {
    rt_str_len(str_obj.unwrap_ptr())
}

/// Get the length of a string (as i64 for Python's len()).
/// Returns the cached codepoint count (`StrObj::char_len`), matching
/// CPython's character-based len. Internal byte length is `rt_str_len`.
pub fn rt_str_len_int(str_obj: *mut Obj) -> i64 {
    if str_obj.is_null() {
        return 0;
    }
    unsafe {
        debug_assert_type_tag!(str_obj, TypeTagKind::Str, "rt_str_len_int");
        let str_obj = str_obj as *mut StrObj;
        // Shared debug validator for the char_len invariant: this is the
        // hottest read of the cache, so every runtime test and debug corpus
        // run re-checks that allocation sites filled char_len correctly.
        debug_assert_eq!(
            (*str_obj).char_len,
            count_codepoints((*str_obj).data.as_ptr(), (*str_obj).len),
            "StrObj::char_len cache out of sync with data"
        );
        (*str_obj).char_len as i64
    }
}
#[export_name = "rt_str_len_int"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_len_int_abi(str_obj: Value) -> i64 {
    rt_str_len_int(str_obj.unwrap_ptr())
}

/// Concatenate two strings
/// Returns: pointer to new allocated StrObj
pub fn rt_str_concat(a: *mut Obj, b: *mut Obj) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    use std::ptr;

    if a.is_null() || b.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(a, TypeTagKind::Str, "rt_str_concat");
        debug_assert_type_tag!(b, TypeTagKind::Str, "rt_str_concat");
        let str_a = a as *mut StrObj;
        let str_b = b as *mut StrObj;

        let len_a = (*str_a).len;
        let len_b = (*str_b).len;
        // Read char_len BEFORE gc_alloc (concat of two valid caches is exact).
        let total_char_len = (*str_a).char_len + (*str_b).char_len;
        let total_len = match len_a.checked_add(len_b) {
            Some(l) => l,
            None => {
                raise_exc!(
                    ExceptionType::OverflowError,
                    "string concatenation result is too long"
                );
            }
        };

        let size = str_alloc_size(total_len);

        // Root a and b across gc_alloc: a GC collection triggered inside
        // gc_alloc would free a or b if they are not reachable from the shadow
        // stack.  We re-derive str_a/str_b after gc_alloc to ensure we read
        // from the still-live objects (the GC is non-moving, so addresses are
        // unchanged, but re-deriving makes the live-range explicit).
        let mut roots: [*mut Obj; 2] = [a, b];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        // Allocate using GC (may collect; a and b stay alive via shadow frame)
        let obj = gc::gc_alloc(size, TypeTagKind::Str as u8);

        gc_pop();

        // Re-derive after gc_alloc.
        let str_a = a as *mut StrObj;
        let str_b = b as *mut StrObj;

        let str_obj = obj as *mut StrObj;
        (*str_obj).len = total_len;
        (*str_obj).char_len = total_char_len;

        // Copy data from both strings
        if len_a > 0 {
            ptr::copy_nonoverlapping((*str_a).data.as_ptr(), (*str_obj).data.as_mut_ptr(), len_a);
        }
        if len_b > 0 {
            ptr::copy_nonoverlapping(
                (*str_b).data.as_ptr(),
                (*str_obj).data.as_mut_ptr().add(len_a),
                len_b,
            );
        }

        obj
    }
}
#[export_name = "rt_str_concat"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_concat_abi(a: Value, b: Value) -> Value {
    Value::from_ptr(rt_str_concat(a.unwrap_ptr(), b.unwrap_ptr()))
}

/// The codecs `str.encode` / `bytes.decode` honor (§9). Anything else is an
/// unknown encoding (`LookupError`). `errors=` is NOT modeled (a documented
/// limit) — an out-of-range byte/codepoint always raises.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Encoding {
    Utf8,
    Ascii,
    Latin1,
    Unknown,
}

/// Classify an encoding-name str (null ⇒ utf-8, the default codec). The name is
/// normalized — lower-cased with `-`/`_`/space stripped — so "UTF-8", "utf_8",
/// "latin-1", "ISO-8859-1", "US-ASCII" all resolve. Only the codecs the runtime
/// implements are recognized; everything else is `Unknown` (the caller raises a
/// `LookupError`). §9.
///
/// # Safety
/// `enc` must be null or a valid `StrObj` pointer.
pub(crate) unsafe fn classify_encoding(enc: *mut Obj) -> Encoding {
    if enc.is_null() {
        return Encoding::Utf8;
    }
    let s = enc as *mut StrObj;
    let bytes = std::slice::from_raw_parts((*s).data.as_ptr(), (*s).len);
    let mut norm: Vec<u8> = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == b'-' || b == b'_' || b == b' ' {
            continue;
        }
        norm.push(b.to_ascii_lowercase());
    }
    match norm.as_slice() {
        b"utf8" => Encoding::Utf8,
        b"ascii" | b"usascii" | b"646" => Encoding::Ascii,
        b"latin1" | b"iso88591" | b"latin" | b"l1" | b"8859" | b"cp819" => Encoding::Latin1,
        _ => Encoding::Unknown,
    }
}

/// Encode string to bytes honoring the encoding (§9): `utf-8` (default) is the
/// identity copy; `ascii` raises `UnicodeEncodeError` on any non-ASCII byte;
/// `latin-1` maps each codepoint to one byte, raising `UnicodeEncodeError` for a
/// codepoint > 0xFF; an unknown encoding name raises `LookupError`. `errors=` is
/// not modeled.
/// encoding: pointer to encoding string (utf-8 default if null)
/// Returns: pointer to allocated BytesObj
pub fn rt_str_encode(s: *mut Obj, encoding: *mut Obj) -> *mut Obj {
    use crate::bytes::rt_make_bytes;
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    if s.is_null() {
        return unsafe { rt_make_bytes(std::ptr::null(), 0) };
    }

    unsafe {
        debug_assert_type_tag!(s, TypeTagKind::Str, "rt_str_encode");
        let str_obj = s as *mut StrObj;
        let len = (*str_obj).len;
        let data = (*str_obj).data.as_ptr();
        let bytes = std::slice::from_raw_parts(data, len);

        match classify_encoding(encoding) {
            Encoding::Unknown => raise_exc!(ExceptionType::LookupError, "unknown encoding"),
            Encoding::Ascii => {
                if bytes.iter().any(|&b| b >= 0x80) {
                    raise_exc!(
                        ExceptionType::UnicodeEncodeError,
                        "'ascii' codec can't encode character: ordinal not in range(128)"
                    );
                }
                // All ASCII ⇒ the UTF-8 bytes are already the ASCII bytes.
            }
            Encoding::Latin1 => {
                // Map each codepoint to a single byte; raise for any > 0xFF. The
                // result Vec is independent of the GC heap, so no rooting needed.
                let src = std::str::from_utf8(bytes).unwrap_or_else(|_| {
                    raise_exc!(ExceptionType::ValueError, "invalid UTF-8 in string")
                });
                let mut out: Vec<u8> = Vec::with_capacity(len);
                for ch in src.chars() {
                    let cp = ch as u32;
                    if cp > 0xFF {
                        raise_exc!(
                            ExceptionType::UnicodeEncodeError,
                            "'latin-1' codec can't encode character: ordinal not in range(256)"
                        );
                    }
                    out.push(cp as u8);
                }
                return rt_make_bytes(out.as_ptr(), out.len());
            }
            Encoding::Utf8 => {}
        }

        // utf-8 / validated-ascii: identity copy. Root `s` across rt_make_bytes →
        // gc_alloc (a collection could free the StrObj and invalidate `data`).
        let mut roots: [*mut Obj; 1] = [s];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 1,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);
        let result = rt_make_bytes(data, len);
        gc_pop();
        result
    }
}
#[export_name = "rt_str_encode"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_encode_abi(s: Value, encoding: Value) -> Value {
    Value::from_ptr(rt_str_encode(s.unwrap_ptr(), encoding.unwrap_ptr()))
}
