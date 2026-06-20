//! Text codecs for `str.encode` / `bytes.decode` (§9).
//!
//! Recognized encodings: `utf-8` (default), `ascii`, `latin-1`, the algorithmic
//! Unicode codecs `utf-16`/`utf-32` (native-with-BOM, plus explicit `-le`/`-be`),
//! and the single-byte charmap codecs (`iso-8859-2..16`, `cp125x`, `koi8-*`,
//! `mac-*`, DOS/IBM code pages, …) driven off the generated [`codec_tables`].
//! Multi-byte CJK codecs (`shift_jis`/`gbk`/`big5`) are out of scope and raise
//! `LookupError`, as does any unrecognized name. The `errors=` handler is
//! honored: `strict` (default, raise), `ignore`, `replace`, `backslashreplace`,
//! and `xmlcharrefreplace` (encode only). An unknown handler name raises
//! `LookupError`, but — like CPython — only when an actual coding error triggers
//! it, so clean data with a bogus handler still succeeds.
//!
//! Messages are intentionally simplified (no byte-position detail); the
//! differential corpus checks the *result* and exception *type*, not the text.

use crate::object::{BytesObj, Obj, StrObj};
use crate::string::codec_tables::{
    SINGLE_BYTE_CODEC_NAMES, SINGLE_BYTE_NAMES, SINGLE_BYTE_TABLES, UNDEF,
};
use pyaot_core_defs::Value;

/// Byte order for the Unicode multi-byte codecs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ByteOrder {
    /// `utf-16`/`utf-32`: BOM on encode, BOM-detect (else native) on decode.
    Native,
    Le,
    Be,
}

/// The codecs `str.encode` / `bytes.decode` honor (§9).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    Utf8,
    Ascii,
    Latin1,
    Utf16(ByteOrder),
    Utf32(ByteOrder),
    /// A single-byte charmap codec — index into [`SINGLE_BYTE_TABLES`].
    SingleByte(usize),
    Unknown,
}

/// An `errors=` handler. `Unknown` carries no name and is resolved lazily: it
/// only raises `LookupError` if a coding error actually invokes it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorHandler {
    Strict,
    Ignore,
    Replace,
    BackslashReplace,
    XmlCharRefReplace,
    Unknown,
}

/// Lower-case and strip `-`/`_`/space from an encoding/handler name.
unsafe fn normalize(name: *mut Obj) -> Vec<u8> {
    let s = name as *mut StrObj;
    let bytes = std::slice::from_raw_parts((*s).data.as_ptr(), (*s).len);
    let mut norm = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == b'-' || b == b'_' || b == b' ' {
            continue;
        }
        norm.push(b.to_ascii_lowercase());
    }
    norm
}

/// Classify an encoding-name str (null ⇒ utf-8, the default). Names are
/// normalized so "UTF-8", "utf_8", "latin-1", "ISO-8859-1", "US-ASCII",
/// "UTF-16LE" all resolve. Unknown ⇒ `Encoding::Unknown` (caller raises).
///
/// # Safety
/// `enc` must be null or a valid `StrObj` pointer.
pub(crate) unsafe fn classify_encoding(enc: *mut Obj) -> Encoding {
    if enc.is_null() {
        return Encoding::Utf8;
    }
    match normalize(enc).as_slice() {
        b"utf8" => Encoding::Utf8,
        b"ascii" | b"usascii" | b"646" => Encoding::Ascii,
        b"latin1" | b"iso88591" | b"latin" | b"l1" | b"8859" | b"cp819" => Encoding::Latin1,
        b"utf16" | b"u16" | b"utf" => Encoding::Utf16(ByteOrder::Native),
        b"utf16le" | b"unicodelittleunmarked" => Encoding::Utf16(ByteOrder::Le),
        b"utf16be" | b"unicodebigunmarked" => Encoding::Utf16(ByteOrder::Be),
        b"utf32" | b"u32" => Encoding::Utf32(ByteOrder::Native),
        b"utf32le" => Encoding::Utf32(ByteOrder::Le),
        b"utf32be" => Encoding::Utf32(ByteOrder::Be),
        // Fall back to the generated single-byte charmap table; still Unknown if
        // unlisted (a multi-byte CJK codec or a genuinely unknown name).
        other => single_byte_lookup(other),
    }
}

