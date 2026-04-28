//! urllib.parse module runtime support
//!
//! Provides URL parsing and encoding functions:
//! - urlparse(url): Parse a URL into components
//! - urlencode(params): Encode a dict as a query string
//! - quote(string, safe): Percent-encode a string
//! - unquote(string): Decode percent-encoded string
//! - urljoin(base, url): Join a base URL with a relative URL
//! - parse_qs(query): Parse a query string into a dict

use crate::dict::{rt_dict_set, rt_make_dict};
use crate::gc::{self, ShadowFrame};
use crate::list::rt_make_list;
use crate::object::{DictObj, ListObj, Obj, ObjHeader, ParseResultObj, TypeTagKind};
use crate::utils::{make_str_from_rust, str_obj_to_rust_string};
use pyaot_core_defs::Value;

/// Characters that are safe by default in URL encoding (RFC 3986 unreserved characters)
const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// URL components parsed from a URL string
struct UrlComponents {
    scheme: String,
    netloc: String,
    path: String,
    params: String,
    query: String,
    fragment: String,
}

/// Parse a URL into its components following RFC 3986
fn parse_url(url: &str) -> UrlComponents {
    let mut scheme = String::new();
    let mut netloc = String::new();
    let mut params = String::new();
    let mut query = String::new();
    let mut fragment = String::new();

    let mut remaining = url;

    // Extract fragment (after #) — use find (first occurrence) per RFC 3986
    if let Some(hash_pos) = remaining.find('#') {
        fragment = remaining[hash_pos + 1..].to_string();
        remaining = &remaining[..hash_pos];
    }

    // Extract query (after ?) — use find (first occurrence) per RFC 3986
    if let Some(question_pos) = remaining.find('?') {
        query = remaining[question_pos + 1..].to_string();
        remaining = &remaining[..question_pos];
    }

    // Extract scheme (before ://)
    if let Some(colon_pos) = remaining.find(':') {
        let potential_scheme = &remaining[..colon_pos];
        // Scheme must start with alpha and contain only alphanumeric, +, -, .
        if !potential_scheme.is_empty()
            && potential_scheme
                .chars()
                .next()
                .unwrap()
                .is_ascii_alphabetic()
            && potential_scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        {
            scheme = potential_scheme.to_lowercase();
            remaining = &remaining[colon_pos + 1..];
        }
    }

    // Extract netloc (after // and before next /)
    if remaining.starts_with("//") {
        remaining = &remaining[2..];
        // Find the end of netloc (first /, ?, or #, or end of string)
        let netloc_end = remaining.find(['/', '?', '#']).unwrap_or(remaining.len());
        netloc = remaining[..netloc_end].to_string();
        remaining = &remaining[netloc_end..];
    }

    // Extract params (between path and query, using ;)
    // Note: params are rarely used in modern URLs
    if let Some(semicolon_pos) = remaining.rfind(';') {
        params = remaining[semicolon_pos + 1..].to_string();
        remaining = &remaining[..semicolon_pos];
    }

    // Everything else is the path
    let path = remaining.to_string();

    UrlComponents {
        scheme,
        netloc,
        path,
        params,
        query,
        fragment,
    }
}

/// Create a ParseResultObj from URL components
unsafe fn create_parse_result(components: &UrlComponents) -> *mut Obj {
    let size = std::mem::size_of::<ParseResultObj>();
    let ptr = gc::gc_alloc(size, TypeTagKind::ParseResult as u8) as *mut ParseResultObj;

    (*ptr).header = ObjHeader {
        type_tag: TypeTagKind::ParseResult,
        marked: false,
        size,
    };

    // Root ptr so the 6 subsequent string allocations don't collect it.
    let mut roots: [*mut Obj; 1] = [ptr as *mut Obj];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 1,
        roots: roots.as_mut_ptr(),
    };
    gc::gc_push(&mut frame);

    let pr = roots[0] as *mut ParseResultObj;
    (*pr).scheme = make_str_from_rust(&components.scheme);
    (*pr).netloc = make_str_from_rust(&components.netloc);
    (*pr).path = make_str_from_rust(&components.path);
    (*pr).params = make_str_from_rust(&components.params);
    (*pr).query = make_str_from_rust(&components.query);
    (*pr).fragment = make_str_from_rust(&components.fragment);

    gc::gc_pop();
    roots[0]
}

