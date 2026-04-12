//! urllib module definition
//!
//! Provides URL parsing and encoding functions through urllib.parse submodule,
//! and HTTP request functionality through urllib.request submodule.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibClassDef, StdlibFunctionDef, StdlibMethodDef,
    StdlibModuleDef, TypeSpec, TYPE_DICT_STR_STR, TYPE_LIST_STR, TYPE_STR,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

// Static type reference specific to urllib
pub static TYPE_DICT_STR_LIST_STR: TypeSpec = TypeSpec::Dict(&TYPE_STR, &TYPE_LIST_STR);

// =============================================================================
// urllib.parse module functions
// =============================================================================

/// urllib.parse.urlparse(url) - Parse a URL into components
/// Returns a ParseResult with scheme, netloc, path, params, query, fragment
pub static URLPARSE: StdlibFunctionDef = StdlibFunctionDef {
    name: "urlparse",
    runtime_name: "rt_urlparse",
    params: &[ParamDef::required("url", TypeSpec::Str)],
    return_type: TypeSpec::ParseResult,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_urlparse", &[P_I64], Some(R_I64), false),
};

/// urllib.parse.urlencode(params) - Encode a dict as a query string
/// Example: {"key": "value"} -> "key=value"
pub static URLENCODE: StdlibFunctionDef = StdlibFunctionDef {
    name: "urlencode",
    runtime_name: "rt_urlencode",
    params: &[ParamDef::required("params", TYPE_DICT_STR_STR)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_urlencode", &[P_I64], Some(R_I64), false),
};

/// urllib.parse.quote(string, safe='') - Percent-encode a string
/// Characters in `safe` are not encoded
pub static QUOTE: StdlibFunctionDef = StdlibFunctionDef {
    name: "quote",
    runtime_name: "rt_quote",
    params: &[
        ParamDef::required("string", TypeSpec::Str),
        ParamDef::optional_with_default("safe", TypeSpec::Str, ConstValue::Str("")),
    ],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_quote", &[P_I64, P_I64], Some(R_I64), false),
};

/// urllib.parse.unquote(string) - Decode percent-encoded string
pub static UNQUOTE: StdlibFunctionDef = StdlibFunctionDef {
    name: "unquote",
    runtime_name: "rt_unquote",
    params: &[ParamDef::required("string", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_unquote", &[P_I64], Some(R_I64), false),
};

/// urllib.parse.urljoin(base, url) - Join a base URL with a relative URL
pub static URLJOIN: StdlibFunctionDef = StdlibFunctionDef {
    name: "urljoin",
    runtime_name: "rt_urljoin",
    params: &[
        ParamDef::required("base", TypeSpec::Str),
        ParamDef::required("url", TypeSpec::Str),
    ],
    return_type: TypeSpec::Str,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_urljoin", &[P_I64, P_I64], Some(R_I64), false),
};

/// urllib.parse.parse_qs(query) - Parse a query string into a dict
/// Returns dict[str, list[str]] since keys can have multiple values
pub static PARSE_QS: StdlibFunctionDef = StdlibFunctionDef {
    name: "parse_qs",
    runtime_name: "rt_parse_qs",
    params: &[ParamDef::required("query", TypeSpec::Str)],
    return_type: TYPE_DICT_STR_LIST_STR,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_parse_qs", &[P_I64], Some(R_I64), false),
};

// =============================================================================
// ParseResult class methods (field accessors are defined in object_types.rs)
// =============================================================================

/// ParseResult.geturl() - Reassemble the URL from components
pub static PARSE_RESULT_GETURL: StdlibMethodDef = StdlibMethodDef {
    name: "geturl",
    runtime_name: "rt_parse_result_geturl",
    params: &[],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_parse_result_geturl", &[P_I64], Some(R_I64), false),
};

/// ParseResult class definition
static PARSE_RESULT_CLASS: StdlibClassDef = StdlibClassDef {
    name: "ParseResult",
    methods: &[PARSE_RESULT_GETURL],
    type_spec: Some(TypeSpec::ParseResult),
};

// =============================================================================
// Module definitions
// =============================================================================

/// urllib.parse module - URL parsing utilities
pub static URLLIB_PARSE_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "urllib.parse",
    functions: &[URLPARSE, URLENCODE, QUOTE, UNQUOTE, URLJOIN, PARSE_QS],
    attrs: &[],
    constants: &[],
    classes: &[PARSE_RESULT_CLASS],
    exceptions: &[],
    submodules: &[],
};

// =============================================================================
// urllib.request module functions
// =============================================================================

/// urllib.request.urlopen(url_or_request, data=None, timeout=30.0)
///
/// First argument accepts either a `str` (URL) or a `Request` object — this
/// matches the standard CPython signature so code targeting pyaot runs
/// unchanged on any Python interpreter. Returns an HTTPResponse.
pub static URLOPEN: StdlibFunctionDef = StdlibFunctionDef {
    name: "urlopen",
    runtime_name: "rt_urlopen",
    params: &[
        // `TypeSpec::Any` lets both `str` and `Request` bind; rt_urlopen
        // inspects the runtime type_tag and dispatches.
        ParamDef::required("url", TypeSpec::Any),
        ParamDef::optional("data", TypeSpec::Optional(&TypeSpec::Bytes)),
        ParamDef::optional_with_default("timeout", TypeSpec::Float, ConstValue::Float(30.0)),
    ],
    return_type: TypeSpec::HttpResponse,
    min_args: 1,
    max_args: 3,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_urlopen", &[P_I64, P_I64, P_F64], Some(R_I64), false),
};

/// `urllib.request.Request(url, data=None, headers=None, method=None)`
/// — standard CPython constructor that bundles the pieces of a request for
/// later dispatch via `urlopen(Request(...))`.
pub static REQUEST_INIT: StdlibFunctionDef = StdlibFunctionDef {
    name: "Request",
    runtime_name: "rt_make_request",
    params: &[
        ParamDef::required("url", TypeSpec::Str),
        ParamDef::optional("data", TypeSpec::Optional(&TypeSpec::Bytes)),
        ParamDef::optional("headers", TypeSpec::Optional(&TYPE_DICT_STR_STR)),
        ParamDef::optional("method", TypeSpec::Optional(&TypeSpec::Str)),
    ],
    return_type: TypeSpec::Request,
    min_args: 1,
    max_args: 4,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_make_request",
        &[P_I64, P_I64, P_I64, P_I64],
        Some(R_I64),
        false,
    ),
};

// =============================================================================
// HTTPResponse class methods (field accessors are defined in object_types.rs)
// =============================================================================

/// HTTPResponse.read() - Read the response body
pub static HTTP_RESPONSE_READ: StdlibMethodDef = StdlibMethodDef {
    name: "read",
    runtime_name: "rt_http_response_read",
    params: &[],
    return_type: TypeSpec::Bytes,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_http_response_read", &[P_I64], Some(R_I64), false),
};

/// HTTPResponse.json() - Parse body as JSON (requests-library convention).
pub static HTTP_RESPONSE_JSON: StdlibMethodDef = StdlibMethodDef {
    name: "json",
    runtime_name: "rt_http_response_json",
    params: &[],
    return_type: TypeSpec::Any,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_http_response_json", &[P_I64], Some(R_I64), false),
};

/// HTTPResponse.geturl() - Get the URL of the response
pub static HTTP_RESPONSE_GETURL: StdlibMethodDef = StdlibMethodDef {
    name: "geturl",
    runtime_name: "rt_http_response_geturl",
    params: &[],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_http_response_geturl", &[P_I64], Some(R_I64), false),
};

/// HTTPResponse.getcode() - Get the HTTP status code
pub static HTTP_RESPONSE_GETCODE: StdlibMethodDef = StdlibMethodDef {
    name: "getcode",
    runtime_name: "rt_http_response_getcode",
    params: &[],
    return_type: TypeSpec::Int,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::new("rt_http_response_getcode", &[P_I64], Some(R_I64), false),
};

/// urllib.request module - HTTP request utilities.
///
/// Note: `HTTPResponse` lives in `http.client` to match CPython exactly.
/// `urlopen` still returns one; users import it from `http.client`.
pub static URLLIB_REQUEST_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "urllib.request",
    functions: &[URLOPEN, REQUEST_INIT],
    attrs: &[],
    constants: &[],
    classes: &[],
    exceptions: &[],
    submodules: &[],
};

/// urllib module (parent module, contains submodules)
pub static URLLIB_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "urllib",
    functions: &[],
    attrs: &[],
    constants: &[],
    classes: &[],
    exceptions: &[],
    submodules: &[
        &URLLIB_PARSE_MODULE,
        &URLLIB_REQUEST_MODULE,
        &crate::modules::urllib_error::URLLIB_ERROR_MODULE,
    ],
};