/// Resolve a normalized name against the generated single-byte codec table
/// ([`SINGLE_BYTE_NAMES`] is sorted, so binary-search it).
fn single_byte_lookup(norm: &[u8]) -> Encoding {
    match SINGLE_BYTE_NAMES.binary_search_by(|&(n, _)| n.as_bytes().cmp(norm)) {
        Ok(i) => Encoding::SingleByte(SINGLE_BYTE_NAMES[i].1),
        Err(_) => Encoding::Unknown,
    }
}

/// Parse an `errors=` handler name (null ⇒ `strict`). Unknown names do NOT raise
/// here — they map to `Unknown` and only raise if a coding error invokes them.
///
/// # Safety
/// `errors` must be null or a valid `StrObj` pointer.
pub(crate) unsafe fn parse_error_handler(errors: *mut Obj) -> ErrorHandler {
    if errors.is_null() {
        return ErrorHandler::Strict;
    }
    match normalize(errors).as_slice() {
        b"strict" => ErrorHandler::Strict,
        b"ignore" => ErrorHandler::Ignore,
        b"replace" => ErrorHandler::Replace,
        b"backslashreplace" => ErrorHandler::BackslashReplace,
        b"xmlcharrefreplace" => ErrorHandler::XmlCharRefReplace,
        _ => ErrorHandler::Unknown,
    }
}

const REPLACEMENT: char = '\u{FFFD}';

fn push_backslash(out: &mut Vec<u8>, cp: u32) {
    if cp <= 0xFF {
        out.extend_from_slice(format!("\\x{cp:02x}").as_bytes());
    } else if cp <= 0xFFFF {
        out.extend_from_slice(format!("\\u{cp:04x}").as_bytes());
    } else {
        out.extend_from_slice(format!("\\U{cp:08x}").as_bytes());
    }
}

/// Encode a (validated) `&str` to bytes per `enc`/`handler`. Raises on a strict
/// failure, an `Unknown` handler invoked by a failure, or an `Unknown` encoding.
pub(crate) unsafe fn encode(src: &str, enc: Encoding, handler: ErrorHandler) -> Vec<u8> {
    match enc {
        Encoding::Utf8 => src.as_bytes().to_vec(),
        Encoding::Ascii => encode_narrow(src, 0x7F, "ascii", 128, handler),
        Encoding::Latin1 => encode_narrow(src, 0xFF, "latin-1", 256, handler),
        Encoding::Utf16(order) => encode_utf16(src, order),
        Encoding::Utf32(order) => encode_utf32(src, order),
        Encoding::SingleByte(idx) => encode_single_byte(src, idx, handler),
        Encoding::Unknown => {
            raise_exc!(crate::exceptions::ExceptionType::LookupError, "unknown encoding")
        }
    }
}

/// Apply a *non-raising* encode error handler to one unencodable codepoint.
/// Returns `true` if it substituted (`ignore`/`replace`/`backslashreplace`/
/// `xmlcharrefreplace`); `false` for `strict`/`Unknown`, where the caller raises
/// the codec-specific exception.
fn push_encode_sub(out: &mut Vec<u8>, cp: u32, handler: ErrorHandler) -> bool {
    match handler {
        ErrorHandler::Ignore => {}
        ErrorHandler::Replace => out.push(b'?'),
        ErrorHandler::BackslashReplace => push_backslash(out, cp),
        ErrorHandler::XmlCharRefReplace => out.extend_from_slice(format!("&#{cp};").as_bytes()),
        ErrorHandler::Strict | ErrorHandler::Unknown => return false,
    }
    true
}

/// `ascii`/`latin-1`: one byte per codepoint up to `max`, else the handler.
unsafe fn encode_narrow(src: &str, max: u32, codec: &str, range: u32, handler: ErrorHandler) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for ch in src.chars() {
        let cp = ch as u32;
        if cp <= max {
            out.push(cp as u8);
            continue;
        }
        if !push_encode_sub(&mut out, cp, handler) {
            match handler {
                ErrorHandler::Strict => raise_exc!(
                    crate::exceptions::ExceptionType::UnicodeEncodeError,
                    "'{}' codec can't encode character: ordinal not in range({})",
                    codec,
                    range
                ),
                _ => raise_exc!(
                    crate::exceptions::ExceptionType::LookupError,
                    "unknown error handler name"
                ),
            }
        }
    }
    out
}

