//! JSON module runtime support
//!
//! Provides:
//! - json.dumps(obj) -> str (serialize Python object to JSON string)
//! - json.loads(s) -> obj (deserialize JSON string to Python object)
//! - json.dump(obj, fp) -> None (serialize Python object to JSON file)
//! - json.load(fp) -> obj (deserialize JSON object from JSON file)
//!
//! Both directions are implemented directly on runtime objects (no
//! intermediate tree), reproducing CPython's default `json` behaviour:
//! - encoding: `", "` / `": "` separators, `ensure_ascii`, dict insertion
//!   order, arbitrary-precision ints, `float.__repr__` floats, `NaN` /
//!   `Infinity` / `-Infinity`, int / float / bool / None keys converted to
//!   strings (other keys raise `TypeError`), `TypeError: Object of type X is
//!   not JSON serializable`;
//! - decoding: object key order preserved, big ints exact, `NaN` /
//!   `Infinity` / `-Infinity` accepted, CPython's `JSONDecodeError` messages
//!   (`Expecting value: line 1 column 1 (char 0)`; raised as `ValueError`, its
//!   base class).

use crate::object::{BigIntObj, DictObj, FloatObj, ListObj, Obj, TupleObj, TypeTagKind};
use crate::utils::make_str_from_rust;
use pyaot_core_defs::Value as RtValue;

// ==================== Encoder ====================

unsafe fn type_error(msg: String) -> ! {
    crate::raise_exc!(crate::exceptions::ExceptionType::TypeError, "{}", msg)
}

unsafe fn value_error(msg: String) -> ! {
    crate::raise_exc!(crate::exceptions::ExceptionType::ValueError, "{}", msg)
}

/// Python type name of a value for error messages.
unsafe fn py_type_name(obj: *mut Obj) -> String {
    let v = RtValue(obj as u64);
    if obj.is_null() || v.is_none() {
        return "NoneType".into();
    }
    if v.is_int() {
        return "int".into();
    }
    if v.is_bool() {
        return "bool".into();
    }
    if (*obj).type_tag() == TypeTagKind::Instance {
        let cid = (*(obj as *const crate::object::InstanceObj)).class_id;
        return crate::ops::instance_class_name(cid);
    }
    crate::ops::comparison_type_name((*obj).type_tag()).to_string()
}

