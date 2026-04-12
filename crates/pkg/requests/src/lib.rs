//! `requests` package — ergonomic HTTP client API.
//!
//! Shipped as a separate staticlib (`libpyaot_pkg_requests.a`) that is linked
//! into the user's binary only when the compiled Python script contains
//! `import requests`. Metadata (`REQUESTS_MODULE`) is consumed by the
//! compiler at build time through the `pyaot-pkg-defs` registry.
//!
//! Each function forwards to the main runtime's `rt_http_request_raw`
//! helper; this keeps the package as a thin facade and reuses the single
//! ureq pipeline that also backs `urllib.request.urlopen`. A future
//! follow-up may move HTTP execution into this crate once a stable
//! `pyaot-runtime-abi` crate exists.

use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, R_I64};
use pyaot_core_defs::RuntimeFuncDef;
use pyaot_stdlib_defs::{
    ConstValue, LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec,
};

/// Opaque runtime object pointer. Layout-compatible with
/// `pyaot_runtime::Obj`. We never dereference this; the pointer is passed
/// through to runtime helpers that own the layout definition.
#[repr(C)]
pub struct Obj {
    _opaque: [u8; 0],
}

// Generic HTTP request entry point exported by the main runtime when built
// with `stdlib-network`. See `crates/runtime/src/urllib_request.rs`.
extern "C" {
    fn rt_http_request_raw(
        method_ptr: *const u8,
        method_len: usize,
        url: *mut Obj,
        params: *mut Obj,  // dict[str, str] or null
        data: *mut Obj,    // bytes or null
        headers: *mut Obj, // dict[str, str] or null
        timeout: f64,
    ) -> *mut Obj;
}

// Small helper: convert a method literal into `(ptr, len)` without
// allocating. Used by each method wrapper below.
macro_rules! method_ptr_len {
    ($name:literal) => {{
        let b: &'static [u8] = $name.as_bytes();
        (b.as_ptr(), b.len())
    }};
}

// =============================================================================
// extern "C" entry points consumed by generated code
// =============================================================================

/// `requests.get(url, params=None, headers=None, timeout=5.0)`
///
/// # Safety
/// `url` must be a non-null `StrObj`. `params` / `headers` must be
/// `DictObj[str, str]` or null. Runtime helpers validate arguments and raise
/// Python exceptions on invalid input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_requests_get(
    url: *mut Obj,
    params: *mut Obj,
    headers: *mut Obj,
    timeout: f64,
) -> *mut Obj {
    let (p, l) = method_ptr_len!("GET");
    unsafe { rt_http_request_raw(p, l, url, params, std::ptr::null_mut(), headers, timeout) }
}

/// `requests.post(url, data=None, headers=None, timeout=5.0)`
///
/// # Safety
/// `url` must be a non-null `StrObj`. `data` must be a `BytesObj` or null;
/// `headers` must be `DictObj[str, str]` or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_requests_post(
    url: *mut Obj,
    data: *mut Obj,
    headers: *mut Obj,
    timeout: f64,
) -> *mut Obj {
    let (p, l) = method_ptr_len!("POST");
    unsafe { rt_http_request_raw(p, l, url, std::ptr::null_mut(), data, headers, timeout) }
}

/// `requests.put(url, data=None, headers=None, timeout=5.0)`
///
/// # Safety
/// Same invariants as `rt_requests_post`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_requests_put(
    url: *mut Obj,
    data: *mut Obj,
    headers: *mut Obj,
    timeout: f64,
) -> *mut Obj {
    let (p, l) = method_ptr_len!("PUT");
    unsafe { rt_http_request_raw(p, l, url, std::ptr::null_mut(), data, headers, timeout) }
}

/// `requests.delete(url, headers=None, timeout=5.0)`
///
/// # Safety
/// `url` must be a non-null `StrObj`. `headers` must be `DictObj[str, str]`
/// or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_requests_delete(
    url: *mut Obj,
    headers: *mut Obj,
    timeout: f64,
) -> *mut Obj {
    let (p, l) = method_ptr_len!("DELETE");
    unsafe {
        rt_http_request_raw(
            p,
            l,
            url,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            headers,
            timeout,
        )
    }
}