/// A single-byte charmap codec: reverse-map each codepoint to its byte via a
/// linear scan of the 256-entry table (encode is not a hot path). Unmappable
/// codepoints go through the error handler.
unsafe fn encode_single_byte(src: &str, idx: usize, handler: ErrorHandler) -> Vec<u8> {
    let table = &SINGLE_BYTE_TABLES[idx];
    let codec = SINGLE_BYTE_CODEC_NAMES[idx];
    let mut out = Vec::with_capacity(src.len());
    'chars: for ch in src.chars() {
        let cp = ch as u32;
        if cp <= 0xFFFF {
            let target = cp as u16;
            for (b, &mapped) in table.iter().enumerate() {
                // Skip UNDEF slots so encoding U+FFFF itself never matches one.
                if mapped != UNDEF && mapped == target {
                    out.push(b as u8);
                    continue 'chars;
                }
            }
        }
        if !push_encode_sub(&mut out, cp, handler) {
            match handler {
                ErrorHandler::Strict => raise_exc!(
                    crate::exceptions::ExceptionType::UnicodeEncodeError,
                    "'{}' codec can't encode character: character maps to <undefined>",
                    codec
                ),
                _ => raise_exc!(
                    crate::exceptions::ExceptionType::LookupError,
                    "unknown error handler name"
                ),
            }
        }
    }
    out
}

fn encode_utf16(src: &str, order: ByteOrder) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 2 + 2);
    let be = matches!(order, ByteOrder::Be) || (matches!(order, ByteOrder::Native) && cfg!(target_endian = "big"));
    if matches!(order, ByteOrder::Native) {
        // Byte-order mark U+FEFF in the host's native order.
        push_u16(&mut out, 0xFEFF, be);
    }
    let mut buf = [0u16; 2];
    for ch in src.chars() {
        for &unit in ch.encode_utf16(&mut buf).iter() {
            push_u16(&mut out, unit, be);
        }
    }
    out
}

fn encode_utf32(src: &str, order: ByteOrder) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 4 + 4);
    let be = matches!(order, ByteOrder::Be) || (matches!(order, ByteOrder::Native) && cfg!(target_endian = "big"));
    if matches!(order, ByteOrder::Native) {
        push_u32(&mut out, 0xFEFF, be);
    }
    for ch in src.chars() {
        push_u32(&mut out, ch as u32, be);
    }
    out
}

