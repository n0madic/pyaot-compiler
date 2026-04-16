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

/// Determine the elem_tag for a given element type.
/// Returns the constant value that corresponds to how the runtime stores elements:
/// - 0 (ELEM_HEAP_OBJ): Elements are *mut Obj with valid headers
/// - 1 (ELEM_RAW_INT): Elements are raw i64 values
/// - 2 (ELEM_RAW_BOOL): Elements are raw i8 cast to pointer (currently not used in lists)
///
/// This is used when passing elem_tag to runtime functions that need to box
/// raw elements before calling key functions (sorted, min, max with key=).
pub(crate) fn elem_tag_for_type(elem_type: &Type) -> i64 {
    match elem_type {
        Type::Int => pyaot_core_defs::ELEM_RAW_INT as i64,
        Type::Bool => pyaot_core_defs::ELEM_HEAP_OBJ as i64,
        _ => pyaot_core_defs::ELEM_HEAP_OBJ as i64,
    }
}

// =============================================================================
// Tuple element access dispatch
// =============================================================================

/// Select the appropriate TupleGet runtime function for the given element type.
/// Primitive types (Int, Float, Bool) use specialized getters that handle unboxing.
pub(crate) fn tuple_get_func(elem_type: &Type) -> mir::RuntimeFunc {
    match elem_type {
        Type::Int => mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET_INT),
        Type::Float => {
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET_FLOAT)
        }
        Type::Bool => mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET_BOOL),
        _ => mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
    }
}

// =============================================================================
// Print dispatch
// =============================================================================

/// Select the runtime function for `print(arg)` based on arg type.
pub(crate) fn select_print_func(ty: &Type) -> &'static RuntimeFuncDef {
    if ty.is_union() {
        return &runtime_func_def::RT_PRINT_OBJ;
    }
    match ty {
        Type::Int => &runtime_func_def::RT_PRINT_INT,
        Type::Float => &runtime_func_def::RT_PRINT_FLOAT,
        Type::Bool => &runtime_func_def::RT_PRINT_BOOL,
        Type::Str => &runtime_func_def::RT_PRINT_STR_OBJ,
        Type::Bytes => &runtime_func_def::RT_PRINT_BYTES_OBJ,
        Type::HeapAny => &runtime_func_def::RT_PRINT_OBJ,
        Type::List(_)
        | Type::Tuple(_)
        | Type::TupleVar(_)
        | Type::Dict(_, _)
        | Type::DefaultDict(_, _)
        | Type::Set(_)
        | Type::Iterator(_)
        | Type::RuntimeObject(_)
        | Type::File(_) => &runtime_func_def::RT_PRINT_OBJ,
        // Any: ambiguous (could be raw i64) — print as Int
        Type::Any => &runtime_func_def::RT_PRINT_INT,
        _ => &runtime_func_def::RT_PRINT_INT,
    }
}

// =============================================================================
// Len dispatch
// =============================================================================

/// Select the runtime function for `len(arg)` based on arg type.
/// Returns `None` for types that need special handling (e.g., Class with __len__, Any).
pub(crate) fn select_len_func(ty: &Type) -> Option<&'static RuntimeFuncDef> {
    match ty {
        Type::Str => Some(&runtime_func_def::RT_STR_LEN_INT),
        Type::List(_) => Some(&runtime_func_def::RT_LIST_LEN),
        Type::Tuple(_) | Type::TupleVar(_) => Some(&runtime_func_def::RT_TUPLE_LEN),
        Type::Dict(_, _) | Type::DefaultDict(_, _) => Some(&runtime_func_def::RT_DICT_LEN),
        Type::Set(_) => Some(&runtime_func_def::RT_SET_LEN),
        Type::Bytes => Some(&runtime_func_def::RT_BYTES_LEN),
        // Counter is dict-based
        Type::RuntimeObject(pyaot_core_defs::type_tags::TypeTagKind::Counter) => {
            Some(&runtime_func_def::RT_DICT_LEN)
        }
        // Deque has its own len
        Type::RuntimeObject(pyaot_core_defs::type_tags::TypeTagKind::Deque) => {
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
    match ty {
        Type::List(_) => mir::IterSourceKind::List,
        Type::Tuple(_) | Type::TupleVar(_) => mir::IterSourceKind::Tuple,
        Type::Dict(_, _) | Type::DefaultDict(_, _) => mir::IterSourceKind::Dict,
        Type::Set(_) => mir::IterSourceKind::Set,
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
    match ty {
        Type::Str => Some(&runtime_func_def::RT_STR_SLICE),
        Type::List(_) => Some(&runtime_func_def::RT_LIST_SLICE),
        Type::Tuple(_) | Type::TupleVar(_) => Some(&runtime_func_def::RT_TUPLE_SLICE),
        Type::Bytes => Some(&runtime_func_def::RT_BYTES_SLICE),
        _ => None,
    }
}

/// Select the runtime function for step slicing (with step) by object type.
/// Returns `None` for types that need special handling or do not support slicing.
pub(crate) fn select_slicing_step_func(ty: &Type) -> Option<&'static RuntimeFuncDef> {
    match ty {
        Type::Str => Some(&runtime_func_def::RT_STR_SLICE_STEP),
        Type::List(_) => Some(&runtime_func_def::RT_LIST_SLICE_STEP),
        Type::Tuple(_) | Type::TupleVar(_) => Some(&runtime_func_def::RT_TUPLE_SLICE_STEP),
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
    match ty {
        Type::Bool => TruthinessStrategy::AlreadyBool,
        Type::Int => TruthinessStrategy::IntNotZero,
        Type::Float => TruthinessStrategy::FloatNotZero,
        Type::Str => TruthinessStrategy::LenBased(&runtime_func_def::RT_STR_LEN_INT),
        Type::None => TruthinessStrategy::AlwaysFalse,
        Type::Bytes => TruthinessStrategy::LenBased(&runtime_func_def::RT_BYTES_LEN),
        Type::List(_) => TruthinessStrategy::LenBased(&runtime_func_def::RT_LIST_LEN),
        Type::Tuple(_) | Type::TupleVar(_) => {
            TruthinessStrategy::LenBased(&runtime_func_def::RT_TUPLE_LEN)
        }
        Type::Dict(_, _) | Type::DefaultDict(_, _) => {
            TruthinessStrategy::LenBased(&runtime_func_def::RT_DICT_LEN)
        }
        Type::Set(_) => TruthinessStrategy::LenBased(&runtime_func_def::RT_SET_LEN),
        Type::Class { .. } => TruthinessStrategy::ClassInstance,
        Type::Union(_) | Type::Any => TruthinessStrategy::RuntimeIsTruthy,
        _ => TruthinessStrategy::RuntimeIsTruthy,
    }
}
