//! HTTP request runtime support shared by `urllib.request` and the
//! `requests` package.
//!
//! Public extern "C" entry points:
//! - `rt_urlopen(url, data, timeout)` — the urllib stdlib shim; GET if `data`
//!   is null, POST otherwise.
//! - `rt_http_request_raw(method_ptr, method_len, url, params, data, headers,
//!   timeout)` — generic HTTP request with any method, optional query-string
//!   params, body bytes, and custom headers. Used by the `pyaot-pkg-requests`
//!   crate to back `requests.get/post/put/delete`.
//!
//! Both wrap a private `do_http_request` helper so the single ureq pipeline
//! (TLS setup, redirect handling, body size guard, response construction)
//! stays in one place.

use crate::bytes::rt_make_bytes;
use crate::dict::{rt_dict_set, rt_make_dict};
use crate::gc;
use crate::object::{BytesObj, DictObj, HttpResponseObj, Obj, ObjHeader, RequestObj, TypeTagKind};
use crate::utils::{is_none_or_null, make_str_from_rust, raise_io_error, str_obj_to_rust_string};
use pyaot_core_defs::Value;
use std::time::Duration;

/// Iterate `str -> str` entries of a DictObj. Entries with non-`Str` keys or
/// values are silently skipped (the stdlib type system only allows
/// `dict[str, str]` at call sites, so this is defensive).
///
/// # Safety
/// `dict` must be a valid `*mut DictObj` or null.
unsafe fn for_each_str_entry<F: FnMut(String, String)>(dict: *mut Obj, mut f: F) {
    if is_none_or_null(dict) {
        return;
    }
    if (*dict).header.type_tag != TypeTagKind::Dict {
        return;
    }
    let d = dict as *mut DictObj;
    let entries_len = (*d).entries_len;
    let entries = (*d).entries;
    for i in 0..entries_len {
        let entry = entries.add(i);
        let key_val = (*entry).key;
        if key_val.0 == 0 {
            continue; // deleted slot
        }
        let key_obj = key_val.0 as *mut Obj;
        let val_obj = (*entry).value.0 as *mut Obj;
        if (*key_obj).header.type_tag != TypeTagKind::Str
            || val_obj.is_null()
            || (*val_obj).header.type_tag != TypeTagKind::Str
        {
            continue;
        }
        let k = str_obj_to_rust_string(key_obj);
        let v = str_obj_to_rust_string(val_obj);
        f(k, v);
    }
}

/// Create an HttpResponseObj from HTTP response data.
///
/// # Safety
/// `headers` must be a valid pointer to a DictObj (or null).
/// All pointers are rooted across every allocating call so that a GC triggered
/// by any allocation cannot sweep them while we are still using them.
unsafe fn create_http_response(status: i64, url: &str, headers: *mut Obj, body: &[u8]) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};

    // Root `headers` across gc_alloc for the response object.
    // roots[0] = headers, roots[1] = response ptr (filled after alloc),
    // roots[2] = url string (filled after make_str_from_rust)
    let mut roots: [*mut Obj; 3] = [headers, std::ptr::null_mut(), std::ptr::null_mut()];
    let mut frame = ShadowFrame {
        prev: std::ptr::null_mut(),
        nroots: 3,
        roots: roots.as_mut_ptr(),
    };
    gc_push(&mut frame);

    let size = std::mem::size_of::<HttpResponseObj>();
    let ptr = gc::gc_alloc(size, TypeTagKind::HttpResponse as u8) as *mut HttpResponseObj;
    roots[1] = ptr as *mut Obj; // root the response object across subsequent allocs

    (*ptr).header = ObjHeader {
        type_tag: TypeTagKind::HttpResponse,
        marked: false,
        size,
    };
    (*ptr).status = status;

    // make_str_from_rust may trigger GC; headers and ptr are both rooted.
    let url_str = make_str_from_rust(url);
    roots[2] = url_str; // root url string across rt_make_bytes
    (*(roots[1] as *mut HttpResponseObj)).url = url_str;

    // rt_make_bytes may trigger GC; headers, ptr, and url_str are all rooted.
    let body_obj = rt_make_bytes(body.as_ptr(), body.len());

    gc_pop();

    let resp = roots[1] as *mut HttpResponseObj;
    (*resp).headers = roots[0]; // live headers pointer
    (*resp).body = body_obj;

    roots[1]
}