#[inline]
fn push_u16(out: &mut Vec<u8>, v: u16, be: bool) {
    if be {
        out.extend_from_slice(&v.to_be_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

#[inline]
fn push_u32(out: &mut Vec<u8>, v: u32, be: bool) {
    if be {
        out.extend_from_slice(&v.to_be_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Decode bytes to a `String` per `enc`/`handler`. Raises on a strict failure,
/// an `Unknown` handler invoked by a failure, or an `Unknown` encoding.
pub(crate) unsafe fn decode(slice: &[u8], enc: Encoding, handler: ErrorHandler) -> String {
    match enc {
        Encoding::Latin1 => slice.iter().map(|&b| b as char).collect(),
        Encoding::Ascii => decode_ascii(slice, handler),
        Encoding::Utf8 => decode_utf8(slice, handler),
        Encoding::Utf16(order) => decode_utf16(slice, order, handler),
        Encoding::Utf32(order) => decode_utf32(slice, order, handler),
        Encoding::SingleByte(idx) => decode_single_byte(slice, idx, handler),
        Encoding::Unknown => {
            raise_exc!(crate::exceptions::ExceptionType::LookupError, "unknown encoding")
        }
    }
}

/// A single-byte charmap codec: one byte -> one codepoint via the generated
/// table. An `UNDEF` slot (a byte the codec leaves unassigned) goes through the
/// decode error handler.
unsafe fn decode_single_byte(slice: &[u8], idx: usize, handler: ErrorHandler) -> String {
    let table = &SINGLE_BYTE_TABLES[idx];
    let codec = SINGLE_BYTE_CODEC_NAMES[idx];
    let mut out = String::with_capacity(slice.len());
    for &b in slice {
        let cp = table[b as usize];
        if cp == UNDEF {
            decode_error(&mut out, &[b], handler, codec);
        } else {
            // The generator guarantees every non-UNDEF entry is a BMP scalar value.
            out.push(char::from_u32(cp as u32).unwrap_or(REPLACEMENT));
        }
    }
    out
}

/// Apply a decode error handler to one ill-formed byte span. Returns nothing for
/// `ignore`; pushes the substitution otherwise; `strict`/`Unknown` raise.
unsafe fn decode_error(out: &mut String, bad: &[u8], handler: ErrorHandler, codec: &str) {
    match handler {
        ErrorHandler::Ignore => {}
        ErrorHandler::Replace => out.push(REPLACEMENT),
        ErrorHandler::BackslashReplace => {
            for &b in bad {
                out.push_str(&format!("\\x{b:02x}"));
            }
        }
        ErrorHandler::Strict => raise_exc!(
            crate::exceptions::ExceptionType::UnicodeDecodeError,
            "'{}' codec can't decode byte: invalid data",
            codec
        ),
        // xmlcharrefreplace is encode-only; CPython raises TypeError when a decode
        // error tries to use it. An unknown name raises LookupError.
        ErrorHandler::XmlCharRefReplace => raise_exc!(
            crate::exceptions::ExceptionType::TypeError,
            "don't know how to handle UnicodeDecodeError in error callback"
        ),
        ErrorHandler::Unknown => raise_exc!(
            crate::exceptions::ExceptionType::LookupError,
            "unknown error handler name"
        ),
    }
}

unsafe fn decode_ascii(slice: &[u8], handler: ErrorHandler) -> String {
    let mut out = String::with_capacity(slice.len());
    for &b in slice {
        if b < 0x80 {
            out.push(b as char);
        } else {
            decode_error(&mut out, &[b], handler, "ascii");
        }
    }
    out
}

unsafe fn decode_utf8(slice: &[u8], handler: ErrorHandler) -> String {
    let mut out = String::with_capacity(slice.len());
    let mut rest = slice;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                out.push_str(valid);
                return out;
            }
            Err(e) => {
                let upto = e.valid_up_to();
                // SAFETY: bytes up to `valid_up_to` are valid UTF-8 by definition.
                out.push_str(std::str::from_utf8_unchecked(&rest[..upto]));
                // Rust's `error_len` is the maximal ill-formed subpart — exactly
                // the unit CPython substitutes one U+FFFD per. `None` ⇒ truncated
                // tail; treat the remainder as one span.
                let bad_len = e.error_len().unwrap_or(rest.len() - upto);
                decode_error(&mut out, &rest[upto..upto + bad_len], handler, "utf-8");
                rest = &rest[upto + bad_len..];
            }
        }
    }
}

/// Strip a leading BOM for `Native` order and return the resolved endianness.
fn resolve_bom_u16(slice: &[u8], order: ByteOrder) -> (bool, usize) {
    match order {
        ByteOrder::Le => (false, 0),
        ByteOrder::Be => (true, 0),
        ByteOrder::Native => {
            if slice.len() >= 2 && slice[0] == 0xFF && slice[1] == 0xFE {
                (false, 2)
            } else if slice.len() >= 2 && slice[0] == 0xFE && slice[1] == 0xFF {
                (true, 2)
            } else {
                (cfg!(target_endian = "big"), 0)
            }
        }
    }
}

unsafe fn decode_utf16(slice: &[u8], order: ByteOrder, handler: ErrorHandler) -> String {
    let (be, start) = resolve_bom_u16(slice, order);
    let body = &slice[start..];
    let mut out = String::with_capacity(body.len() / 2);
    let read = |i: usize| -> u16 {
        if be {
            u16::from_be_bytes([body[i], body[i + 1]])
        } else {
            u16::from_le_bytes([body[i], body[i + 1]])
        }
    };
    let mut i = 0;
    while i + 1 < body.len() {
        let unit = read(i);
        if (0xD800..=0xDBFF).contains(&unit) {
            // High surrogate — needs a following low surrogate.
            if i + 3 < body.len() {
                let lo = read(i + 2);
                if (0xDC00..=0xDFFF).contains(&lo) {
                    let cp = 0x10000 + (((unit as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                    out.push(char::from_u32(cp).unwrap_or(REPLACEMENT));
                    i += 4;
                    continue;
                }
            }
            decode_error(&mut out, &body[i..i + 2], handler, "utf-16");
            i += 2;
        } else if (0xDC00..=0xDFFF).contains(&unit) {
            decode_error(&mut out, &body[i..i + 2], handler, "utf-16"); // lone low surrogate
            i += 2;
        } else {
            out.push(char::from_u32(unit as u32).unwrap_or(REPLACEMENT));
            i += 2;
        }
    }
    if i < body.len() {
        decode_error(&mut out, &body[i..], handler, "utf-16"); // dangling odd byte
    }
    out
}

unsafe fn decode_utf32(slice: &[u8], order: ByteOrder, handler: ErrorHandler) -> String {
    // BOM is 4 bytes (FF FE 00 00 / 00 00 FE FF) for utf-32.
    let (be, start) = match order {
        ByteOrder::Le => (false, 0),
        ByteOrder::Be => (true, 0),
        ByteOrder::Native => {
            if slice.len() >= 4 && slice[..4] == [0xFF, 0xFE, 0x00, 0x00] {
                (false, 4)
            } else if slice.len() >= 4 && slice[..4] == [0x00, 0x00, 0xFE, 0xFF] {
                (true, 4)
            } else {
                (cfg!(target_endian = "big"), 0)
            }
        }
    };
    let body = &slice[start..];
    let mut out = String::with_capacity(body.len() / 4);
    let mut i = 0;
    while i + 3 < body.len() {
        let v = if be {
            u32::from_be_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]])
        } else {
            u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]])
        };
        match char::from_u32(v) {
            Some(c) => out.push(c),
            None => decode_error(&mut out, &body[i..i + 4], handler, "utf-32"),
        }
        i += 4;
    }
    if i < body.len() {
        decode_error(&mut out, &body[i..], handler, "utf-32"); // trailing < 4 bytes
    }
    out
}

