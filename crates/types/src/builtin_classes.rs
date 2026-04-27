//! Reserved `ClassId` constants for built-in generic container types.
//!
//! These occupy the range
//! `[BUILTIN_EXCEPTION_COUNT + RESERVED_STDLIB_EXCEPTION_SLOTS,
//!   FIRST_USER_CLASS_ID)` — the five slots added in S3.2 between the stdlib
//! exception range and the user-class range.

use pyaot_utils::ClassId;

const BASE: u32 = (pyaot_core_defs::BUILTIN_EXCEPTION_COUNT as u32)
    + (pyaot_core_defs::RESERVED_STDLIB_EXCEPTION_SLOTS as u32);

/// `ClassId` for `list[T]` — `Type::Generic { base: BUILTIN_LIST_CLASS_ID, args: [T] }`.
pub const BUILTIN_LIST_CLASS_ID: ClassId = ClassId(BASE);

/// `ClassId` for `dict[K, V]` — `Type::Generic { base: BUILTIN_DICT_CLASS_ID, args: [K, V] }`.
pub const BUILTIN_DICT_CLASS_ID: ClassId = ClassId(BASE + 1);

/// `ClassId` for `set[T]` — `Type::Generic { base: BUILTIN_SET_CLASS_ID, args: [T] }`.
pub const BUILTIN_SET_CLASS_ID: ClassId = ClassId(BASE + 2);

/// `ClassId` for fixed-arity `tuple[T1, T2, ...]` —
/// `Type::Generic { base: BUILTIN_TUPLE_CLASS_ID, args: [T1, T2, ...] }`.
pub const BUILTIN_TUPLE_CLASS_ID: ClassId = ClassId(BASE + 3);

/// `ClassId` for variable-length `tuple[T, ...]` (PEP 484) —
/// `Type::Generic { base: BUILTIN_TUPLE_VAR_CLASS_ID, args: [T] }`.
pub const BUILTIN_TUPLE_VAR_CLASS_ID: ClassId = ClassId(BASE + 4);