/// Core HTTP request implementation shared by every public entry point.
///
/// # Safety
/// `params_dict` and `headers_dict` must be valid `*mut DictObj` or null.
/// All GC-managed objects returned by runtime helpers are rooted internally
/// where needed.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn do_http_request(
    method: &str,
    url_str: &str,
    params_dict: *mut Obj,
    body: Option<&[u8]>,
    headers_dict: *mut Obj,
    timeout: f64,
    verify: bool,
    allow_redirects: bool,
    error_prefix: &str,
) -> *mut Obj {
    // Validate URL scheme
    if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
        crate::raise_exc!(
            crate::exceptions::ExceptionType::ValueError,
            "{}: unsupported URL scheme in '{}'",
            error_prefix,
            url_str
        );
    }

    // Build the agent with timeout + optional TLS / redirect overrides.
    // ureq 3.x uses Agent::config_builder() instead of AgentBuilder.
    //
    // TLS provider: `rustls-platform-verifier` delegates to the OS trust
    // store (macOS Keychain, Windows cert store, Linux /etc/ssl/...), so
    // the verified CA set matches what CPython's urllib.request sees.
    // Without this we fall back to `webpki-roots` — Mozilla's bundled CA
    // list — which misses a handful of ISP-issued CAs. Users reported
    // `UnknownIssuer` on sites like `example.com` (Cloudflare cert chain
    // not present in Mozilla bundle) that resolve fine in CPython.
    let timeout_duration = Duration::from_secs_f64(timeout.max(0.1));
    let mut tls_cfg = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier);
    if !verify {
        tls_cfg = tls_cfg.disable_verification(true);
    }
    let mut agent_cfg = ureq::Agent::config_builder()
        .timeout_global(Some(timeout_duration))
        .http_status_as_error(false) // Return response even for 4xx/5xx
        .tls_config(tls_cfg.build());
    if !allow_redirects {
        agent_cfg = agent_cfg.max_redirects(0);
    }
    let agent = agent_cfg.build().new_agent();

    // Collect params / headers from their DictObjs up-front so the request
    // builder can consume them without re-entering the closure (RequestBuilder
    // methods are value-consuming).
    let mut params: Vec<(String, String)> = Vec::new();
    for_each_str_entry(params_dict, |k, v| params.push((k, v)));
    let mut extra_headers: Vec<(String, String)> = Vec::new();
    for_each_str_entry(headers_dict, |k, v| extra_headers.push((k, v)));

    // ureq 3.x returns different builder types for body-less methods
    // (GET/DELETE/HEAD) and body-carrying ones (POST/PUT/PATCH), so we
    // dispatch per method. The `apply` macro dedupes the params/headers
    // loops since it works across both builder types.
    let response_result = {
        macro_rules! apply {
            ($r:expr) => {{
                let mut req = $r;
                for (k, v) in &params {
                    req = req.query(k.as_str(), v.as_str());
                }
                for (k, v) in &extra_headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                req
            }};
        }
        match method {
            "GET" => apply!(agent.get(url_str)).call(),
            "DELETE" => apply!(agent.delete(url_str)).call(),
            "HEAD" => apply!(agent.head(url_str)).call(),
            "POST" => {
                let req = apply!(agent.post(url_str));
                match body {
                    Some(b) => req.send(b),
                    None => req.send(b"" as &[u8]),
                }
            }
            "PUT" => {
                let req = apply!(agent.put(url_str));
                match body {
                    Some(b) => req.send(b),
                    None => req.send(b"" as &[u8]),
                }
            }
            "PATCH" => {
                let req = apply!(agent.patch(url_str));
                match body {
                    Some(b) => req.send(b),
                    None => req.send(b"" as &[u8]),
                }
            }
            other => {
                crate::raise_exc!(
                    crate::exceptions::ExceptionType::ValueError,
                    "{}: unsupported HTTP method '{}'",
                    error_prefix,
                    other
                );
            }
        }
    };

    match response_result {
        Ok(response) => {
            let status = response.status().as_u16() as i64;
            // Get the final URL (after redirects) — ureq 3.x doesn't expose
            // it directly, so fall back to the caller-supplied URL.
            let final_url = url_str.to_string();

            // Collect response headers into a dict.
            // Root [headers_dict, key_obj, value_obj] across every allocating
            // call (make_str_from_rust, rt_dict_set) so no GC can sweep them.
            let headers_dict = rt_make_dict(16);
            let mut hdr_roots: [*mut Obj; 3] =
                [headers_dict, std::ptr::null_mut(), std::ptr::null_mut()];
            let mut hdr_frame = crate::gc::ShadowFrame {
                prev: std::ptr::null_mut(),
                nroots: 3,
                roots: hdr_roots.as_mut_ptr(),
            };
            crate::gc::gc_push(&mut hdr_frame);
            for (name, value) in response.headers().iter() {
                let key_obj = make_str_from_rust(name.as_str());
                hdr_roots[1] = key_obj; // keep key alive across next make_str_from_rust
                let value_str = value.to_str().unwrap_or("");
                let value_obj = make_str_from_rust(value_str);
                hdr_roots[2] = value_obj; // keep value alive across rt_dict_set
                rt_dict_set(hdr_roots[0], hdr_roots[1], hdr_roots[2]);
                hdr_roots[1] = std::ptr::null_mut();
                hdr_roots[2] = std::ptr::null_mut();
            }
            crate::gc::gc_pop();
            let headers_dict = hdr_roots[0]; // live pointer after all allocations

            // Read the response body, capped at 1 GB to prevent OOM.
            const MAX_BODY_SIZE: u64 = 1 << 30; // 1 GB
            let body_bytes = match response
                .into_body()
                .into_with_config()
                .limit(MAX_BODY_SIZE)
                .read_to_vec()
            {
                Ok(bytes) => bytes,
                Err(e) => {
                    let e_str = e.to_string();
                    if e_str.contains("BodyExceedsLimit") || e_str.contains("body") {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::RuntimeError,
                            "{}: response body exceeds maximum allowed size ({} bytes)",
                            error_prefix,
                            MAX_BODY_SIZE
                        );
                    } else {
                        crate::raise_exc!(
                            crate::exceptions::ExceptionType::RuntimeError,
                            "{}: failed to read response body: {}",
                            error_prefix,
                            e
                        );
                    }
                }
            };

            create_http_response(status, &final_url, headers_dict, &body_bytes)
        }
        Err(e) => {
            crate::raise_exc!(
                crate::exceptions::ExceptionType::IOError,
                "{}: {}",
                error_prefix,
                e
            );
        }
    }
}