/// Reassemble URL components into a URL string
fn assemble_url(components: &UrlComponents) -> String {
    let mut result = String::new();

    if !components.scheme.is_empty() {
        result.push_str(&components.scheme);
        result.push(':');
    }

    if !components.netloc.is_empty() || components.scheme == "file" {
        result.push_str("//");
        result.push_str(&components.netloc);
    }

    result.push_str(&components.path);

    if !components.params.is_empty() {
        result.push(';');
        result.push_str(&components.params);
    }

    if !components.query.is_empty() {
        result.push('?');
        result.push_str(&components.query);
    }

    if !components.fragment.is_empty() {
        result.push('#');
        result.push_str(&components.fragment);
    }

    result
}

/// urllib.parse.urlparse(url) - Parse a URL into components
/// Returns a ParseResult with scheme, netloc, path, params, query, fragment
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_urlparse(url: *mut Obj) -> *mut Obj {
    if url.is_null() {
        let empty = UrlComponents {
            scheme: String::new(),
            netloc: String::new(),
            path: String::new(),
            params: String::new(),
            query: String::new(),
            fragment: String::new(),
        };
        return unsafe { create_parse_result(&empty) };
    }

    unsafe {
        let url_str = str_obj_to_rust_string(url);
        let components = parse_url(&url_str);
        create_parse_result(&components)
    }
}
#[export_name = "rt_urlparse"]
pub extern "C" fn rt_urlparse_abi(url: Value) -> Value {
    Value::from_ptr(rt_urlparse(url.unwrap_ptr()))
}


/// Percent-encode a single byte
fn percent_encode_byte(byte: u8) -> String {
    format!("%{:02X}", byte)
}

/// Percent-encode a string, keeping safe characters unencoded
fn percent_encode(s: &str, safe: &str) -> String {
    let safe_bytes: Vec<u8> = safe.bytes().collect();
    let mut result = String::new();

    for byte in s.bytes() {
        if UNRESERVED.contains(&byte) || safe_bytes.contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&percent_encode_byte(byte));
        }
    }

    result
}

/// Percent-encode a string using quote_plus semantics (spaces → '+')
/// Used by urlencode() to match CPython behavior
fn percent_encode_plus(s: &str) -> String {
    let mut result = String::new();

    for byte in s.bytes() {
        if byte == b' ' {
            result.push('+');
        } else if UNRESERVED.contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&percent_encode_byte(byte));
        }
    }

    result
}

/// Decode percent-encoded string
/// If `plus_as_space` is true, '+' is decoded as a space (for query strings / unquote_plus).
/// If false, '+' is kept as-is (for unquote which does not decode '+').
fn percent_decode(s: &str, plus_as_space: bool) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            // Try to parse hex digits
            let hex_str = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte_val) = u8::from_str_radix(hex_str, 16) {
                result.push(byte_val);
                i += 3;
                continue;
            }
        }
        // Handle + as space only for unquote_plus
        if plus_as_space && bytes[i] == b'+' {
            result.push(b' ');
        } else {
            result.push(bytes[i]);
        }
        i += 1;
    }

    String::from_utf8_lossy(&result).into_owned()
}

/// urllib.parse.quote(string, safe='') - Percent-encode a string
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_quote(string: *mut Obj, safe: *mut Obj) -> *mut Obj {
    if string.is_null() {
        return unsafe { make_str_from_rust("") };
    }

    unsafe {
        let s = str_obj_to_rust_string(string);
        let safe_str = if safe.is_null() {
            String::new()
        } else {
            str_obj_to_rust_string(safe)
        };

        let encoded = percent_encode(&s, &safe_str);
        make_str_from_rust(&encoded)
    }
}
#[export_name = "rt_quote"]
pub extern "C" fn rt_quote_abi(string: Value, safe: Value) -> Value {
    Value::from_ptr(rt_quote(string.unwrap_ptr(), safe.unwrap_ptr()))
}


