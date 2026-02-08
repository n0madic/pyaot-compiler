//! sys module definition
//!
//! Provides access to system-specific parameters and functions.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibAttrDef, StdlibFunctionDef, StdlibModuleDef,
    TypeSpec, TYPE_STR,
};

/// sys.argv attribute
pub static SYS_ARGV: StdlibAttrDef = StdlibAttrDef {
    name: "argv",
    runtime_getter: "rt_sys_get_argv",
    ty: TypeSpec::List(&TYPE_STR),
    writable: false,
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
