//! math module definition
//!
//! Provides mathematical functions and constants.

use crate::types::{
    ConstValue, LoweringHints, ParamDef, StdlibConstDef, StdlibFunctionDef, StdlibModuleDef,
    TypeSpec,
};

/// math.pi constant
pub static MATH_PI: StdlibConstDef = StdlibConstDef {
    name: "pi",
    value: ConstValue::Float(std::f64::consts::PI),
    ty: TypeSpec::Float,
};

/// math.e constant
pub static MATH_E: StdlibConstDef = StdlibConstDef {
    name: "e",
    value: ConstValue::Float(std::f64::consts::E),
    ty: TypeSpec::Float,
};

/// math.tau constant (2 * pi)
pub static MATH_TAU: StdlibConstDef = StdlibConstDef {
    name: "tau",
    value: ConstValue::Float(std::f64::consts::TAU),
    ty: TypeSpec::Float,
};

/// math.inf constant (positive infinity)
pub static MATH_INF: StdlibConstDef = StdlibConstDef {
    name: "inf",
    value: ConstValue::Float(f64::INFINITY),
    ty: TypeSpec::Float,
};

/// math.nan constant (not a number)
pub static MATH_NAN: StdlibConstDef = StdlibConstDef {
    name: "nan",
    value: ConstValue::Float(f64::NAN),
    ty: TypeSpec::Float,
};

