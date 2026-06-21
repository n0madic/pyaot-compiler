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

use crate::types::{LoweringHints, ParamDef, StdlibFunctionDef, StdlibMethodDef, TypeSpec};
use pyaot_core_defs::runtime_func_def::{P_I64, R_I64};
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

/// `input([prompt])` — read one line from stdin (the trailing newline stripped),
/// first writing `prompt` (if given) with no newline. Wraps the existing
/// `rt_input` (a tagged `prompt` pointer → a fresh `str`); an absent prompt fills
/// the null-pointer sentinel the runtime reads as "no prompt". Raises `EOFError`
/// at end of input, matching CPython.
pub static BUILTIN_INPUT: StdlibFunctionDef = StdlibFunctionDef {
    name: "input",
    runtime_name: "rt_input",
    params: &[ParamDef::optional("prompt", TypeSpec::Str)],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::ptr_unary("rt_input"),
};

/// `bytes.fromhex(string)` — parse a string of hex digit pairs (spaces allowed)
/// into bytes. The classmethod on the `bytes` type; the frontend routes the
/// `bytes.fromhex(...)` form here. Wraps `rt_bytes_fromhex` (a tagged `str` → a
/// fresh `bytes`).
pub static BYTES_FROMHEX: StdlibFunctionDef = StdlibFunctionDef {
    name: "bytes.fromhex",
    runtime_name: "rt_bytes_fromhex",
    params: &[ParamDef::required("string", TypeSpec::Str)],
    return_type: TypeSpec::Bytes,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::DEFAULT,
    codegen: RuntimeFuncDef::ptr_unary("rt_bytes_fromhex"),
};

// =============================================================================
// frozenset — construction targets + read-only methods
// =============================================================================
// `frozenset` is a bare builtin (no module). The frontend's bare-name intercept
// (`lower_frozenset_construct`) routes the 0-arg / 1-arg forms here, typed
// `RuntimeObject(FrozenSet)` via `TypeSpec::FrozenSet`. Methods resolve through
// the `object_types::FROZENSET` registry.

/// `frozenset()` — empty construction.
pub static FROZENSET_EMPTY: StdlibFunctionDef = StdlibFunctionDef {
    name: "frozenset",
    runtime_name: "rt_make_frozenset_empty",
    params: &[],
    return_type: TypeSpec::FrozenSet,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_frozenset_empty", &[], Some(R_I64), true),
};

/// `frozenset(iterable)` — built from any iterable (normalized to an iterator
/// inside the runtime, like `Counter(iterable)`).
pub static FROZENSET_FROM_ITER: StdlibFunctionDef = StdlibFunctionDef {
    name: "frozenset",
    runtime_name: "rt_make_frozenset_from_iter",
    params: &[ParamDef::required("iterable", TypeSpec::Any)],
    return_type: TypeSpec::FrozenSet,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_frozenset_from_iter", &[P_I64], Some(R_I64), true),
};

/// `fs.union(other)` → frozenset.
pub static FROZENSET_UNION: StdlibMethodDef = StdlibMethodDef {
    name: "union",
    runtime_name: "rt_frozenset_union",
    params: &[ParamDef::required("other", TypeSpec::Any)],
    return_type: TypeSpec::FrozenSet,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::ptr_binary("rt_frozenset_union"),
};

/// `fs.intersection(other)` → frozenset.
pub static FROZENSET_INTERSECTION: StdlibMethodDef = StdlibMethodDef {
    name: "intersection",
    runtime_name: "rt_frozenset_intersection",
    params: &[ParamDef::required("other", TypeSpec::Any)],
    return_type: TypeSpec::FrozenSet,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::ptr_binary("rt_frozenset_intersection"),
};

/// `fs.difference(other)` → frozenset.
pub static FROZENSET_DIFFERENCE: StdlibMethodDef = StdlibMethodDef {
    name: "difference",
    runtime_name: "rt_frozenset_difference",
    params: &[ParamDef::required("other", TypeSpec::Any)],
    return_type: TypeSpec::FrozenSet,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::ptr_binary("rt_frozenset_difference"),
};

/// `fs.symmetric_difference(other)` → frozenset.
pub static FROZENSET_SYMMETRIC_DIFFERENCE: StdlibMethodDef = StdlibMethodDef {
    name: "symmetric_difference",
    runtime_name: "rt_frozenset_symmetric_difference",
    params: &[ParamDef::required("other", TypeSpec::Any)],
    return_type: TypeSpec::FrozenSet,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::ptr_binary("rt_frozenset_symmetric_difference"),
};

/// `fs.copy()` → frozenset.
pub static FROZENSET_COPY: StdlibMethodDef = StdlibMethodDef {
    name: "copy",
    runtime_name: "rt_frozenset_copy",
    params: &[],
    return_type: TypeSpec::FrozenSet,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::ptr_unary("rt_frozenset_copy"),
};