/// Extract a body byte slice from a `data: *mut Obj` argument. Returns None
/// if `data` is null; raises a Python IOError if it's non-null but not a
/// BytesObj.
///
/// # Safety
/// `data` must be a valid `*mut Obj` or null.
unsafe fn data_to_body_slice<'a>(data: *mut Obj, error_prefix: &str) -> Option<&'a [u8]> {
    if is_none_or_null(data) {
        return None;
    }
    if (*data).header.type_tag != TypeTagKind::Bytes {
        raise_io_error(&format!("{}: data must be bytes or None", error_prefix));
    }
    let bytes_obj = data as *const BytesObj;
    let data_len = (*bytes_obj).len;
    let data_ptr = (*bytes_obj).data.as_ptr();
    Some(std::slice::from_raw_parts(data_ptr, data_len))
}

/// `urllib.request.urlopen(url_or_request, data=None, timeout=30.0)`
///
/// The first argument is either a `StrObj` (plain URL) or a `RequestObj`
/// built via `urllib.request.Request(...)`. When a Request is passed, its
/// `data` / `headers` / `method` fields override the positional `data`
/// argument. This mirrors CPython semantics so code targeting pyaot runs
/// unchanged on any Python interpreter.
///
/// Returns an `HTTPResponse`.
///
/// TODO: CPython also accepts a `context=ssl.SSLContext` kwarg for TLS
/// configuration and supports custom `build_opener()` directors for
/// redirect handling. Both require new stdlib types (ssl.SSLContext,
/// OpenerDirector, HTTPRedirectHandler) — tracked as a follow-up for when
/// we need verify=False / allow_redirects=False from requests.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_urlopen(url_or_request: *mut Obj, data: *mut Obj, timeout: f64) -> *mut Obj {
    unsafe {
        if url_or_request.is_null() {
            raise_io_error("urlopen: URL cannot be None");
        }
        let tag = (*url_or_request).header.type_tag;
        if tag == TypeTagKind::Request {
            // Request path — fields of the Request drive the call. Positional
            // `data` is ignored, matching CPython behaviour.
            let req = url_or_request as *const RequestObj;
            let url_str = str_obj_to_rust_string((*req).url);
            let body = data_to_body_slice((*req).data, "urlopen");
            // Own the method String so the &str below is valid until the
            // call completes. A null pointer or a Python `None` singleton
            // both mean "no explicit method".
            let method_owned: String = if is_none_or_null((*req).method) {
                String::new()
            } else {
                str_obj_to_rust_string((*req).method)
            };
            let method: &str = if method_owned.is_empty() {
                if body.is_some() {
                    "POST"
                } else {
                    "GET"
                }
            } else {
                method_owned.as_str()
            };
            // Normalise headers — null and NoneObj both mean "no headers".
            let headers_for_call = if is_none_or_null((*req).headers) {
                std::ptr::null_mut()
            } else {
                (*req).headers
            };
            return do_http_request(
                method,
                &url_str,
                std::ptr::null_mut(),
                body,
                headers_for_call,
                timeout,
                true,
                true,
                "urlopen",
            );
        }
        if tag != TypeTagKind::Str {
            raise_io_error("urlopen: first argument must be a URL string or Request object");
        }
        // String URL path — legacy form: GET when data is null, POST otherwise.
        let url_str = str_obj_to_rust_string(url_or_request);
        let body = data_to_body_slice(data, "urlopen");
        let method = if body.is_some() { "POST" } else { "GET" };
        do_http_request(
            method,
            &url_str,
            std::ptr::null_mut(),
            body,
            std::ptr::null_mut(),
            timeout,
            true,
            true,
            "urlopen",
        )
    }
}
#[export_name = "rt_urlopen"]
pub extern "C" fn rt_urlopen_abi(url_or_request: Value, data: Value, timeout: f64) -> Value {
    Value::from_ptr(rt_urlopen(url_or_request.unwrap_ptr(), data.unwrap_ptr(), timeout))
}


