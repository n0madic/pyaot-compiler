//! random module definition
//!
//! Provides pseudo-random number generation functions.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibFunctionDef, StdlibModuleDef, TypeSpec,
};

/// random.random() -> float in [0.0, 1.0)
pub static RANDOM_RANDOM: StdlibFunctionDef = StdlibFunctionDef {
    name: "random",
    runtime_name: "rt_random_random",
    params: &[],
    return_type: TypeSpec::Float,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
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
};

/// random.choice(seq) -> element
pub static RANDOM_CHOICE: StdlibFunctionDef = StdlibFunctionDef {
    name: "choice",
    runtime_name: "rt_random_choice",
    params: &[ParamDef::required("seq", TypeSpec::Any)],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
};

/// random.shuffle(seq) -> None
pub static RANDOM_SHUFFLE: StdlibFunctionDef = StdlibFunctionDef {
    name: "shuffle",
    runtime_name: "rt_random_shuffle",
    params: &[ParamDef::required("seq", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
};

/// random.seed(n) -> None
pub static RANDOM_SEED: StdlibFunctionDef = StdlibFunctionDef {
    name: "seed",
    runtime_name: "rt_random_seed",
    params: &[ParamDef::optional_with_default(
        "n",
        TypeSpec::Int,
        ConstValue::Int(0),
    )],
    return_type: TypeSpec::None,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
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
    hints: LoweringHints::DEFAULT,
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
    ],
    attrs: &[],
    constants: &[],
    classes: &[],
    submodules: &[],
};
