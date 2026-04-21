//! random module definition
//!
//! Provides pseudo-random number generation functions.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec,
};
#[allow(unused_imports)]
use pyaot_core_defs::runtime_func_def::{P_F64, P_I64, P_I8, R_F64, R_I64, R_I8};
use pyaot_core_defs::RuntimeFuncDef;

/// random.random() -> float in [0.0, 1.0)
pub static RANDOM_RANDOM: StdlibFunctionDef = StdlibFunctionDef {
    name: "random",
    runtime_name: "rt_random_random",
    params: &[],
    return_type: TypeSpec::Float,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_random_random", &[], Some(R_F64), false),
};

/// random.randint(a, b) -> int in [a, b]
pub static RANDOM_RANDINT: StdlibFunctionDef = StdlibFunctionDef {
    name: "randint",
    runtime_name: "rt_random_randint",
    params: &[
        ParamDef::required("a", TypeSpec::Int),
        ParamDef::required("b", TypeSpec::Int),
    ],
    return_type: TypeSpec::Int,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_random_randint", &[P_I64, P_I64], Some(R_I64), false),
};

/// random.choice(seq) -> element
pub static RANDOM_CHOICE: StdlibFunctionDef = StdlibFunctionDef {
    name: "choice",
    runtime_name: "rt_random_choice",
    params: &[ParamDef::required("seq", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_random_choice", &[P_I64], Some(R_I64), false),
};

/// random.shuffle(seq) -> None
pub static RANDOM_SHUFFLE: StdlibFunctionDef = StdlibFunctionDef {
    name: "shuffle",
    runtime_name: "rt_random_shuffle",
    params: &[ParamDef::required("seq", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::void("rt_random_shuffle", &[P_I64]),
};

/// random.seed(n) -> None
/// The runtime receives the user-supplied argument count as a second i64 parameter.
/// When arg_count == 0, the runtime uses system entropy regardless of the seed value.
/// This avoids the i64::MIN sentinel collision problem.
pub static RANDOM_SEED: StdlibFunctionDef = StdlibFunctionDef {
    name: "seed",
    runtime_name: "rt_random_seed",
    params: &[ParamDef::optional_with_default(
        "n",
        TypeSpec::Int,
        ConstValue::Int(0), // placeholder; ignored when arg_count == 0
    )],
    return_type: TypeSpec::None,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX_PASS_ARG_COUNT,
    // pass_arg_count: lowering appends an extra PI64 for the arg count
    codegen: RuntimeFuncDef::void("rt_random_seed", &[P_I64, P_I64]),
};

/// random.uniform(a, b) -> float
pub static RANDOM_UNIFORM: StdlibFunctionDef = StdlibFunctionDef {
    name: "uniform",
    runtime_name: "rt_random_uniform",
    params: &[
        ParamDef::required("a", TypeSpec::Float),
        ParamDef::required("b", TypeSpec::Float),
    ],
    return_type: TypeSpec::Float,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_random_uniform", &[P_F64, P_F64], Some(R_F64), false),
};

/// random.randrange(start, stop?, step?) -> int
pub static RANDOM_RANDRANGE: StdlibFunctionDef = StdlibFunctionDef {
    name: "randrange",
    runtime_name: "rt_random_randrange",
    params: &[
        ParamDef::required("start", TypeSpec::Int),
        ParamDef::optional_with_default("stop", TypeSpec::Int, ConstValue::Int(i64::MIN)),
        ParamDef::optional_with_default("step", TypeSpec::Int, ConstValue::Int(0)),
    ],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 3,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_random_randrange",
        &[P_I64, P_I64, P_I64],
        Some(R_I64),
        false,
    ),
};

/// random.sample(population, k) -> list
pub static RANDOM_SAMPLE: StdlibFunctionDef = StdlibFunctionDef {
    name: "sample",
    runtime_name: "rt_random_sample",
    params: &[
        ParamDef::required("population", TypeSpec::Any),
        ParamDef::required("k", TypeSpec::Int),
    ],
    return_type: TypeSpec::Any,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_random_sample", &[P_I64, P_I64], Some(R_I64), false),
};

/// random.gauss(mu, sigma) -> float
pub static RANDOM_GAUSS: StdlibFunctionDef = StdlibFunctionDef {
    name: "gauss",
    runtime_name: "rt_random_gauss",
    params: &[
        ParamDef::required("mu", TypeSpec::Float),
        ParamDef::required("sigma", TypeSpec::Float),
    ],
    return_type: TypeSpec::Float,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_random_gauss", &[P_F64, P_F64], Some(R_F64), false),
};

/// random.choices(population, weights=None, k=1) -> list
/// When weights is None (omitted), uses uniform distribution
pub static RANDOM_CHOICES: StdlibFunctionDef = StdlibFunctionDef {
    name: "choices",
    runtime_name: "rt_random_choices",
    params: &[
        ParamDef::required("population", TypeSpec::Any),
        ParamDef::optional("weights", TypeSpec::Any),
        ParamDef::optional_with_default("k", TypeSpec::Int, ConstValue::Int(1)),
    ],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 3,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new(
        "rt_random_choices",
        &[P_I64, P_I64, P_I64],
        Some(R_I64),
        false,
    ),
};

/// random module definition
pub static RANDOM_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "random",
    functions: &[
        RANDOM_RANDOM,
        RANDOM_RANDINT,
        RANDOM_CHOICE,
        RANDOM_SHUFFLE,
        RANDOM_SEED,
        RANDOM_UNIFORM,
        RANDOM_RANDRANGE,
        RANDOM_SAMPLE,
        RANDOM_GAUSS,
        RANDOM_CHOICES,
    ],
    attrs: &[],
    constants: &[],
    classes: &[],
    exceptions: &[],
    submodules: &[],
};
