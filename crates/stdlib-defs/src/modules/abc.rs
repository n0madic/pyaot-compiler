//! abc module definition
//!
//! Abstract Base Classes support module.
//! The @abstractmethod decorator is recognized at parse time,
//! so this function serves mainly to allow imports.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// abc.abstractmethod function
/// This is a no-op decorator that returns its argument unchanged.
/// The actual abstract method handling is done at parse time.
pub static ABC_ABSTRACTMETHOD: StdlibFunctionDef = StdlibFunctionDef {
    name: "abstractmethod",
    runtime_name: "rt_abc_abstractmethod",
    params: &[ParamDef::required("funcobj", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT, // Auto-box for Any type
    codegen: RuntimeFuncDef::new("rt_abc_abstractmethod", &[P_I64], Some(R_I64), false),
};

/// abc module definition
pub static ABC_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "abc",
    functions: &[ABC_ABSTRACTMETHOD],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
