//! urllib.request module runtime support
//!
//! Provides HTTP request functionality:
//! - urlopen(url, data=None, timeout=30.0): Open a URL and return an HTTPResponse

use crate::bytes::rt_make_bytes;
use crate::dict::{rt_dict_set, rt_make_dict};
use crate::gc;
use crate::object::{BytesObj, HttpResponseObj, Obj, ObjHeader, TypeTagKind};
use crate::utils::{make_str_from_rust, raise_io_error, str_obj_to_rust_string};
use std::time::Duration;

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

/// urllib.request.urlopen(url, data=None, timeout=30.0) - Open a URL
/// Returns an HTTPResponse object
///
/// # Arguments
/// * `url` - The URL to open (StrObj)
/// * `data` - Optional bytes to send as POST data (BytesObj or null for GET)
/// * `timeout` - Timeout in seconds (f64)
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn rt_urlopen(url: *mut Obj, data: *mut Obj, timeout: f64) -> *mut Obj {
    unsafe {
        if url.is_null() {
            raise_io_error("urlopen: URL cannot be None");
        }
        let url_str = str_obj_to_rust_string(url);

        // Validate URL scheme
        if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
            crate::raise_exc!(
                crate::exceptions::ExceptionType::ValueError,
                "unsupported URL scheme in '{}'",
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

        // Determine if this is GET or POST based on data
        let response_result = if data.is_null() {
            // GET request
            agent.get(&url_str).call()
        } else {
            // POST request with data
            if (*data).header.type_tag != TypeTagKind::Bytes {
                raise_io_error("urlopen: data must be bytes or None");
            }
            let bytes_obj = data as *const BytesObj;
            let data_len = (*bytes_obj).len;
            let data_ptr = (*bytes_obj).data.as_ptr();
            let body_data = std::slice::from_raw_parts(data_ptr, data_len);

            agent
                .post(&url_str)
                .content_type("application/x-www-form-urlencoded")
                .send(body_data)
        };

        // Handle the response
        match response_result {
            Ok(response) => {
                let status = response.status().as_u16() as i64;
                // Get the final URL (after redirects) - ureq 3.x doesn't expose this directly
                let final_url = url_str.clone();

                // Collect headers into a dict.
                // Root [headers_dict, key_obj, value_obj] across every allocating call
                // (make_str_from_rust, rt_dict_set) so no GC can sweep them.
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
                // ureq's into_with_config().limit() raises an error if the body
                // exceeds the given size, so we don't need a manual take wrapper.
                const MAX_BODY_SIZE: u64 = 1 << 30; // 1 GB
                let body_bytes = match response
                    .into_body()
                    .into_with_config()
                    .limit(MAX_BODY_SIZE)
                    .read_to_vec()
                {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        // Distinguish limit-exceeded from other I/O errors so we
                        // raise a descriptive RuntimeError rather than a generic IOError.
                        let e_str = e.to_string();
                        if e_str.contains("BodyExceedsLimit") || e_str.contains("body") {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::RuntimeError,
                                "urlopen: response body exceeds maximum allowed size ({} bytes)",
                                MAX_BODY_SIZE
                            );
                        } else {
                            crate::raise_exc!(
                                crate::exceptions::ExceptionType::RuntimeError,
                                "urlopen: failed to read response body: {}",
                                e
                            );
                        }
                    }
                };

                create_http_response(status, &final_url, headers_dict, &body_bytes)
            }
            Err(e) => {
                crate::utils::raise_io_error_owned(format!("urlopen: {}", e));
            }
        }
    }
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