// =============================================================================
// HTTPResponse field getters
// =============================================================================

/// Get status field from HTTPResponse
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_get_status(obj: *mut Obj) -> i64 {
    if obj.is_null() {
        return 0;
    }
    unsafe {
        crate::debug_assert_type_tag!(
            obj,
            TypeTagKind::HttpResponse,
            "rt_http_response_get_status"
        );
        let hr = obj as *const HttpResponseObj;
        (*hr).status
    }
}
#[export_name = "rt_http_response_get_status"]
pub extern "C" fn rt_http_response_get_status_abi(obj: Value) -> i64 {
    rt_http_response_get_status(obj.unwrap_ptr())
}


/// Get url field from HTTPResponse
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_get_url(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::HttpResponse, "rt_http_response_get_url");
        let hr = obj as *const HttpResponseObj;
        (*hr).url
    }
}
#[export_name = "rt_http_response_get_url"]
pub extern "C" fn rt_http_response_get_url_abi(obj: Value) -> Value {
    Value::from_ptr(rt_http_response_get_url(obj.unwrap_ptr()))
}


/// Get headers field from HTTPResponse
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_get_headers(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return rt_make_dict(0);
    }
    unsafe {
        crate::debug_assert_type_tag!(
            obj,
            TypeTagKind::HttpResponse,
            "rt_http_response_get_headers"
        );
        let hr = obj as *const HttpResponseObj;
        (*hr).headers
    }
}
#[export_name = "rt_http_response_get_headers"]
pub extern "C" fn rt_http_response_get_headers_abi(obj: Value) -> Value {
    Value::from_ptr(rt_http_response_get_headers(obj.unwrap_ptr()))
}