// =============================================================================
// Compile-time module metadata consumed by the compiler.
// =============================================================================

/// Shared type: `dict[str, str]` used for both query params and headers.
static TYPE_DICT_STR_STR: TypeSpec = TypeSpec::Dict(&TypeSpec::Str, &TypeSpec::Str);
static TYPE_OPT_DICT_STR_STR: TypeSpec = TypeSpec::Optional(&TYPE_DICT_STR_STR);
static TYPE_OPT_BYTES: TypeSpec = TypeSpec::Optional(&TypeSpec::Bytes);

/// `requests.get(url, params=None, headers=None, timeout=5.0)`
pub static REQUESTS_GET: StdlibFunctionDef = StdlibFunctionDef {
    name: "get",
    runtime_name: "rt_requests_get",
    params: &[
        ParamDef::required("url", TypeSpec::Str),
        ParamDef::optional("params", TYPE_OPT_DICT_STR_STR),
        ParamDef::optional("headers", TYPE_OPT_DICT_STR_STR),
        ParamDef::optional_with_default("timeout", TypeSpec::Float, ConstValue::Float(5.0)),
    ],
    return_type: TypeSpec::HttpResponse,
    min_args: 1,
    max_args: 4,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_requests_get",
        &[P_I64, P_I64, P_I64, P_F64],
        Some(R_I64),
        false,
    ),
};

/// `requests.post(url, data=None, headers=None, timeout=5.0)`
pub static REQUESTS_POST: StdlibFunctionDef = StdlibFunctionDef {
    name: "post",
    runtime_name: "rt_requests_post",
    params: &[
        ParamDef::required("url", TypeSpec::Str),
        ParamDef::optional("data", TYPE_OPT_BYTES),
        ParamDef::optional("headers", TYPE_OPT_DICT_STR_STR),
        ParamDef::optional_with_default("timeout", TypeSpec::Float, ConstValue::Float(5.0)),
    ],
    return_type: TypeSpec::HttpResponse,
    min_args: 1,
    max_args: 4,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_requests_post",
        &[P_I64, P_I64, P_I64, P_F64],
        Some(R_I64),
        false,
    ),
};

/// `requests.put(url, data=None, headers=None, timeout=5.0)`
pub static REQUESTS_PUT: StdlibFunctionDef = StdlibFunctionDef {
    name: "put",
    runtime_name: "rt_requests_put",
    params: &[
        ParamDef::required("url", TypeSpec::Str),
        ParamDef::optional("data", TYPE_OPT_BYTES),
        ParamDef::optional("headers", TYPE_OPT_DICT_STR_STR),
        ParamDef::optional_with_default("timeout", TypeSpec::Float, ConstValue::Float(5.0)),
    ],
    return_type: TypeSpec::HttpResponse,
    min_args: 1,
    max_args: 4,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_requests_put",
        &[P_I64, P_I64, P_I64, P_F64],
        Some(R_I64),
        false,
    ),
};

/// `requests.delete(url, headers=None, timeout=5.0)`
pub static REQUESTS_DELETE: StdlibFunctionDef = StdlibFunctionDef {
    name: "delete",
    runtime_name: "rt_requests_delete",
    params: &[
        ParamDef::required("url", TypeSpec::Str),
        ParamDef::optional("headers", TYPE_OPT_DICT_STR_STR),
        ParamDef::optional_with_default("timeout", TypeSpec::Float, ConstValue::Float(5.0)),
    ],
    return_type: TypeSpec::HttpResponse,
    min_args: 1,
    max_args: 3,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_requests_delete",
        &[P_I64, P_I64, P_F64],
        Some(R_I64),
        false,
    ),
};

/// Top-level module description exported to the compiler's package registry.
pub static REQUESTS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "requests",
    functions: &[REQUESTS_GET, REQUESTS_POST, REQUESTS_PUT, REQUESTS_DELETE],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