/// `json.encoder.py_encode_basestring_ascii`: quote + escape, ASCII only.
fn encode_str(out: &mut String, s: &str) {
    use std::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{:04x}", unit);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `float.__repr__` with JSON's special values.
fn encode_float(out: &mut String, f: f64) {
    if f.is_nan() {
        out.push_str("NaN");
    } else if f.is_infinite() {
        out.push_str(if f > 0.0 { "Infinity" } else { "-Infinity" });
    } else {
        out.push_str(&pyaot_format_shared::float_repr(f));
    }
}

/// Is `obj` one of the dict-layout mappings (dict / defaultdict / Counter /
/// OrderedDict)?
unsafe fn is_dict_like(obj: *mut Obj) -> bool {
    matches!(
        (*obj).type_tag(),
        TypeTagKind::Dict | TypeTagKind::DefaultDict | TypeTagKind::Counter
    )
}

/// The JSON object key for a dict key (CPython: str as is; int, float, bool
/// and None converted; anything else is a `TypeError`).
unsafe fn encode_key(out: &mut String, key: *mut Obj) {
    let v = RtValue(key as u64);
    if key.is_null() || v.is_none() {
        out.push_str("\"null\"");
        return;
    }
    if v.is_bool() {
        out.push_str(if v.unwrap_bool() {
            "\"true\""
        } else {
            "\"false\""
        });
        return;
    }
    if v.is_int() {
        out.push('"');
        out.push_str(&v.unwrap_int().to_string());
        out.push('"');
        return;
    }
    match (*key).type_tag() {
        TypeTagKind::Str => encode_str(out, &crate::utils::extract_str_unchecked(key)),
        TypeTagKind::BigInt => {
            out.push('"');
            out.push_str(&(*(key as *mut BigIntObj)).value.to_string());
            out.push('"');
        }
        TypeTagKind::Float => {
            out.push('"');
            encode_float(out, (*(key as *mut FloatObj)).value);
            out.push('"');
        }
        _ => type_error(format!(
            "keys must be str, int, float, bool or None, not {}",
            py_type_name(key)
        )),
    }
}

unsafe fn encode(out: &mut String, obj: *mut Obj, depth: u32) {
    if depth > MAX_JSON_DEPTH {
        crate::raise_exc!(
            crate::exceptions::ExceptionType::RecursionError,
            "maximum recursion depth exceeded while encoding a JSON object"
        );
    }
    let v = RtValue(obj as u64);
    if obj.is_null() || v.is_none() {
        out.push_str("null");
        return;
    }
    if v.is_bool() {
        out.push_str(if v.unwrap_bool() { "true" } else { "false" });
        return;
    }
    if v.is_int() {
        out.push_str(&v.unwrap_int().to_string());
        return;
    }
    match (*obj).type_tag() {
        TypeTagKind::None => out.push_str("null"),
        TypeTagKind::Float => encode_float(out, (*(obj as *mut FloatObj)).value),
        TypeTagKind::BigInt => out.push_str(&(*(obj as *mut BigIntObj)).value.to_string()),
        TypeTagKind::Str => encode_str(out, &crate::utils::extract_str_unchecked(obj)),
        TypeTagKind::List => {
            let list = obj as *mut ListObj;
            out.push('[');
            for i in 0..(*list).len {
                if i > 0 {
                    out.push_str(", ");
                }
                encode(out, (*(*list).data.add(i)).0 as *mut Obj, depth + 1);
            }
            out.push(']');
        }
        TypeTagKind::Tuple => {
            let t = obj as *const TupleObj;
            out.push('[');
            for i in 0..(*t).len {
                if i > 0 {
                    out.push_str(", ");
                }
                encode(out, (*(*t).data.as_ptr().add(i)).0 as *mut Obj, depth + 1);
            }
            out.push(']');
        }
        _ if is_dict_like(obj) => {
            let d = obj as *const DictObj;
            out.push('{');
            let mut first = true;
            for i in 0..(*d).entries_len {
                let entry = (*d).entries.add(i);
                if (*entry).key.0 == 0 {
                    continue;
                }
                if !first {
                    out.push_str(", ");
                }
                first = false;
                encode_key(out, (*entry).key.0 as *mut Obj);
                out.push_str(": ");
                encode(out, (*entry).value.0 as *mut Obj, depth + 1);
            }
            out.push('}');
        }
        _ => type_error(format!(
            "Object of type {} is not JSON serializable",
            py_type_name(obj)
        )),
    }
}

/// Serialize to a Rust string (no GC allocation happens during encoding).
unsafe fn dumps_string(obj: *mut Obj) -> String {
    let mut out = String::new();
    encode(&mut out, obj, 0);
    out
}

// ==================== Decoder ====================

/// Maximum JSON nesting depth.
const MAX_JSON_DEPTH: u32 = 200;

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    /// Raise `JSONDecodeError`'s message (`msg: line L column C (char P)`).
    unsafe fn fail(&self, msg: &str, pos: usize) -> ! {
        let before = &self.s[..pos.min(self.s.len())];
        let line = before.matches('\n').count() + 1;
        let col = match before.rfind('\n') {
            Some(nl) => before[nl + 1..].chars().count() + 1,
            None => before.chars().count() + 1,
        };
        let char_pos = before.chars().count();
        value_error(format!(
            "{}: line {} column {} (char {})",
            msg, line, col, char_pos
        ))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.b.len() && matches!(self.b[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    unsafe fn parse_value(&mut self, depth: u32) -> *mut Obj {
        if depth >= MAX_JSON_DEPTH {
            crate::raise_exc!(
                crate::exceptions::ExceptionType::RecursionError,
                "maximum recursion depth exceeded while decoding a JSON document"
            );
        }
        self.skip_ws();
        let start = self.pos;
        let Some(&c) = self.b.get(self.pos) else {
            self.fail("Expecting value", start)
        };
        match c {
            b'{' => self.parse_object(depth),
            b'[' => self.parse_array(depth),
            b'"' => {
                let s = self.parse_string();
                make_str_from_rust(&s)
            }
            b'n' if self.s[start..].starts_with("null") => {
                self.pos += 4;
                crate::object::none_obj()
            }
            b't' if self.s[start..].starts_with("true") => {
                self.pos += 4;
                RtValue::from_bool(true).0 as *mut Obj
            }
            b'f' if self.s[start..].starts_with("false") => {
                self.pos += 5;
                RtValue::from_bool(false).0 as *mut Obj
            }
            b'N' if self.s[start..].starts_with("NaN") => {
                self.pos += 3;
                crate::boxing::rt_box_float(f64::NAN)
            }
            b'I' if self.s[start..].starts_with("Infinity") => {
                self.pos += 8;
                crate::boxing::rt_box_float(f64::INFINITY)
            }
            b'-' if self.s[start..].starts_with("-Infinity") => {
                self.pos += 9;
                crate::boxing::rt_box_float(f64::NEG_INFINITY)
            }
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => self.fail("Expecting value", start),
        }
    }

    unsafe fn parse_number(&mut self) -> *mut Obj {
        let start = self.pos;
        let b = self.b;
        let mut i = self.pos;
        if b[i] == b'-' {
            i += 1;
        }
        // int part: "0" or [1-9][0-9]*
        match b.get(i) {
            Some(b'0') => i += 1,
            Some(b'1'..=b'9') => {
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            _ => self.fail("Expecting value", start),
        }
        let mut is_float = false;
        if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
            is_float = true;
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_digit() {
                is_float = true;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        self.pos = i;
        let text = &self.s[start..i];
        if is_float {
            crate::boxing::rt_box_float(text.parse::<f64>().unwrap_or(f64::NAN))
        } else {
            let big: num_bigint::BigInt = text.parse().expect("validated integer literal");
            crate::bigint::make_int_value(big)
        }
    }

    /// Parse a string literal (cursor on the opening quote).
    unsafe fn parse_string(&mut self) -> String {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.b.get(self.pos) else {
                self.fail("Unterminated string starting at", start)
            };
            match c {
                b'"' => {
                    self.pos += 1;
                    return out;
                }
                b'\\' => {
                    let esc_pos = self.pos;
                    let Some(&e) = self.b.get(self.pos + 1) else {
                        self.fail("Unterminated string starting at", start)
                    };
                    self.pos += 2;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut cp = self.hex4(esc_pos);
                            if (0xD800..0xDC00).contains(&cp)
                                && self.s[self.pos..].starts_with("\\u")
                            {
                                let save = self.pos;
                                self.pos += 2;
                                let lo = self.hex4(save);
                                if (0xDC00..0xE000).contains(&lo) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                } else {
                                    self.pos = save;
                                }
                            }
                            // A lone surrogate cannot be represented in a
                            // UTF-8 str: substitute U+FFFD.
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => self.fail("Invalid \\escape", esc_pos),
                    }
                }
                0..=0x1f => self.fail("Invalid control character at", self.pos),
                _ => {
                    // Copy one UTF-8 character.
                    let ch = self.s[self.pos..].chars().next().expect("in bounds");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    unsafe fn hex4(&mut self, esc_pos: usize) -> u32 {
        let digits = self.s.get(self.pos..self.pos + 4).unwrap_or("");
        match u32::from_str_radix(digits, 16) {
            Ok(v) if digits.len() == 4 && digits.bytes().all(|d| d.is_ascii_hexdigit()) => {
                self.pos += 4;
                v
            }
            _ => self.fail("Invalid \\uXXXX escape", esc_pos),
        }
    }

    unsafe fn parse_array(&mut self, depth: u32) -> *mut Obj {
        self.pos += 1; // '['
        let list = crate::list::rt_make_list(0);
        // Root the list (and the current element) across nested allocations.
        let mut roots: [*mut Obj; 2] = [list, std::ptr::null_mut()];
        let mut frame = crate::gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        crate::gc::gc_push(&mut frame);
        self.skip_ws();
        if self.b.get(self.pos) == Some(&b']') {
            self.pos += 1;
        } else {
            loop {
                roots[1] = self.parse_value(depth + 1);
                crate::list::rt_list_push(roots[0], roots[1]);
                self.skip_ws();
                match self.b.get(self.pos) {
                    Some(b',') => self.pos += 1,
                    Some(b']') => {
                        self.pos += 1;
                        break;
                    }
                    _ => {
                        crate::gc::gc_pop();
                        self.fail("Expecting ',' delimiter", self.pos)
                    }
                }
            }
        }
        crate::gc::gc_pop();
        roots[0]
    }

    unsafe fn parse_object(&mut self, depth: u32) -> *mut Obj {
        self.pos += 1; // '{'
        let dict = crate::dict::rt_make_dict(8);
        let mut roots: [*mut Obj; 2] = [dict, std::ptr::null_mut()];
        let mut frame = crate::gc::ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 2,
            roots: roots.as_mut_ptr(),
        };
        crate::gc::gc_push(&mut frame);
        self.skip_ws();
        if self.b.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            crate::gc::gc_pop();
            return roots[0];
        }
        loop {
            self.skip_ws();
            if self.b.get(self.pos) != Some(&b'"') {
                crate::gc::gc_pop();
                self.fail(
                    "Expecting property name enclosed in double quotes",
                    self.pos,
                )
            }
            let key = self.parse_string();
            self.skip_ws();
            if self.b.get(self.pos) != Some(&b':') {
                crate::gc::gc_pop();
                self.fail("Expecting ':' delimiter", self.pos)
            }
            self.pos += 1;
            roots[1] = make_str_from_rust(&key);
            let value = self.parse_value(depth + 1);
            // `rt_dict_set` may grow the table (allocation); `value` is only
            // reachable from here, so root it in place of the (stored) key
            // after the store would be too late — keep the key rooted and
            // store immediately: rt_dict_set roots its own arguments.
            crate::dict::rt_dict_set(roots[0], roots[1], value);
            roots[1] = std::ptr::null_mut();
            self.skip_ws();
            match self.b.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    crate::gc::gc_pop();
                    self.fail("Expecting ',' delimiter", self.pos)
                }
            }
        }
        crate::gc::gc_pop();
        roots[0]
    }
}

unsafe fn loads_str(text: &str) -> *mut Obj {
    let mut p = Parser {
        s: text,
        b: text.as_bytes(),
        pos: 0,
    };
    let v = p.parse_value(0);
    // Root the result while checking for trailing data (no allocation until
    // the error path, which unwinds anyway).
    p.skip_ws();
    if p.pos != p.b.len() {
        p.fail("Extra data", p.pos);
    }
    v
}

// ==================== FFI ====================

/// Serialize a Python object to a JSON string
///
/// # Safety
/// `obj` must be a valid pointer to a Python runtime object (or null for None).
#[no_mangle]
pub unsafe extern "C" fn rt_json_dumps(obj: *mut Obj) -> *mut Obj {
    let s = dumps_string(obj);
    make_str_from_rust(&s)
}

/// Deserialize a JSON string to a Python object
///
/// # Safety
/// `s` must be a valid pointer to a StrObj containing valid JSON.
#[no_mangle]
pub unsafe extern "C" fn rt_json_loads(s: *mut Obj) -> *mut Obj {
    if s.is_null() || RtValue(s as u64).is_none() {
        type_error("the JSON object must be str, bytes or bytearray, not NoneType".into());
    }
    if !RtValue(s as u64).is_ptr() || (*s).type_tag() != TypeTagKind::Str {
        type_error(format!(
            "the JSON object must be str, bytes or bytearray, not {}",
            py_type_name(s)
        ));
    }
    let json_str = crate::utils::extract_str_unchecked(s);
    loads_str(&json_str)
}

/// Check `fp` is an open file object.
unsafe fn open_file(fp: *mut Obj) -> *mut crate::object::FileObj {
    use crate::object::FileObj;
    if fp.is_null() || !RtValue(fp as u64).is_ptr() || (*fp).type_tag() != TypeTagKind::File {
        crate::utils::raise_value_error("TypeError: expected file object");
    }
    let file_obj = fp as *mut FileObj;
    if (*file_obj).closed || (*file_obj).handle.is_null() {
        value_error("I/O operation on closed file.".into());
    }
    file_obj
}

/// Serialize a Python object to a JSON file
///
/// # Safety
/// `obj` must be a valid pointer to a Python runtime object.
/// `fp` must be a valid pointer to a FileObj opened for writing.
#[no_mangle]
pub unsafe extern "C" fn rt_json_dump(obj: *mut Obj, fp: *mut Obj) {
    use std::io::Write;
    let file_obj = open_file(fp);
    let s = dumps_string(obj);
    let handle = &mut *(*file_obj).handle;
    if let Err(e) = handle.write_all(s.as_bytes()) {
        crate::raise_exc!(crate::exceptions::ExceptionType::OSError, "{}", e);
    }
}

/// Deserialize a Python object from a JSON file
///
/// # Safety
/// `fp` must be a valid pointer to a FileObj opened for reading.
#[no_mangle]
pub unsafe extern "C" fn rt_json_load(fp: *mut Obj) -> *mut Obj {
    use std::io::Read;
    let file_obj = open_file(fp);
    let handle = &mut *(*file_obj).handle;
    let mut content = String::new();
    if let Err(e) = handle.read_to_string(&mut content) {
        crate::raise_exc!(crate::exceptions::ExceptionType::OSError, "{}", e);
    }
    loads_str(&content)
}
