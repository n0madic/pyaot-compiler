//! `requests` package — ergonomic HTTP client API.
//!
//! Shipped as a separate staticlib (`libpyaot_pkg_requests.a`) that is linked
//! into the user's binary only when the compiled Python script contains
//! `import requests`. Metadata (`REQUESTS_MODULE`) is consumed by the compiler
//! at build time through the `pyaot-pkg-defs` registry.
//!
//! For the initial POC the implementation delegates HTTP work to the runtime's
//! `rt_urlopen` symbol (exported when the runtime is built with the
//! `stdlib-network` feature, which is on by default). Later revisions are
//! expected to perform HTTP calls directly via `ureq` inside this crate once a
//! stable runtime ABI crate exists for constructing `HttpResponseObj` values
//! independently of the main runtime internals.

use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, R_I64};
use pyaot_core_defs::RuntimeFuncDef;
use pyaot_stdlib_defs::{
    ConstValue, LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec,
};

/// Opaque runtime object pointer. Layout-compatible with `pyaot_runtime::Obj`.
///
/// We never dereference this here; the pointer is passed through to runtime
/// helpers that own the layout definition.
#[repr(C)]
pub struct Obj {
    _opaque: [u8; 0],
}

extern "C" {
    /// From the main runtime (`crates/runtime/src/urllib_request.rs`). Performs
    /// the HTTP request and constructs an `HttpResponseObj`.
    fn rt_urlopen(url: *mut Obj, data: *mut Obj, timeout: f64) -> *mut Obj;
}

/// `requests.get(url, timeout=5.0) -> HTTPResponse`
///
/// # Safety
/// `url` must be a valid pointer to a `StrObj` or null; runtime helpers
/// validate arguments and raise Python exceptions on invalid input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_requests_get(url: *mut Obj, timeout: f64) -> *mut Obj {
    // Delegate to the runtime HTTP implementation. A GET is signaled by
    // passing a null `data` pointer.
    unsafe { rt_urlopen(url, std::ptr::null_mut(), timeout) }
}

// =============================================================================
// Compile-time module metadata consumed by the compiler.
// =============================================================================

/// `requests.get(url, timeout=5.0)`
pub static REQUESTS_GET: StdlibFunctionDef = StdlibFunctionDef {
    name: "get",
    runtime_name: "rt_requests_get",
    params: &[
        ParamDef::required("url", TypeSpec::Str),
        ParamDef::optional_with_default("timeout", TypeSpec::Float, ConstValue::Float(5.0)),
    ],
    return_type: TypeSpec::HttpResponse,
    min_args: 1,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_requests_get", &[P_I64, P_F64], Some(R_I64), false),
};

/// Top-level module description exported to the compiler's package registry.
pub static REQUESTS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "requests",
    functions: &[REQUESTS_GET],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
