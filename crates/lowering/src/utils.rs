//! Utility functions for HIR to MIR lowering

use pyaot_hir as hir;
use pyaot_types::Type;

/// Check if a type requires GC tracking (heap-allocated types)
pub(crate) fn is_heap_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Str
            | Type::Bytes
            | Type::List(_)
            | Type::Dict(_, _)
            | Type::Set(_)
            | Type::Tuple(_)
            | Type::Class { .. }
            | Type::Iterator(_)
            | Type::Union(_)
            | Type::File
    )
}

/// Check if a type is mutable and thus needs special handling for function defaults.
/// In Python, mutable defaults (list, dict, set, class instances) are evaluated once
/// at function definition time and shared across all calls.
pub(crate) fn is_mutable_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::List(_) | Type::Dict(_, _) | Type::Set(_) | Type::Class { .. }
    )
}

/// Check if an expression represents a mutable default value.
/// This checks both the expression kind and its inferred type.
pub(crate) fn is_mutable_default_expr(expr: &hir::Expr) -> bool {
    match &expr.kind {
        // Empty list literal: []
        hir::ExprKind::List(_) => true,
        // Dict literal: {} or {k: v}
        hir::ExprKind::Dict(_) => true,
        // Set literal: {a, b} (note: empty {} is a dict in Python)
        hir::ExprKind::Set(_) => true,
        // Function call could create mutable object (e.g., list(), dict(), set(), MyClass())
        hir::ExprKind::Call { .. } => {
            // Check the type annotation if available
            if let Some(ref ty) = expr.ty {
                is_mutable_type(ty)
            } else {
                // Conservatively assume calls might create mutable objects
                // This handles: list(), dict(), set(), MyClass()
                true
            }
        }
        // Class instantiation
        hir::ExprKind::ClassRef(_) => true,
        _ => false,
    }
}

/// Kind of iterable for for-loop lowering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IterableKind {
    /// Iterate over list elements
    List,
    /// Iterate over tuple elements
    Tuple,
    /// Iterate over dict keys
    Dict,
    /// Iterate over set elements
    Set,
    /// Iterate over string characters
    Str,
    /// Iterate over bytes (yields integers)
    Bytes,
    /// Iterate using iterator protocol (for generators)
    Iterator,
    /// Iterate over file lines (readlines then iterate)
    File,
}

/// Try to determine if an expression is an iterable and extract its kind and element type.
/// Returns None if the type is not a supported iterable (e.g., range() should be handled separately).
pub(crate) fn get_iterable_info(ty: &Type) -> Option<(IterableKind, Type)> {
    match ty {
        Type::List(elem_ty) => Some((IterableKind::List, (**elem_ty).clone())),
        Type::Tuple(elem_types) => {
            // For tuples, compute the union of all element types for iteration
            let elem_ty = if elem_types.is_empty() {
                Type::Any
            } else {
                Type::normalize_union(elem_types.clone())
            };
            Some((IterableKind::Tuple, elem_ty))
        }
        Type::Dict(key_ty, _value_ty) => {
            // Iterating over a dict yields keys
            Some((IterableKind::Dict, (**key_ty).clone()))
        }
        Type::Set(elem_ty) => {
            // Iterating over a set yields elements
            Some((IterableKind::Set, (**elem_ty).clone()))
        }
        Type::Str => {
            // Iterating over a string yields single-character strings
            Some((IterableKind::Str, Type::Str))
        }
        Type::Bytes => {
            // Iterating over bytes yields integers (0-255)
            Some((IterableKind::Bytes, Type::Int))
        }
        Type::Iterator(elem_ty) => {
            // Iterating over an iterator/generator yields its element type
            Some((IterableKind::Iterator, (**elem_ty).clone()))
        }
        Type::File => {
            // Iterating over a file yields lines as strings
            Some((IterableKind::File, Type::Str))
        }
        _ => None,
    }
}

/// Step direction for range loops
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepDirection {
    /// Step is known positive at compile time
    Positive,
    /// Step is known negative at compile time
    Negative,
    /// Step direction unknown at compile time (requires runtime check)
    Unknown,
}

/// Try to determine the step direction from a HIR expression at compile time.
/// Handles: Int(n), UnaryOp(Neg, Int(n))
pub(crate) fn get_step_direction(expr: &hir::Expr, hir_module: &hir::Module) -> StepDirection {
    match &expr.kind {
        hir::ExprKind::Int(n) => {
            if *n > 0 {
                StepDirection::Positive
            } else if *n < 0 {
                StepDirection::Negative
            } else {
                // step == 0 is an error in Python, treat as unknown
                StepDirection::Unknown
            }
        }
        hir::ExprKind::UnOp {
            op: hir::UnOp::Neg,
            operand,
        } => {
            // -n where n is positive means negative step
            let operand_expr = &hir_module.exprs[*operand];
            if let hir::ExprKind::Int(n) = &operand_expr.kind {
                if *n > 0 {
                    StepDirection::Negative
                } else if *n < 0 {
                    // --n (double negative) means positive
                    StepDirection::Positive
                } else {
                    StepDirection::Unknown
                }
            } else {
                StepDirection::Unknown
            }
        }
        _ => StepDirection::Unknown,
    }
}