/// urllib.parse.unquote(string) - Decode percent-encoded string
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_unquote(string: *mut Obj) -> *mut Obj {
    if string.is_null() {
        return unsafe { make_str_from_rust("") };
    }

    unsafe {
        let s = str_obj_to_rust_string(string);
        // rt_unquote does NOT decode '+' as space (only rt_unquote_plus does)
        let decoded = percent_decode(&s, false);
        make_str_from_rust(&decoded)
    }
}
#[export_name = "rt_unquote"]
pub extern "C" fn rt_unquote_abi(string: Value) -> Value {
    Value::from_ptr(rt_unquote(string.unwrap_ptr()))
}


/// urllib.parse.urlencode(params) - Encode a dict as a query string
/// Example: {"key": "value", "a": "b"} -> "key=value&a=b"
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_urlencode(params: *mut Obj) -> *mut Obj {
    if params.is_null() {
        return unsafe { make_str_from_rust("") };
    }

    unsafe {
        if (*params).header.type_tag != TypeTagKind::Dict {
            return make_str_from_rust("");
        }

        let dict = params as *const DictObj;
        let entries_len = (*dict).entries_len;
        let entries = (*dict).entries;

        let mut pairs: Vec<String> = Vec::new();

        for i in 0..entries_len {
            let entry = entries.add(i);
            let key = (*entry).key;

            if key.0 != 0 {
                let key_ptr = key.0 as *mut Obj;
                let key_str = str_obj_to_rust_string(key_ptr);
                let value = (*entry).value;
                let value_ptr = value.0 as *mut Obj;
                if value_ptr.is_null()
                    || (*value_ptr).header.type_tag != crate::object::TypeTagKind::Str
                {
                    raise_exc!(
                        pyaot_core_defs::BuiltinExceptionKind::TypeError,
                        "urlencode values must be strings"
                    );
                }
                let value_str = str_obj_to_rust_string(value_ptr);

                let encoded_key = percent_encode_plus(&key_str);
                let encoded_value = percent_encode_plus(&value_str);

                pairs.push(format!("{}={}", encoded_key, encoded_value));
            }
        }

        let result = pairs.join("&");
        make_str_from_rust(&result)
    }
}
#[export_name = "rt_urlencode"]
pub extern "C" fn rt_urlencode_abi(params: Value) -> Value {
    Value::from_ptr(rt_urlencode(params.unwrap_ptr()))
}


/// urllib.parse.urljoin(base, url) - Join a base URL with a relative URL
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_urljoin(base: *mut Obj, url: *mut Obj) -> *mut Obj {
    if base.is_null() && url.is_null() {
        return unsafe { make_str_from_rust("") };
    }

    unsafe {
        let base_str = if base.is_null() {
            String::new()
        } else {
            str_obj_to_rust_string(base)
        };

        let url_str = if url.is_null() {
            String::new()
        } else {
            str_obj_to_rust_string(url)
        };

        // If url is empty, return base
        if url_str.is_empty() {
            return make_str_from_rust(&base_str);
        }

        let url_components = parse_url(&url_str);

        // If url has a scheme, it's an absolute URL - return it as-is
        if !url_components.scheme.is_empty() {
            return make_str_from_rust(&url_str);
        }

        let base_components = parse_url(&base_str);

        // Build the result using base's scheme and possibly netloc
        let mut result = UrlComponents {
            scheme: base_components.scheme.clone(),
            netloc: String::new(),
            path: String::new(),
            params: url_components.params.clone(),
            query: url_components.query.clone(),
            fragment: url_components.fragment.clone(),
        };

        // If url has netloc, use url's netloc and path
        if !url_components.netloc.is_empty() {
            result.netloc = url_components.netloc.clone();
            result.path = normalize_path(&url_components.path);
        } else {
            result.netloc = base_components.netloc.clone();

            // If url path is empty, use base path
            if url_components.path.is_empty() {
                result.path = base_components.path.clone();
                // If url has no query, use base query
                if url_components.query.is_empty() {
                    result.query = base_components.query.clone();
                }
            } else if url_components.path.starts_with('/') {
                // Absolute path
                result.path = normalize_path(&url_components.path);
            } else {
                // Relative path - merge with base
                let merged = merge_paths(&base_components.path, &url_components.path);
                result.path = normalize_path(&merged);
            }
        }

        let assembled = assemble_url(&result);
        make_str_from_rust(&assembled)
    }
}
#[export_name = "rt_urljoin"]
pub extern "C" fn rt_urljoin_abi(base: Value, url: Value) -> Value {
    Value::from_ptr(rt_urljoin(base.unwrap_ptr(), url.unwrap_ptr()))
}


