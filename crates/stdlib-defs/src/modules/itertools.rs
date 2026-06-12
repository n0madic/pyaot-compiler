//! itertools module definition
//!
//! `chain` rides the `variadic_to_list` lowering: every iterable argument is
//! collected into one list passed as the single runtime arg, and the runtime
//! (`rt_chain_new`) lazily `iter()`-wraps each element. `islice` has dedicated
//! lowering (`lower_islice`) that `iter()`-wraps the iterable and resolves
//! start/stop/step from the argument count. Both are reachable as `from
//! itertools import …` or `itertools.…` — the definitions here are the
//! recognition + codegen seam.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

static CHAIN: StdlibFunctionDef = StdlibFunctionDef {
    name: "chain",
    runtime_name: "rt_chain_new",
    // Variadic: all iterables fold into one list (the single runtime arg). The
    // runtime reads its length from the list and `iter()`-wraps each element, so
    // `chain([1,2],[3,4])`, `chain(gen_a(), gen_b())`, and `chain()` all work.
    params: &[ParamDef::variadic("iterables", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 0,
    max_args: usize::MAX,
    hints: LoweringHints::VARIADIC_TO_LIST,
    codegen: RuntimeFuncDef::new("rt_chain_new", &[P_I64], Some(R_I64), false),
};

// islice has dedicated lowering (`lower_islice` in the lowering crate): it
// `iter()`-wraps the iterable and resolves start/stop/step from the provided
// argument count (a lone numeric arg is the STOP, start defaults to 0, step to
// 1) — the generic positional path can express neither. The `params` below are
// the Python-level surface; the runtime ABI is `rt_islice_new(iter,start,stop,
// step)`, all four supplied by `lower_islice`.
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
    hints: LoweringHints::SLICE_ITERATOR,
    codegen: RuntimeFuncDef::new(
        "rt_islice_new",
        &[P_I64, P_I64, P_I64, P_I64],
        Some(R_I64),
        false,
    ),
};

pub static ITERTOOLS_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "itertools",
    functions: &[CHAIN, ISLICE],
    attrs: &[],
    constants: &[],
    classes: &[],
    exceptions: &[],
    submodules: &[],
};
