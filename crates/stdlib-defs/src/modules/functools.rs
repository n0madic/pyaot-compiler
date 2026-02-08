//! functools module definition
//!
//! Note: `reduce` is handled as a Builtin (like map/filter) because it takes
//! a callable argument. The definition here exists for `from functools import reduce`
//! recognition; the actual lowering intercepts it before StdlibCall dispatch.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};

static REDUCE: StdlibFunctionDef = StdlibFunctionDef {
    name: "reduce",
    runtime_name: "rt_reduce",
    params: &[
        ParamDef::required("function", TypeSpec::Any),
        ParamDef::required("iterable", TypeSpec::Any),
        ParamDef::optional("initial", TypeSpec::Any),
    ],
    return_type: TypeSpec::Any,
    min_args: 2,
    max_args: 3,
    hints: LoweringHints::DEFAULT,
};

pub static FUNCTOOLS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "functools",
    functions: &[REDUCE],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
