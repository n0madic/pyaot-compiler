//! Centralized type-to-operation mapping
//!
//! Single source of truth for selecting runtime functions based on type.
//! Adding new type support = adding arms to these functions.
//! See REFACTORING_PLAN.md Phase 3.6 (P13).

use pyaot_core_defs::runtime_func_def::{self, RuntimeFuncDef};
use pyaot_mir as mir;
use pyaot_types::Type;

// =============================================================================
// Element tag dispatch
// =============================================================================

// =============================================================================
// Print dispatch
// =============================================================================

/// Select the runtime function for `print(arg)` based on arg type.
pub(crate) fn select_print_func(ty: &Type) -> &'static RuntimeFuncDef {
    if ty.is_union() {
        return &runtime_func_def::RT_PRINT_OBJ;
    }
    if ty.is_list_like() || ty.is_tuple_like() || ty.is_dict_like() || ty.is_set_like() {
        return &runtime_func_def::RT_PRINT_OBJ;
    }
    match ty {
        Type::Int => &runtime_func_def::RT_PRINT_INT,
        Type::Float => &runtime_func_def::RT_PRINT_FLOAT,
        Type::Bool => &runtime_func_def::RT_PRINT_BOOL,
        Type::Str => &runtime_func_def::RT_PRINT_STR_OBJ,
        Type::Bytes => &runtime_func_def::RT_PRINT_BYTES_OBJ,
        // Post-Stage-E `Any`/`HeapAny` always carry tagged `Value` bits
        // (immediate Int/Bool/None or pointer to a heap object). The
        // polymorphic `rt_print_obj` decodes the tag and dispatches
        // correctly; routing through `rt_print_int_value` would read
        // tagged bits like `(payload << 3) | 1` as a raw int and either
        // print garbage (e.g. `49` for payload `6`) or SEGV when the
        // value is a real heap pointer.
        Type::Any | Type::HeapAny => &runtime_func_def::RT_PRINT_OBJ,
        Type::Iterator(_) | Type::RuntimeObject(_) | Type::File(_) => {
            &runtime_func_def::RT_PRINT_OBJ
        }
        _ => &runtime_func_def::RT_PRINT_INT,
    }
}

// =============================================================================
// Len dispatch
// =============================================================================

/// Select the runtime function for `len(arg)` based on arg type.
/// Returns `None` for types that need special handling (e.g., Class with __len__, Any).
pub(crate) fn select_len_func(ty: &Type) -> Option<&'static RuntimeFuncDef> {
    if ty.is_list_like() {
        return Some(&runtime_func_def::RT_LIST_LEN);
    }
    if ty.is_tuple_like() {
        return Some(&runtime_func_def::RT_TUPLE_LEN);
    }
    if ty.is_dict_like() {
        return Some(&runtime_func_def::RT_DICT_LEN);
    }
    if ty.is_set_like() {
        return Some(&runtime_func_def::RT_SET_LEN);
    }
    match ty {
        Type::Str => Some(&runtime_func_def::RT_STR_LEN_INT),
        Type::Bytes => Some(&runtime_func_def::RT_BYTES_LEN),
        // Counter is dict-based
        Type::RuntimeObject(pyaot_core_defs::TypeTagKind::Counter) => {
            Some(&runtime_func_def::RT_DICT_LEN)
        }
        // Deque has its own len
        Type::RuntimeObject(pyaot_core_defs::TypeTagKind::Deque) => {
            Some(&pyaot_stdlib_defs::modules::collections::DEQUE_LEN.codegen)
        }
        _ => None,
    }
}

// =============================================================================
// Conversion kind mapping
// =============================================================================

/// Map a Type to its `ConversionTypeKind` for use with `ConversionTypeKind::convert_def()`.
/// Returns `None` for types that don't have a direct conversion (e.g., Class, BuiltinException).
pub(crate) fn type_to_conversion_kind(ty: &Type) -> Option<mir::ConversionTypeKind> {
    match ty {
        Type::Int => Some(mir::ConversionTypeKind::Int),
        Type::Float => Some(mir::ConversionTypeKind::Float),
        Type::Bool => Some(mir::ConversionTypeKind::Bool),
        Type::None => Some(mir::ConversionTypeKind::None),
        Type::Str => Some(mir::ConversionTypeKind::Str),
        _ => None,
    }
}

// =============================================================================
// Iterator source mapping
// =============================================================================

