//! copy module definition
//!
//! Provides shallow and deep copy operations.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};

/// copy.copy(obj) -> shallow copy
pub static COPY_COPY: StdlibFunctionDef = StdlibFunctionDef {
    name: "copy",
    runtime_name: "rt_copy_copy",
    params: &[ParamDef::required("obj", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
};

/// copy.deepcopy(obj) -> deep copy
pub static COPY_DEEPCOPY: StdlibFunctionDef = StdlibFunctionDef {
    name: "deepcopy",
    runtime_name: "rt_copy_deepcopy",
    params: &[ParamDef::required("obj", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
};

/// copy module definition
pub static COPY_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "copy",
    functions: &[COPY_COPY, COPY_DEEPCOPY],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
