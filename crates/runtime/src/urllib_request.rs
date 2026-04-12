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
use crate::object::{BytesObj, DictObj, HttpResponseObj, Obj, ObjHeader, TypeTagKind};
use crate::utils::{make_str_from_rust, raise_io_error, str_obj_to_rust_string};
use std::time::Duration;

/// Iterate `str -> str` entries of a DictObj. Entries with non-`Str` keys or
/// values are silently skipped (the stdlib type system only allows
/// `dict[str, str]` at call sites, so this is defensive).
///
/// # Safety
/// `dict` must be a valid `*mut DictObj` or null.
unsafe fn for_each_str_entry<F: FnMut(String, String)>(dict: *mut Obj, mut f: F) {
    if dict.is_null() {
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
        let key_obj = (*entry).key;
        if key_obj.is_null() {
            continue; // deleted slot
        }
        let val_obj = (*entry).value;
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
unsafe fn do_http_request(
    method: &str,
    url_str: &str,
    params_dict: *mut Obj,
    body: Option<&[u8]>,
    headers_dict: *mut Obj,
    timeout: f64,
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

    // Build the agent with timeout configuration
    // ureq 3.x uses Agent::config_builder() instead of AgentBuilder
    let timeout_duration = Duration::from_secs_f64(timeout.max(0.1));
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout_duration))
        .http_status_as_error(false) // Return response even for 4xx/5xx
        .build()
        .new_agent();

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
    if data.is_null() {
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

/// `urllib.request.urlopen(url, data=None, timeout=30.0)` — Open a URL.
/// Returns an `HTTPResponse` object. GET when `data` is null, POST otherwise.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_urlopen(url: *mut Obj, data: *mut Obj, timeout: f64) -> *mut Obj {
    unsafe {
        if url.is_null() {
            raise_io_error("urlopen: URL cannot be None");
        }
        let url_str = str_obj_to_rust_string(url);
        let body = data_to_body_slice(data, "urlopen");
        let method = if body.is_some() { "POST" } else { "GET" };
        do_http_request(
            method,
            &url_str,
            std::ptr::null_mut(),
            body,
            std::ptr::null_mut(),
            timeout,
            "urlopen",
        )
    }
}

/// Generic HTTP request entry point used by the `pyaot-pkg-requests` crate.
///
/// `method_ptr` / `method_len` point to a UTF-8 HTTP method literal (e.g.
/// `b"POST"`). `params` and `headers` are `dict[str, str]` (or null). `data`
/// is a `BytesObj` request body (or null).
///
/// # Safety
/// `method_ptr` must reference at least `method_len` valid UTF-8 bytes.
/// `url` must be a non-null StrObj. Dict/body arguments must point to valid
/// runtime objects of the claimed type or be null.
#[no_mangle]
pub unsafe extern "C" fn rt_http_request_raw(
    method_ptr: *const u8,
    method_len: usize,
    url: *mut Obj,
    params: *mut Obj,
    data: *mut Obj,
    headers: *mut Obj,
    timeout: f64,
) -> *mut Obj {
    if url.is_null() {
        raise_io_error("http_request: URL cannot be None");
    }
    let method_bytes = std::slice::from_raw_parts(method_ptr, method_len);
    let method = std::str::from_utf8(method_bytes).unwrap_or("GET");
    let url_str = str_obj_to_rust_string(url);
    let body = data_to_body_slice(data, "http_request");
    do_http_request(
        method,
        &url_str,
        params,
        body,
        headers,
        timeout,
        "http_request",
    )
}

// =============================================================================
// HTTPResponse field getters
// =============================================================================

/// Get status field from HTTPResponse
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_http_response_get_status(obj: *mut Obj) -> i64 {
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

/// Get url field from HTTPResponse
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_http_response_get_url(obj: *mut Obj) -> *mut Obj {
    if obj.is_null() {
        return unsafe { make_str_from_rust("") };
    }
    unsafe {
        crate::debug_assert_type_tag!(obj, TypeTagKind::HttpResponse, "rt_http_response_get_url");
        let hr = obj as *const HttpResponseObj;
        (*hr).url
    }
}

/// Get headers field from HTTPResponse
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_http_response_get_headers(obj: *mut Obj) -> *mut Obj {
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

// =============================================================================
// HTTPResponse methods
// =============================================================================

/// HTTPResponse.read() - Read the response body as bytes
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_http_response_read(obj: *mut Obj) -> *mut Obj {
    unsafe {
        if obj.is_null() {
            return rt_make_bytes(std::ptr::null(), 0);
        }
        crate::debug_assert_type_tag!(obj, TypeTagKind::HttpResponse, "rt_http_response_read");
        let hr = obj as *const HttpResponseObj;
        (*hr).body
    }
}

/// HTTPResponse.geturl() - Get the URL of the response
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_http_response_geturl(obj: *mut Obj) -> *mut Obj {
    rt_http_response_get_url(obj)
}

/// HTTPResponse.getcode() - Get the HTTP status code
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_http_response_getcode(obj: *mut Obj) -> i64 {
    rt_http_response_get_status(obj)
}

/// repr() for HTTPResponse
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_http_response_repr(obj: *mut Obj) -> *mut Obj {
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
