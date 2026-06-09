//! `http.client` module definition.
//!
//! CPython places the `HTTPResponse` class here; `urllib.request.urlopen()`
//! returns `http.client.HTTPResponse` instances. Exposing it under the same
//! module name keeps bundled Python code portable — e.g. `from http.client
//! import HTTPResponse` works identically on CPython and pyaot.
//!
//! The method defs themselves live in `modules/urllib.rs` because they are
//! declared next to the urllib entry points that consume them; this module
//! only re-binds them into the `http.client` namespace via the class.

use crate::modules::urllib::{
    HTTP_RESPONSE_GETCODE, HTTP_RESPONSE_GETURL, HTTP_RESPONSE_JSON, HTTP_RESPONSE_READ,
};
use crate::types::{StdlibClassDef, StdlibModuleDef, TypeSpec};

/// `http.client.HTTPResponse` class — the canonical home of this type in
/// the CPython standard library. Plus a requests-library-compatible
/// `.json()` method so pyaot's bundled requests behaves like the real pip
/// package.
pub static HTTP_RESPONSE_CLASS: StdlibClassDef = StdlibClassDef {
    name: "HTTPResponse",
    methods: &[
        HTTP_RESPONSE_READ,
        HTTP_RESPONSE_GETURL,
        HTTP_RESPONSE_GETCODE,
        HTTP_RESPONSE_JSON,
    ],
    type_spec: Some(TypeSpec::HttpResponse),
};

/// `http.client` module — hosts `HTTPResponse` for CPython compatibility.
pub static HTTP_CLIENT_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "http.client",
    functions: &[],
    attrs: &[],
    constants: &[],
    classes: &[HTTP_RESPONSE_CLASS],
    exceptions: &[],
    submodules: &[],
};

/// `http` parent module — currently exposes only `http.client`.
pub static HTTP_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "http",
    functions: &[],
    attrs: &[],
    constants: &[],
    classes: &[],
    exceptions: &[],
    submodules: &[&HTTP_CLIENT_MODULE],
};
