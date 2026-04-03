//! sys module definition
//!
//! Provides access to system-specific parameters and functions.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibAttrDef, StdlibFunctionDef, StdlibModuleDef,
    TypeSpec, TYPE_STR,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// sys.argv attribute
pub static SYS_ARGV: StdlibAttrDef = StdlibAttrDef {
    name: "argv",
    runtime_getter: "rt_sys_get_argv",
    ty: TypeSpec::List(&TYPE_STR),
    writable: false,
    codegen: RuntimeFuncDef::new("rt_sys_get_argv", &[], Some(R_I64), false),
};

/// sys.exit function
pub static SYS_EXIT: StdlibFunctionDef = StdlibFunctionDef {
    name: "exit",
    runtime_name: "rt_sys_exit",
    params: &[ParamDef::optional_with_default(
        "code",
        TypeSpec::Int,
        ConstValue::Int(0),
    )],
    return_type: TypeSpec::None, // Never returns, but type is None
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Int directly
    codegen: RuntimeFuncDef::void("rt_sys_exit", &[P_I64]),
};

/// sys.intern function
pub static SYS_INTERN: StdlibFunctionDef = StdlibFunctionDef {
    name: "intern",
    runtime_name: "rt_sys_intern",
    params: &[ParamDef::required("string", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX, // Takes Str directly
    codegen: RuntimeFuncDef::new("rt_sys_intern", &[P_I64], Some(R_I64), false),
};

/// sys module definition
pub static SYS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "sys",
    functions: &[SYS_EXIT, SYS_INTERN],
    attrs: &[SYS_ARGV],
    constants: &[],
    classes: &[],
    submodules: &[],
};