// =============================================================================
// HTTPResponse methods
// =============================================================================

/// HTTPResponse.read() - Read the response body as bytes
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_read(obj: *mut Obj) -> *mut Obj {
    unsafe {
        if obj.is_null() {
            return rt_make_bytes(std::ptr::null(), 0);
        }
        crate::debug_assert_type_tag!(obj, TypeTagKind::HttpResponse, "rt_http_response_read");
        let hr = obj as *const HttpResponseObj;
        (*hr).body
    }
}
#[export_name = "rt_http_response_read"]
pub extern "C" fn rt_http_response_read_abi(obj: Value) -> Value {
    Value::from_ptr(rt_http_response_read(obj.unwrap_ptr()))
}


/// HTTPResponse.ok — true iff status is 2xx (requests-library convention).
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_get_ok(obj: *mut Obj) -> i8 {
    if obj.is_null() {
        return 0;
    }
    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::HttpResponse, "rt_http_response_get_ok");
        let hr = obj as *const HttpResponseObj;
        if (200..300).contains(&(*hr).status) {
            1
        } else {
            0
        }
    }
}
#[export_name = "rt_http_response_get_ok"]
pub extern "C" fn rt_http_response_get_ok_abi(obj: Value) -> i8 {
    rt_http_response_get_ok(obj.unwrap_ptr())
}


/// HTTPResponse.text — decode the response body as UTF-8 string.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_get_text(obj: *mut Obj) -> *mut Obj {
    unsafe {
        if obj.is_null() {
            return make_str_from_rust("");
        }
        crate::debug_assert_type_tag!(obj, TypeTagKind::HttpResponse, "rt_http_response_get_text");
        let hr = obj as *const HttpResponseObj;
        let body = (*hr).body;
        if body.is_null() {
            return make_str_from_rust("");
        }
        let bytes_obj = body as *const BytesObj;
        let data_len = (*bytes_obj).len;
        let data_ptr = (*bytes_obj).data.as_ptr();
        let slice = std::slice::from_raw_parts(data_ptr, data_len);
        let s = std::str::from_utf8(slice).unwrap_or("");
        make_str_from_rust(s)
    }
}
#[export_name = "rt_http_response_get_text"]
pub extern "C" fn rt_http_response_get_text_abi(obj: Value) -> Value {
    Value::from_ptr(rt_http_response_get_text(obj.unwrap_ptr()))
}


/// HTTPResponse.json() — parse body as JSON (requests-library convention).
/// Delegates to `rt_json_loads`.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_json(obj: *mut Obj) -> *mut Obj {
    unsafe {
        let text = rt_http_response_get_text(obj);
        crate::json::rt_json_loads(text)
    }
}
#[export_name = "rt_http_response_json"]
pub extern "C" fn rt_http_response_json_abi(obj: Value) -> Value {
    Value::from_ptr(rt_http_response_json(obj.unwrap_ptr()))
}