/// `fs.issubset(other)` → bool. Reuses the shared set primitive (set family).
pub static FROZENSET_ISSUBSET: StdlibMethodDef = StdlibMethodDef {
    name: "issubset",
    runtime_name: "rt_set_issubset",
    params: &[ParamDef::required("other", TypeSpec::Any)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i8("rt_set_issubset"),
};

/// `fs.issuperset(other)` → bool.
pub static FROZENSET_ISSUPERSET: StdlibMethodDef = StdlibMethodDef {
    name: "issuperset",
    runtime_name: "rt_set_issuperset",
    params: &[ParamDef::required("other", TypeSpec::Any)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i8("rt_set_issuperset"),
};

/// `fs.isdisjoint(other)` → bool.
pub static FROZENSET_ISDISJOINT: StdlibMethodDef = StdlibMethodDef {
    name: "isdisjoint",
    runtime_name: "rt_set_isdisjoint",
    params: &[ParamDef::required("other", TypeSpec::Any)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i8("rt_set_isdisjoint"),
};

// =============================================================================
// bytearray — construction targets + methods
// =============================================================================
// A bare builtin; the frontend's `lower_bytearray_construct` routes the 0/1/2-arg
// forms here, typed `RuntimeObject(ByteArray)`. The 1-arg form goes through the
// unified runtime dispatcher `rt_make_bytearray` (int → zeros, bytes → copy,
// iterable → elements). Methods resolve via `object_types::BYTEARRAY`.

/// `bytearray()` — empty.
pub static BYTEARRAY_EMPTY: StdlibFunctionDef = StdlibFunctionDef {
    name: "bytearray",
    runtime_name: "rt_make_bytearray_empty",
    params: &[],
    return_type: TypeSpec::ByteArray,
    min_args: 0,
    max_args: 0,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_bytearray_empty", &[], Some(R_I64), true),
};

/// `bytearray(arg)` — the unified 1-arg constructor (runtime dispatch).
pub static BYTEARRAY_NEW: StdlibFunctionDef = StdlibFunctionDef {
    name: "bytearray",
    runtime_name: "rt_make_bytearray",
    params: &[ParamDef::required("source", TypeSpec::Any)],
    return_type: TypeSpec::ByteArray,
    min_args: 1,
    max_args: 1,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_bytearray", &[P_I64], Some(R_I64), true),
};

/// `bytearray(str, encoding)` — UTF-8 bytes of the string.
pub static BYTEARRAY_FROM_STR: StdlibFunctionDef = StdlibFunctionDef {
    name: "bytearray",
    runtime_name: "rt_make_bytearray_from_str",
    params: &[
        ParamDef::required("source", TypeSpec::Str),
        ParamDef::required("encoding", TypeSpec::Str),
    ],
    return_type: TypeSpec::ByteArray,
    min_args: 2,
    max_args: 2,
    hints: LoweringHints::NO_AUTO_BOX,
    codegen: RuntimeFuncDef::new("rt_make_bytearray_from_str", &[P_I64, P_I64], Some(R_I64), true),
};

/// `ba.append(int)` — append one byte (0-255).
pub static BYTEARRAY_APPEND: StdlibMethodDef = StdlibMethodDef {
    name: "append",
    runtime_name: "rt_bytearray_append",
    params: &[ParamDef::required("value", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_bytearray_append", &[P_I64, P_I64]),
};

/// `ba.extend(iterable)` — append each int.
pub static BYTEARRAY_EXTEND: StdlibMethodDef = StdlibMethodDef {
    name: "extend",
    runtime_name: "rt_bytearray_extend",
    params: &[ParamDef::required("iterable", TypeSpec::Any)],
    return_type: TypeSpec::None,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::void("rt_bytearray_extend", &[P_I64, P_I64]),
};

/// `ba.hex()` → str.
pub static BYTEARRAY_HEX: StdlibMethodDef = StdlibMethodDef {
    name: "hex",
    runtime_name: "rt_bytearray_hex",
    params: &[],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 0,
    codegen: RuntimeFuncDef::ptr_unary("rt_bytearray_hex"),
};

/// `ba.decode(encoding="utf-8", errors="strict")` → str.
pub static BYTEARRAY_DECODE: StdlibMethodDef = StdlibMethodDef {
    name: "decode",
    runtime_name: "rt_bytearray_decode",
    params: &[
        ParamDef::optional("encoding", TypeSpec::Str),
        ParamDef::optional("errors", TypeSpec::Str),
    ],
    return_type: TypeSpec::Str,
    min_args: 0,
    max_args: 2,
    codegen: RuntimeFuncDef::ptr_ternary("rt_bytearray_decode"),
};

/// `ba.find(sub)` → int.
pub static BYTEARRAY_FIND: StdlibMethodDef = StdlibMethodDef {
    name: "find",
    runtime_name: "rt_bytearray_find",
    params: &[ParamDef::required("sub", TypeSpec::Any)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i64("rt_bytearray_find"),
};

/// `ba.rfind(sub)` → int.
pub static BYTEARRAY_RFIND: StdlibMethodDef = StdlibMethodDef {
    name: "rfind",
    runtime_name: "rt_bytearray_rfind",
    params: &[ParamDef::required("sub", TypeSpec::Any)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i64("rt_bytearray_rfind"),
};

/// `ba.count(sub)` → int.
pub static BYTEARRAY_COUNT: StdlibMethodDef = StdlibMethodDef {
    name: "count",
    runtime_name: "rt_bytearray_count",
    params: &[ParamDef::required("sub", TypeSpec::Any)],
    return_type: TypeSpec::Int,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i64("rt_bytearray_count"),
};

/// `ba.startswith(prefix)` → bool.
pub static BYTEARRAY_STARTSWITH: StdlibMethodDef = StdlibMethodDef {
    name: "startswith",
    runtime_name: "rt_bytearray_startswith",
    params: &[ParamDef::required("prefix", TypeSpec::Any)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i8("rt_bytearray_startswith"),
};

/// `ba.endswith(suffix)` → bool.
pub static BYTEARRAY_ENDSWITH: StdlibMethodDef = StdlibMethodDef {
    name: "endswith",
    runtime_name: "rt_bytearray_endswith",
    params: &[ParamDef::required("suffix", TypeSpec::Any)],
    return_type: TypeSpec::Bool,
    min_args: 1,
    max_args: 1,
    codegen: RuntimeFuncDef::binary_to_i8("rt_bytearray_endswith"),
};
