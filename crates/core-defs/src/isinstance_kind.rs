//! Builtin-type `isinstance` kind codes — the single source of truth shared by
//! lowering's `rt_isinstance_builtin` emission and the runtime's tag query.
//!
//! Used only for a gradual (`Dyn`/`Union`) receiver, where the verdict cannot
//! be folded statically and the runtime must inspect the value's tag. A
//! statically-typed receiver still folds at lowering (see
//! `lowering::lower_isinstance_builtin`). The codes match by Python `type`
//! KIND: `isinstance` ignores element types, so `list`/`dict`/`set`/`tuple`
//! match any instance of the container regardless of its element types, and
//! `bool ⊂ int` (a `bool` value satisfies `isinstance(x, int)`).

#![forbid(unsafe_code)]

/// `str`
pub const STR: i64 = 0;
/// `int` (matches `int`, big integers, AND `bool` — `bool ⊂ int`).
pub const INT: i64 = 1;
/// `float`
pub const FLOAT: i64 = 2;
/// `bool`
pub const BOOL: i64 = 3;
/// `bytes`
pub const BYTES: i64 = 4;
/// `list`
pub const LIST: i64 = 5;
/// `dict`
pub const DICT: i64 = 6;
/// `set`
pub const SET: i64 = 7;
/// `tuple`
pub const TUPLE: i64 = 8;
/// `frozenset`
pub const FROZENSET: i64 = 9;
/// `bytearray`
pub const BYTEARRAY: i64 = 10;