/// HTTPResponse.geturl() - Get the URL of the response
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_geturl(obj: *mut Obj) -> *mut Obj {
    rt_http_response_get_url(obj)
}
#[export_name = "rt_http_response_geturl"]
pub extern "C" fn rt_http_response_geturl_abi(obj: Value) -> Value {
    Value::from_ptr(rt_http_response_geturl(obj.unwrap_ptr()))
}


/// HTTPResponse.getcode() - Get the HTTP status code
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_getcode(obj: *mut Obj) -> i64 {
    rt_http_response_get_status(obj)
}
#[export_name = "rt_http_response_getcode"]
pub extern "C" fn rt_http_response_getcode_abi(obj: Value) -> i64 {
    rt_http_response_getcode(obj.unwrap_ptr())
}


/// repr() for HTTPResponse
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_http_response_repr(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("<http.client.HTTPResponse>") };
    }

    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::HttpResponse, "rt_http_response_repr");
        let hr = obj as *const HttpResponseObj;
        let repr = format!("<http.client.HTTPResponse [{}]>", (*hr).status);
        make_str_from_rust(&repr)
    }
}
#[export_name = "rt_http_response_repr"]
pub extern "C" fn rt_http_response_repr_abi(obj: Value) -> Value {
    Value::from_ptr(rt_http_response_repr(obj.unwrap_ptr()))
}


// =============================================================================
// urllib.request.Request — standard CPython type bundling URL + body +
// headers + method for use with urlopen().
// =============================================================================

/// Construct a new `Request(url, data=None, headers=None, method=None)`.
///
/// # Safety
/// Each non-null argument must point at a valid runtime object of the
/// expected type (StrObj / BytesObj / DictObj / StrObj).
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_make_request(
    url: *mut Obj,
    data: *mut Obj,
    headers: *mut Obj,
    method: *mut Obj,
) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    unsafe {
        // Root every caller-supplied pointer across the allocating call.
        let mut roots: [*mut Obj; 4] = [url, data, headers, method];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 4,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let size = std::mem::size_of::<RequestObj>();
        let ptr = gc::gc_alloc(size, TypeTagKind::Request as u8) as *mut RequestObj;

        (*ptr).header = ObjHeader {
            type_tag: TypeTagKind::Request,
            marked: false,
            size,
        };
        (*ptr).url = roots[0];
        (*ptr).data = roots[1];
        (*ptr).headers = roots[2];
        (*ptr).method = roots[3];

        gc_pop();
        ptr as *mut Obj
    }
}
#[export_name = "rt_make_request"]
pub extern "C" fn rt_make_request_abi(
    url: Value,
    data: Value,
    headers: Value,
    method: Value,
) -> Value {
    Value::from_ptr(rt_make_request(url.unwrap_ptr(), data.unwrap_ptr(), headers.unwrap_ptr(), method.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_request_get_url(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::Request, "rt_request_get_url");
        (*(obj as *const RequestObj)).url
    }
}
#[export_name = "rt_request_get_url"]
pub extern "C" fn rt_request_get_url_abi(obj: Value) -> Value {
    Value::from_ptr(rt_request_get_url(obj.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_request_get_data(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::Request, "rt_request_get_data");
        (*(obj as *const RequestObj)).data
    }
}
#[export_name = "rt_request_get_data"]
pub extern "C" fn rt_request_get_data_abi(obj: Value) -> Value {
    Value::from_ptr(rt_request_get_data(obj.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_request_get_headers(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return rt_make_dict(0);
    }
    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::Request, "rt_request_get_headers");
        (*(obj as *const RequestObj)).headers
    }
}
#[export_name = "rt_request_get_headers"]
pub extern "C" fn rt_request_get_headers_abi(obj: Value) -> Value {
    Value::from_ptr(rt_request_get_headers(obj.unwrap_ptr()))
}


#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_request_get_method(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("GET") };
    }
    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::Request, "rt_request_get_method");
        let method = (*(obj as *const RequestObj)).method;
        if method.is_null() {
            return make_str_from_rust("GET");
        }
        method
    }
}
#[export_name = "rt_request_get_method"]
pub extern "C" fn rt_request_get_method_abi(obj: Value) -> Value {
    Value::from_ptr(rt_request_get_method(obj.unwrap_ptr()))
}


