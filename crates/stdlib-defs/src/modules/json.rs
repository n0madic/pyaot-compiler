//! json module definition
//!
//! Provides JSON encoding and decoding functions.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};

/// json.dumps function - auto-boxes primitives for Any param
pub static JSON_DUMPS: StdlibFunctionDef = StdlibFunctionDef {
    name: "dumps",
    runtime_name: "rt_json_dumps",
    params: &[ParamDef::required("obj", TypeSpec::Any)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT, // Auto-box primitives for Any
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
