//! Declarative descriptors for bare scalar/value builtins.
//!
//! These are NOT module members — they are bare names (`id`, `round`, `bin`,
//! `hex`, `oct`) the frontend dispatch references directly by `&'static`
//! reference (no [`StdlibModuleDef`](crate::types::StdlibModuleDef) registry).
//! Each lowers to [`HirExprKind::CallRuntime`] against its `codegen`
//! descriptor; the generic runtime-call handler resolves the `runtime_name`
//! → `FuncId` on demand, so a new `rt_*` is picked up with no codegen edit.
//!
//! The arithmetic / iterable-consuming builtins (`pow`, `divmod`, `all`,
//! `any`) are pure frontend desugars (binop / staged tuple / iterator loop) and
//! need no descriptor here.

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, TypeSpec};
use pyaot_core_defs::RuntimeFuncDef;

/// `id(obj)` — the object's identity (its address) as an int. Wraps the
/// existing `rt_id_obj` (`unary_to_i64`: a tagged receiver → a raw `i64`). The
/// result is a number (`Repr::Raw(I64)`), never handed back as a GC root or
/// dereferenced (PITFALLS B5/B12); bit-identity is consistent with the §2
/// `is`/`rt_is` path.
pub static BUILTIN_ID: StdlibFunctionDef = StdlibFunctionDef {
    name: "id",
    runtime_name: "rt_id_obj",
    params: &[ParamDef::required("obj", TypeSpec::Any)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::unary_to_i64("rt_id_obj"),
};

/// `round(x[, ndigits])` — banker's rounding (PITFALLS B1). The PRESENCE of
/// `ndigits` selects the result type (int when absent, float when present), so
/// the node stays `Dyn`/Tagged (always-correct, Principle 2). The absent /
/// explicit-`None` second argument lowers to the null sentinel, which the
/// runtime reads as "no ndigits".
pub static BUILTIN_ROUND: StdlibFunctionDef = StdlibFunctionDef {
    name: "round",
    runtime_name: "rt_builtin_round",
    params: &[
        ParamDef::required("x", TypeSpec::Any),
        ParamDef::optional("ndigits", TypeSpec::Any),
    ],
    return_type: TypeSpec::Any,
    min_args: 1,
    max_args: 2,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::ptr_binary("rt_builtin_round"),
};

/// `bin(n)` — binary string. The receiver travels TAGGED (`Any`), never a raw
/// `i64`, so a heap `BigInt` (`bin(2 ** 100)`) formats correctly (PITFALLS B16).
pub static BUILTIN_BIN: StdlibFunctionDef = StdlibFunctionDef {
    name: "bin",
    runtime_name: "rt_builtin_bin",
    params: &[ParamDef::required("n", TypeSpec::Any)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::ptr_unary("rt_builtin_bin"),
};

/// `hex(n)` — hexadecimal string. Tagged, bignum-aware (PITFALLS B16).
pub static BUILTIN_HEX: StdlibFunctionDef = StdlibFunctionDef {
    name: "hex",
    runtime_name: "rt_builtin_hex",
    params: &[ParamDef::required("n", TypeSpec::Any)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::ptr_unary("rt_builtin_hex"),
};

/// `oct(n)` — octal string. Tagged, bignum-aware (PITFALLS B16).
pub static BUILTIN_OCT: StdlibFunctionDef = StdlibFunctionDef {
    name: "oct",
    runtime_name: "rt_builtin_oct",
    params: &[ParamDef::required("n", TypeSpec::Any)],
    return_type: TypeSpec::Str,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::ptr_unary("rt_builtin_oct"),
};