/// Map a container Type to its `IterSourceKind` for iterator creation.
pub(crate) fn type_to_iter_source(ty: &Type) -> mir::IterSourceKind {
    if ty.is_list_like() {
        return mir::IterSourceKind::List;
    }
    if ty.is_tuple_like() {
        return mir::IterSourceKind::Tuple;
    }
    if ty.is_dict_like() {
        return mir::IterSourceKind::Dict;
    }
    if ty.is_set_like() {
        return mir::IterSourceKind::Set;
    }
    match ty {
        Type::Str => mir::IterSourceKind::Str,
        Type::Bytes => mir::IterSourceKind::Bytes,
        Type::Iterator(_) => mir::IterSourceKind::Generator,
        _ => mir::IterSourceKind::List, // Fallback
    }
}

// =============================================================================
// Slicing dispatch
// =============================================================================

/// Select the runtime function for simple slicing (no step) by object type.
/// Returns `None` for types that need special handling or do not support slicing.
pub(crate) fn select_slicing_func(ty: &Type) -> Option<&'static RuntimeFuncDef> {
    if ty.is_list_like() {
        return Some(&runtime_func_def::RT_LIST_SLICE);
    }
    if ty.is_tuple_like() {
        return Some(&runtime_func_def::RT_TUPLE_SLICE);
    }
    match ty {
        Type::Str => Some(&runtime_func_def::RT_STR_SLICE),
        Type::Bytes => Some(&runtime_func_def::RT_BYTES_SLICE),
        _ => None,
    }
}

/// Select the runtime function for step slicing (with step) by object type.
/// Returns `None` for types that need special handling or do not support slicing.
pub(crate) fn select_slicing_step_func(ty: &Type) -> Option<&'static RuntimeFuncDef> {
    if ty.is_list_like() {
        return Some(&runtime_func_def::RT_LIST_SLICE_STEP);
    }
    if ty.is_tuple_like() {
        return Some(&runtime_func_def::RT_TUPLE_SLICE_STEP);
    }
    match ty {
        Type::Str => Some(&runtime_func_def::RT_STR_SLICE_STEP),
        Type::Bytes => Some(&runtime_func_def::RT_BYTES_SLICE_STEP),
        _ => None,
    }
}

// =============================================================================
// Truthiness strategy
// =============================================================================

/// Strategy for converting a value to bool.
/// Used by `convert_to_bool` and `convert_to_bool_in_block` in context/helpers.rs.
pub(crate) enum TruthinessStrategy {
    /// Already a bool, no conversion needed
    AlreadyBool,
    /// Compare != 0 (for Int)
    IntNotZero,
    /// Compare != 0.0 (for Float)
    FloatNotZero,
    /// Use len-based check: truthy if len > 0
    LenBased(&'static RuntimeFuncDef),
    /// Always false (None)
    AlwaysFalse,
    /// Call rt_is_truthy runtime function (for Any/HeapAny/Union/other heap types)
    RuntimeIsTruthy,
    /// Class instance — needs __bool__/__len__ dunder lookup (caller handles)
    ClassInstance,
}

/// Select the truthiness conversion strategy for a type.
pub(crate) fn select_truthiness(ty: &Type) -> TruthinessStrategy {
    if ty.is_list_like() {
        return TruthinessStrategy::LenBased(&runtime_func_def::RT_LIST_LEN);
    }
    if ty.is_tuple_like() {
        return TruthinessStrategy::LenBased(&runtime_func_def::RT_TUPLE_LEN);
    }
    if ty.is_dict_like() {
        return TruthinessStrategy::LenBased(&runtime_func_def::RT_DICT_LEN);
    }
    if ty.is_set_like() {
        return TruthinessStrategy::LenBased(&runtime_func_def::RT_SET_LEN);
    }
    match ty {
        Type::Bool => TruthinessStrategy::AlreadyBool,
        Type::Int => TruthinessStrategy::IntNotZero,
        Type::Float => TruthinessStrategy::FloatNotZero,
        Type::Str => TruthinessStrategy::LenBased(&runtime_func_def::RT_STR_LEN_INT),
        Type::None => TruthinessStrategy::AlwaysFalse,
        Type::Bytes => TruthinessStrategy::LenBased(&runtime_func_def::RT_BYTES_LEN),
        Type::Class { .. } => TruthinessStrategy::ClassInstance,
        Type::Union(_) | Type::Any => TruthinessStrategy::RuntimeIsTruthy,
        _ => TruthinessStrategy::RuntimeIsTruthy,
    }
}