/// math.sqrt(x) - square root
pub static MATH_SQRT: StdlibFunctionDef = StdlibFunctionDef {
    name: "sqrt",
    runtime_name: "rt_math_sqrt",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.sin(x) - sine of x (in radians)
pub static MATH_SIN: StdlibFunctionDef = StdlibFunctionDef {
    name: "sin",
    runtime_name: "rt_math_sin",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.cos(x) - cosine of x (in radians)
pub static MATH_COS: StdlibFunctionDef = StdlibFunctionDef {
    name: "cos",
    runtime_name: "rt_math_cos",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.tan(x) - tangent of x (in radians)
pub static MATH_TAN: StdlibFunctionDef = StdlibFunctionDef {
    name: "tan",
    runtime_name: "rt_math_tan",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.ceil(x) - smallest integer >= x
pub static MATH_CEIL: StdlibFunctionDef = StdlibFunctionDef {
    name: "ceil",
    runtime_name: "rt_math_ceil",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.floor(x) - largest integer <= x
pub static MATH_FLOOR: StdlibFunctionDef = StdlibFunctionDef {
    name: "floor",
    runtime_name: "rt_math_floor",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.factorial(n) - n! (factorial)
pub static MATH_FACTORIAL: StdlibFunctionDef = StdlibFunctionDef {
    name: "factorial",
    runtime_name: "rt_math_factorial",
    params: &[ParamDef::required("n", TypeSpec::Int)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.log(x[, base]) - logarithm (natural log when base is omitted)
pub static MATH_LOG: StdlibFunctionDef = StdlibFunctionDef {
    name: "log",
    runtime_name: "rt_math_log",
    params: &[
        ParamDef::required("x", TypeSpec::Float),
        ParamDef::optional_with_default("base", TypeSpec::Float, ConstValue::Float(f64::NAN)),
    ],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.log2(x) - logarithm base 2
pub static MATH_LOG2: StdlibFunctionDef = StdlibFunctionDef {
    name: "log2",
    runtime_name: "rt_math_log2",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.log10(x) - logarithm base 10
pub static MATH_LOG10: StdlibFunctionDef = StdlibFunctionDef {
    name: "log10",
    runtime_name: "rt_math_log10",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.exp(x) - exponential function e^x
pub static MATH_EXP: StdlibFunctionDef = StdlibFunctionDef {
    name: "exp",
    runtime_name: "rt_math_exp",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.asin(x) - arc sine (result in radians)
pub static MATH_ASIN: StdlibFunctionDef = StdlibFunctionDef {
    name: "asin",
    runtime_name: "rt_math_asin",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.acos(x) - arc cosine (result in radians)
pub static MATH_ACOS: StdlibFunctionDef = StdlibFunctionDef {
    name: "acos",
    runtime_name: "rt_math_acos",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.atan(x) - arc tangent (result in radians)
pub static MATH_ATAN: StdlibFunctionDef = StdlibFunctionDef {
    name: "atan",
    runtime_name: "rt_math_atan",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.sinh(x) - hyperbolic sine
pub static MATH_SINH: StdlibFunctionDef = StdlibFunctionDef {
    name: "sinh",
    runtime_name: "rt_math_sinh",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.cosh(x) - hyperbolic cosine
pub static MATH_COSH: StdlibFunctionDef = StdlibFunctionDef {
    name: "cosh",
    runtime_name: "rt_math_cosh",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.tanh(x) - hyperbolic tangent
pub static MATH_TANH: StdlibFunctionDef = StdlibFunctionDef {
    name: "tanh",
    runtime_name: "rt_math_tanh",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.fabs(x) - absolute value as float
pub static MATH_FABS: StdlibFunctionDef = StdlibFunctionDef {
    name: "fabs",
    runtime_name: "rt_math_fabs",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.degrees(x) - convert radians to degrees
pub static MATH_DEGREES: StdlibFunctionDef = StdlibFunctionDef {
    name: "degrees",
    runtime_name: "rt_math_degrees",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.radians(x) - convert degrees to radians
pub static MATH_RADIANS: StdlibFunctionDef = StdlibFunctionDef {
    name: "radians",
    runtime_name: "rt_math_radians",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Float,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.trunc(x) - truncate to integer
pub static MATH_TRUNC: StdlibFunctionDef = StdlibFunctionDef {
    name: "trunc",
    runtime_name: "rt_math_trunc",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.isnan(x) - test if x is NaN
pub static MATH_ISNAN: StdlibFunctionDef = StdlibFunctionDef {
    name: "isnan",
    runtime_name: "rt_math_isnan",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.isinf(x) - test if x is infinite
pub static MATH_ISINF: StdlibFunctionDef = StdlibFunctionDef {
    name: "isinf",
    runtime_name: "rt_math_isinf",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.isfinite(x) - test if x is finite
pub static MATH_ISFINITE: StdlibFunctionDef = StdlibFunctionDef {
    name: "isfinite",
    runtime_name: "rt_math_isfinite",
    params: &[ParamDef::required("x", TypeSpec::Float)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.atan2(y, x) - arc tangent of y/x (result in radians)
pub static MATH_ATAN2: StdlibFunctionDef = StdlibFunctionDef {
    name: "atan2",
    runtime_name: "rt_math_atan2",
    params: &[
        ParamDef::required("y", TypeSpec::Float),
        ParamDef::required("x", TypeSpec::Float),
    ],
    return_type: TypeSpec::Float,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.fmod(x, y) - floating point remainder
pub static MATH_FMOD: StdlibFunctionDef = StdlibFunctionDef {
    name: "fmod",
    runtime_name: "rt_math_fmod",
    params: &[
        ParamDef::required("x", TypeSpec::Float),
        ParamDef::required("y", TypeSpec::Float),
    ],
    return_type: TypeSpec::Float,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.copysign(x, y) - magnitude of x with sign of y
pub static MATH_COPYSIGN: StdlibFunctionDef = StdlibFunctionDef {
    name: "copysign",
    runtime_name: "rt_math_copysign",
    params: &[
        ParamDef::required("x", TypeSpec::Float),
        ParamDef::required("y", TypeSpec::Float),
    ],
    return_type: TypeSpec::Float,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.hypot(x, y) - Euclidean distance sqrt(x*x + y*y)
pub static MATH_HYPOT: StdlibFunctionDef = StdlibFunctionDef {
    name: "hypot",
    runtime_name: "rt_math_hypot",
    params: &[
        ParamDef::required("x", TypeSpec::Float),
        ParamDef::required("y", TypeSpec::Float),
    ],
    return_type: TypeSpec::Float,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.pow(x, y) - x raised to the power y
pub static MATH_POW: StdlibFunctionDef = StdlibFunctionDef {
    name: "pow",
    runtime_name: "rt_math_pow",
    params: &[
        ParamDef::required("x", TypeSpec::Float),
        ParamDef::required("y", TypeSpec::Float),
    ],
    return_type: TypeSpec::Float,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.gcd(a, b) - greatest common divisor
pub static MATH_GCD: StdlibFunctionDef = StdlibFunctionDef {
    name: "gcd",
    runtime_name: "rt_math_gcd",
    params: &[
        ParamDef::required("a", TypeSpec::Int),
        ParamDef::required("b", TypeSpec::Int),
    ],
    return_type: TypeSpec::Int,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.lcm(a, b) - least common multiple
pub static MATH_LCM: StdlibFunctionDef = StdlibFunctionDef {
    name: "lcm",
    runtime_name: "rt_math_lcm",
    params: &[
        ParamDef::required("a", TypeSpec::Int),
        ParamDef::required("b", TypeSpec::Int),
    ],
    return_type: TypeSpec::Int,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.comb(n, k) - binomial coefficient (n choose k)
pub static MATH_COMB: StdlibFunctionDef = StdlibFunctionDef {
    name: "comb",
    runtime_name: "rt_math_comb",
    params: &[
        ParamDef::required("n", TypeSpec::Int),
        ParamDef::required("k", TypeSpec::Int),
    ],
    return_type: TypeSpec::Int,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math.perm(n, k) - permutation count (n! / (n-k)!)
pub static MATH_PERM: StdlibFunctionDef = StdlibFunctionDef {
    name: "perm",
    runtime_name: "rt_math_perm",
    params: &[
        ParamDef::required("n", TypeSpec::Int),
        ParamDef::required("k", TypeSpec::Int),
    ],
    return_type: TypeSpec::Int,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
};

/// math module definition
pub static MATH_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "math",
    functions: &[
        MATH_SQRT,
        MATH_SIN,
        MATH_COS,
        MATH_TAN,
        MATH_CEIL,
        MATH_FLOOR,
        MATH_FACTORIAL,
        MATH_LOG,
        MATH_LOG2,
        MATH_LOG10,
        MATH_EXP,
        MATH_ASIN,
        MATH_ACOS,
        MATH_ATAN,
        MATH_SINH,
        MATH_COSH,
        MATH_TANH,
        MATH_FABS,
        MATH_DEGREES,
        MATH_RADIANS,
        MATH_TRUNC,
        MATH_ISNAN,
        MATH_ISINF,
        MATH_ISFINITE,
        MATH_ATAN2,
        MATH_FMOD,
        MATH_COPYSIGN,
        MATH_HYPOT,
        MATH_POW,
        MATH_GCD,
        MATH_LCM,
        MATH_COMB,
        MATH_PERM,
    ],
    attrs: &[],
    constants: &[MATH_PI, MATH_E, MATH_TAU, MATH_INF, MATH_NAN],
    classes: &[],
    submodules: &[],
};
