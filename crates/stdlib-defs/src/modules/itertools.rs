//! itertools module definition
//!
//! Note: `chain` and `islice` are handled as Builtins because they need special
//! lowering. The definitions here exist for `from itertools import chain, islice`
//! recognition; the actual lowering intercepts them before StdlibCall dispatch.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};

static CHAIN: StdlibFunctionDef = StdlibFunctionDef {
    name: "chain",
    runtime_name: "rt_chain_new",
    params: &[ParamDef::required("iterables", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 0,
    max_args: 255, // variadic
    hints: LoweringHints::DEFAULT,
};

static ISLICE: StdlibFunctionDef = StdlibFunctionDef {
    name: "islice",
    runtime_name: "rt_islice_new",
    params: &[
        ParamDef::required("iterable", TypeSpec::Any),
        ParamDef::required("start_or_stop", TypeSpec::Int),
        ParamDef::optional("stop", TypeSpec::Int),
        ParamDef::optional("step", TypeSpec::Int),
    ],
    return_type: TypeSpec::Any,
    min_args: 2,
    max_args: 4,
    hints: LoweringHints::DEFAULT,
};

pub static ITERTOOLS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "itertools",
    functions: &[CHAIN, ISLICE],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