/// Merge a base path with a relative path
fn merge_paths(base: &str, relative: &str) -> String {
    if base.is_empty() {
        format!("/{}", relative)
    } else {
        // Remove everything after the last / in base
        match base.rfind('/') {
            Some(pos) => format!("{}/{}", &base[..pos], relative),
            None => relative.to_string(),
        }
    }
}

/// Normalize a path by resolving . and .. segments
fn normalize_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();

    for segment in path.split('/') {
        match segment {
            "" | "." => {
                // Skip empty and current directory
                if segments.is_empty() && path.starts_with('/') {
                    segments.push("");
                }
            }
            ".." => {
                // Go up one directory (but don't go above root)
                if !segments.is_empty() && segments.last() != Some(&"") {
                    segments.pop();
                }
            }
            _ => {
                segments.push(segment);
            }
        }
    }

    if segments.is_empty() || (segments.len() == 1 && segments[0].is_empty()) {
        "/".to_string()
    } else {
        segments.join("/")
    }
}

/// urllib.parse.parse_qs(query) - Parse a query string into a dict
/// Returns dict[str, list[str]] since keys can have multiple values
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_qs(query: *mut Obj) -> *mut Obj {
    if query.is_null() {
        return rt_make_dict(8);
    }

    unsafe {
        let query_str = str_obj_to_rust_string(query);

        // Create result dict
        let dict = rt_make_dict(8);

        // Note: CPython does NOT strip leading '?' — it becomes part of the first key.
        // Users should strip it themselves or pass only the query portion.
        let query_str = &query_str;

        if query_str.is_empty() {
            return dict;
        }

        // Root dict and scratch slots for key_obj / value_obj so that
        // every intermediate allocation below cannot collect live objects.
        // roots[0] = dict, roots[1] = key_obj, roots[2] = value_obj
        let mut roots: [*mut Obj; 3] = [dict, std::ptr::null_mut(), std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 3,
            roots: roots.as_mut_ptr(),
        };
        gc::gc_push(&mut frame);

        // Parse each key=value pair
        for pair in query_str.split('&') {
            if pair.is_empty() {
                continue;
            }

            let (key, value) = match pair.find('=') {
                Some(pos) => (&pair[..pos], &pair[pos + 1..]),
                None => (pair, ""),
            };

            // Decode key and value
            let decoded_key = percent_decode(key, true);
            let decoded_value = percent_decode(value, true);

            // Allocate key and value strings; keep them rooted across further allocs.
            let key_obj = make_str_from_rust(&decoded_key);
            roots[1] = key_obj;
            let value_obj = make_str_from_rust(&decoded_value);
            roots[2] = value_obj;

            // Check if key already exists in dict
            let existing = get_dict_value(roots[0], roots[1]);

            if existing.is_null() {
                // Create new list with this value — dict/key/value all stay rooted.
                let list = rt_make_list(1);
                let list_obj = list as *mut ListObj;
                (*list_obj).len = 1;
                *(*list_obj).data = pyaot_core_defs::Value::from_ptr(roots[2]);

                rt_dict_set(roots[0], roots[1], list);
            } else {
                // Append to existing list
                crate::list::rt_list_push(existing, roots[2]);
            }

            roots[1] = std::ptr::null_mut();
            roots[2] = std::ptr::null_mut();
        }

        gc::gc_pop();
        roots[0]
    }
}
#[export_name = "rt_parse_qs"]
pub extern "C" fn rt_parse_qs_abi(query: Value) -> Value {
    Value::from_ptr(rt_parse_qs(query.unwrap_ptr()))
}


/// Helper to get value from dict
unsafe fn get_dict_value(dict: *mut Obj, key: *mut Obj) -> *mut Obj {
    crate::dict::rt_dict_get(dict, key)
}

