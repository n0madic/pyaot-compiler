//! json module definition
//!
//! Provides JSON encoding and decoding functions.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// json.dumps function - auto-boxes primitives for Any param
pub static JSON_DUMPS: StdlibFunctionDef = StdlibFunctionDef {
    name: "dumps",
    runtime_name: "rt_json_dumps",
    params: &[ParamDef::required("obj", TypeSpec::Any)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT, // Auto-box primitives for Any
    codegen: RuntimeFuncDef::new("rt_json_dumps", &[P_I64], Some(R_I64), false),
};

/// json.loads function
pub static JSON_LOADS: StdlibFunctionDef = StdlibFunctionDef {
    name: "loads",
    runtime_name: "rt_json_loads",
    params: &[ParamDef::required("s", TypeSpec::Str)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Str directly
    codegen: RuntimeFuncDef::new("rt_json_loads", &[P_I64], Some(R_I64), false),
};

/// json.dump function - auto-boxes primitives for Any param
pub static JSON_DUMP: StdlibFunctionDef = StdlibFunctionDef {
    name: "dump",
    runtime_name: "rt_json_dump",
    params: &[
        ParamDef::required("obj", TypeSpec::Any),
        ParamDef::required("fp", TypeSpec::File),
    ],
    return_type: TypeSpec::None,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::DEFAULT, // Auto-box primitives for Any
    codegen: RuntimeFuncDef::void("rt_json_dump", &[P_I64, P_I64]),
};

/// json.load function
pub static JSON_LOAD: StdlibFunctionDef = StdlibFunctionDef {
    name: "load",
    runtime_name: "rt_json_load",
    params: &[ParamDef::required("fp", TypeSpec::File)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX, // Takes File directly
    codegen: RuntimeFuncDef::new("rt_json_load", &[P_I64], Some(R_I64), false),
};

/// json module definition
pub static JSON_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "json",
    functions: &[JSON_DUMPS, JSON_LOADS, JSON_DUMP, JSON_LOAD],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
