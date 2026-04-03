//! base64 module definition
//!
//! Provides base64 encoding/decoding functions.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// base64.b64encode(data) -> bytes
pub static BASE64_B64ENCODE: StdlibFunctionDef = StdlibFunctionDef {
    name: "b64encode",
    runtime_name: "rt_base64_b64encode",
    params: &[ParamDef::required("data", TypeSpec::Bytes)],
    return_type: TypeSpec::Bytes,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_base64_b64encode", &[P_I64], Some(R_I64), false),
};

/// base64.b64decode(data) -> bytes
pub static BASE64_B64DECODE: StdlibFunctionDef = StdlibFunctionDef {
    name: "b64decode",
    runtime_name: "rt_base64_b64decode",
    params: &[ParamDef::required("data", TypeSpec::Any)],
    return_type: TypeSpec::Bytes,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_base64_b64decode", &[P_I64], Some(R_I64), false),
};

/// base64.urlsafe_b64encode(data) -> bytes
pub static BASE64_URLSAFE_B64ENCODE: StdlibFunctionDef = StdlibFunctionDef {
    name: "urlsafe_b64encode",
    runtime_name: "rt_base64_urlsafe_b64encode",
    params: &[ParamDef::required("data", TypeSpec::Bytes)],
    return_type: TypeSpec::Bytes,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_base64_urlsafe_b64encode", &[P_I64], Some(R_I64), false),
};

/// base64.urlsafe_b64decode(data) -> bytes
pub static BASE64_URLSAFE_B64DECODE: StdlibFunctionDef = StdlibFunctionDef {
    name: "urlsafe_b64decode",
    runtime_name: "rt_base64_urlsafe_b64decode",
    params: &[ParamDef::required("data", TypeSpec::Any)],
    return_type: TypeSpec::Bytes,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_base64_urlsafe_b64decode", &[P_I64], Some(R_I64), false),
};

/// base64 module definition
pub static BASE64_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "base64",
    functions: &[
        BASE64_B64ENCODE,
        BASE64_B64DECODE,
        BASE64_URLSAFE_B64ENCODE,
        BASE64_URLSAFE_B64DECODE,
    ],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