// =============================================================================
// ParseResult field getters
// =============================================================================

/// Get scheme field from ParseResult
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_get_scheme(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("");
        }
        let pr = obj as *const ParseResultObj;
        (*pr).scheme
    }
}
#[export_name = "rt_parse_result_get_scheme"]
pub extern "C" fn rt_parse_result_get_scheme_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_get_scheme(obj.unwrap_ptr()))
}


/// Get netloc field from ParseResult
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_get_netloc(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("");
        }
        let pr = obj as *const ParseResultObj;
        (*pr).netloc
    }
}
#[export_name = "rt_parse_result_get_netloc"]
pub extern "C" fn rt_parse_result_get_netloc_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_get_netloc(obj.unwrap_ptr()))
}


/// Get path field from ParseResult
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_get_path(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("");
        }
        let pr = obj as *const ParseResultObj;
        (*pr).path
    }
}
#[export_name = "rt_parse_result_get_path"]
pub extern "C" fn rt_parse_result_get_path_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_get_path(obj.unwrap_ptr()))
}


/// Get params field from ParseResult
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_get_params(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("");
        }
        let pr = obj as *const ParseResultObj;
        (*pr).params
    }
}
#[export_name = "rt_parse_result_get_params"]
pub extern "C" fn rt_parse_result_get_params_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_get_params(obj.unwrap_ptr()))
}


/// Get query field from ParseResult
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_get_query(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("");
        }
        let pr = obj as *const ParseResultObj;
        (*pr).query
    }
}
#[export_name = "rt_parse_result_get_query"]
pub extern "C" fn rt_parse_result_get_query_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_get_query(obj.unwrap_ptr()))
}


/// Get fragment field from ParseResult
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_get_fragment(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("");
        }
        let pr = obj as *const ParseResultObj;
        (*pr).fragment
    }
}
#[export_name = "rt_parse_result_get_fragment"]
pub extern "C" fn rt_parse_result_get_fragment_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_get_fragment(obj.unwrap_ptr()))
}


// =============================================================================
// ParseResult methods
// =============================================================================

/// ParseResult.geturl() - Reassemble the URL from components
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_geturl(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }

    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("");
        }

        let pr = obj as *const ParseResultObj;

        let components = UrlComponents {
            scheme: str_obj_to_rust_string((*pr).scheme),
            netloc: str_obj_to_rust_string((*pr).netloc),
            path: str_obj_to_rust_string((*pr).path),
            params: str_obj_to_rust_string((*pr).params),
            query: str_obj_to_rust_string((*pr).query),
            fragment: str_obj_to_rust_string((*pr).fragment),
        };

        let url = assemble_url(&components);
        make_str_from_rust(&url)
    }
}
#[export_name = "rt_parse_result_geturl"]
pub extern "C" fn rt_parse_result_geturl_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_geturl(obj.unwrap_ptr()))
}


/// repr() for ParseResult
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_parse_result_repr(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe {
            make_str_from_rust(
                "ParseResult(scheme='', netloc='', path='', params='', query='', fragment='')",
            )
        };
    }

    unsafe {
        if (*obj).header.type_tag != TypeTagKind::ParseResult {
            return make_str_from_rust("<invalid ParseResult>");
        }

        let pr = obj as *const ParseResultObj;

        let scheme = str_obj_to_rust_string((*pr).scheme);
        let netloc = str_obj_to_rust_string((*pr).netloc);
        let path = str_obj_to_rust_string((*pr).path);
        let params = str_obj_to_rust_string((*pr).params);
        let query = str_obj_to_rust_string((*pr).query);
        let fragment = str_obj_to_rust_string((*pr).fragment);

        let repr = format!(
            "ParseResult(scheme='{}', netloc='{}', path='{}', params='{}', query='{}', fragment='{}')",
            scheme, netloc, path, params, query, fragment
        );
        make_str_from_rust(&repr)
    }
}
#[export_name = "rt_parse_result_repr"]
pub extern "C" fn rt_parse_result_repr_abi(obj: Value) -> Value {
    Value::from_ptr(rt_parse_result_repr(obj.unwrap_ptr()))
}