// =============================================================================
// urllib.request.urlretrieve — download URL to a local file and return
// (filename, headers) tuple.
// =============================================================================

/// `urllib.request.urlretrieve(url, filename, reporthook=None, data=None)`
///
/// Fetches `url` and writes the response body to `filename`. Returns a
/// 2-tuple `(filename, headers_dict)` — CPython returns `(filename,
/// HTTPMessage)`, but pyaot has no HTTPMessage type, so the headers come
/// back as a `dict[str, str]`.
///
/// `reporthook` is accepted for source compatibility with CPython but is
/// never called — pyaot writes the full body in one step.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn rt_urlretrieve(
    url: *mut Obj,
    filename: *mut Obj,
    _reporthook: *mut Obj,
    data: *mut Obj,
) -> *mut Obj {
    use crate::gc::{gc_pop, gc_push, ShadowFrame};
    unsafe {
        if url.is_null() || (*url).header.type_tag != TypeTagKind::Str {
            raise_io_error("urlretrieve: url must be a str");
        }
        if filename.is_null() || (*filename).header.type_tag != TypeTagKind::Str {
            raise_io_error("urlretrieve: filename must be a str");
        }

        let url_str = str_obj_to_rust_string(url);
        let filename_str = str_obj_to_rust_string(filename);
        let body = data_to_body_slice(data, "urlretrieve");
        let method = if body.is_some() { "POST" } else { "GET" };

        let response = do_http_request(
            method,
            &url_str,
            std::ptr::null_mut(),
            body,
            std::ptr::null_mut(),
            30.0,
            true,
            true,
            "urlretrieve",
        );

        let hr = response as *const HttpResponseObj;
        let body_obj = (*hr).body;
        if body_obj.is_null() || (*body_obj).header.type_tag != TypeTagKind::Bytes {
            raise_io_error("urlretrieve: response body is missing");
        }
        let bytes_obj = body_obj as *const BytesObj;
        let data_len = (*bytes_obj).len;
        let data_ptr = (*bytes_obj).data.as_ptr();
        let body_slice = std::slice::from_raw_parts(data_ptr, data_len);

        if let Err(e) = std::fs::write(&filename_str, body_slice) {
            raise_io_error(&format!(
                "urlretrieve: failed to write '{}': {}",
                filename_str, e
            ));
        }

        // Build (filename, headers) tuple. Root response (keeps headers
        // reachable) and filename across the tuple allocation.
        let headers_dict = (*hr).headers;
        let mut roots: [*mut Obj; 4] = [response, filename, headers_dict, std::ptr::null_mut()];
        let mut frame = ShadowFrame {
            prev: std::ptr::null_mut(),
            nroots: 4,
            roots: roots.as_mut_ptr(),
        };
        gc_push(&mut frame);

        let tuple = crate::tuple::rt_make_tuple(2);
        roots[3] = tuple;
        crate::tuple::rt_tuple_set(roots[3], 0, roots[1]);
        crate::tuple::rt_tuple_set(roots[3], 1, roots[2]);

        gc_pop();
        roots[3]
    }
}
#[export_name = "rt_urlretrieve"]
pub extern "C" fn rt_urlretrieve_abi(
    url: Value,
    filename: Value,
    _reporthook: Value,
    data: Value,
) -> Value {
    Value::from_ptr(rt_urlretrieve(url.unwrap_ptr(), filename.unwrap_ptr(), _reporthook.unwrap_ptr(), data.unwrap_ptr()))
}

