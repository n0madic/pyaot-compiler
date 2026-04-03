//! copy module definition
//!
//! Provides shallow and deep copy operations.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// copy.copy(obj) -> shallow copy
pub static COPY_COPY: StdlibFunctionDef = StdlibFunctionDef {
    name: "copy",
    runtime_name: "rt_copy_copy",
    params: &[ParamDef::required("obj", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::new("rt_copy_copy", &[P_I64], Some(R_I64), false),
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
    codegen: RuntimeFuncDef::new("rt_copy_deepcopy", &[P_I64], Some(R_I64), false),
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