/// `str.encode(encoding='utf-8', errors='strict')` → bytes (§9, 3-arg ABI).
/// `encoding`/`errors` are null for their defaults.
pub fn rt_str_encode(s: *mut Obj, encoding: *mut Obj, errors: *mut Obj) -> *mut Obj {
    use crate::bytes::rt_make_bytes;
    if s.is_null() {
        return unsafe { rt_make_bytes(std::ptr::null(), 0) };
    }
    unsafe {
        crate::debug_assert_type_tag!(s, crate::object::TypeTagKind::Str, "rt_str_encode");
        let str_obj = s as *mut StrObj;
        let bytes = std::slice::from_raw_parts((*str_obj).data.as_ptr(), (*str_obj).len);
        // A StrObj always holds valid UTF-8.
        let src = std::str::from_utf8_unchecked(bytes);
        let enc = classify_encoding(encoding);
        let handler = parse_error_handler(errors);
        let out = encode(src, enc, handler);
        // `out` lives on the Rust heap (GC-independent), so no rooting is needed
        // across the allocating `rt_make_bytes`.
        rt_make_bytes(out.as_ptr(), out.len())
    }
}
#[export_name = "rt_str_encode"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_str_encode_abi(s: Value, encoding: Value, errors: Value) -> Value {
    Value::from_ptr(rt_str_encode(
        s.unwrap_ptr(),
        encoding.unwrap_ptr(),
        errors.unwrap_ptr(),
    ))
}

/// `bytes.decode(encoding='utf-8', errors='strict')` → str (§9, 3-arg ABI).
pub fn rt_bytes_decode(bytes: *mut Obj, encoding: *mut Obj, errors: *mut Obj) -> *mut Obj {
    use crate::string::rt_make_str;
    if bytes.is_null() {
        return unsafe { rt_make_str(std::ptr::null(), 0) };
    }
    unsafe {
        let bytes_obj = bytes as *mut BytesObj;
        let slice = std::slice::from_raw_parts((*bytes_obj).data.as_ptr(), (*bytes_obj).len);
        let enc = classify_encoding(encoding);
        let handler = parse_error_handler(errors);
        let out = decode(slice, enc, handler);
        // `out` is a fresh Rust-heap String (GC-independent), no rooting needed.
        rt_make_str(out.as_ptr(), out.len())
    }
}
#[export_name = "rt_bytes_decode"]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_bytes_decode_abi(bytes: Value, encoding: Value, errors: Value) -> Value {
    Value::from_ptr(rt_bytes_decode(
        bytes.unwrap_ptr(),
        encoding.unwrap_ptr(),
        errors.unwrap_ptr(),
    ))
}

#[cfg(test)]
mod tests {
    /// Drift guard: the committed single-byte codec tables must still match the
    /// system CPython the differential gate compares against. Skips where python3
    /// is absent. One subprocess, then a byte-compare against the regenerated
    /// output — same pattern as the Unicode char-table guard.
    #[test]
    fn codec_tables_match_system_python() {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/gen_codec_tables.py");
        let Ok(out) = std::process::Command::new("python3").arg(script).output() else {
            return; // no python3 here — nothing to compare against
        };
        if !out.status.success() {
            return; // present but failed (e.g. too old) — don't fail unrelated builds
        }
        let regenerated = String::from_utf8(out.stdout).expect("generator output is utf-8");
        let committed = include_str!("codec_tables.rs");
        assert_eq!(
            regenerated, committed,
            "codec_tables.rs drifted from system CPython — \
             rerun tools/gen_codec_tables.py",
        );
    }
}
