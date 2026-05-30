//! Utility functions for HIR to MIR lowering

use pyaot_hir as hir;
use pyaot_types::{Type, TypeLattice};

/// Check if a type is mutable and thus needs special handling for function defaults.
/// In Python, mutable defaults (list, dict, set, class instances) are evaluated once
/// at function definition time and shared across all calls.
pub(crate) fn is_mutable_type(ty: &Type) -> bool {
    ty.is_list_like() || ty.is_dict_like() || ty.is_set_like() || matches!(ty, Type::Class { .. })
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
        // Class reference (not instance) is not mutable — only an instantiation would be
        hir::ExprKind::ClassRef(_) => false,
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
    // Union[A, B, ...]: all variants must be compatible iterables of the same kind.
    // Element types are joined (least upper bound). An empty-tuple variant contributes
    // IterableKind::Tuple but no element (treated as lattice bottom so other variants
    // dominate the join, preserving precision).
    if let Type::Union(variants) = ty {
        if variants.is_empty() {
            return None;
        }
        let mut shared_kind: Option<IterableKind> = None;
        let mut joined_elem: Type = Type::Never;
        for v in variants {
            // Empty-tuple contributes kind=Tuple but no elements — treat as bottom.
            if let Type::Generic { base, args } = v {
                if *base == pyaot_types::builtin_classes::BUILTIN_TUPLE_CLASS_ID && args.is_empty()
                {
                    match shared_kind {
                        None => shared_kind = Some(IterableKind::Tuple),
                        Some(IterableKind::Tuple) => {}
                        Some(_) => return None,
                    }
                    continue;
                }
            }
            let (kind, elem) = get_iterable_info(v)?;
            match shared_kind {
                None => shared_kind = Some(kind),
                Some(k) if k == kind => {}
                Some(_) => return None,
            }
            joined_elem = joined_elem.join(&elem);
        }
        return shared_kind.map(|k| (k, joined_elem));
    }

    if let Some(elem_ty) = ty.list_elem() {
        return Some((IterableKind::List, elem_ty.clone()));
    }
    if let Some(elem_types) = ty.tuple_elems() {
        // For tuples, compute the union of all element types for iteration
        let elem_ty = if elem_types.is_empty() {
            Type::Any
        } else {
            elem_types
                .iter()
                .cloned()
                .reduce(|a, b| a.join(&b))
                .unwrap_or(Type::Never)
        };
        return Some((IterableKind::Tuple, elem_ty));
    }
    if let Some(elem_ty) = ty.tuple_var_elem() {
        return Some((IterableKind::Tuple, elem_ty.clone()));
    }
    if let Some((key_ty, _value_ty)) = ty.dict_kv() {
        // Iterating over a dict yields keys
        return Some((IterableKind::Dict, key_ty.clone()));
    }
    if let Some(elem_ty) = ty.set_elem() {
        // Iterating over a set yields elements
        return Some((IterableKind::Set, elem_ty.clone()));
    }
    match ty {
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
        Type::File(binary) => {
            // Iterating over a file yields lines — str for text mode, bytes
            // for binary mode (matches CPython's file-iterator semantics).
            let elem = if *binary { Type::Bytes } else { Type::Str };
            Some((IterableKind::File, elem))
        }
        // NOTE: a deque is intentionally NOT iterable here. It carries no
        // element type, so iteration would yield `Any`, and arithmetic /
        // method calls on an `Any` loop variable hit the conservative
        // `operand_is_guaranteed_tagged` gate (raw-BinOp verifier reject) and
        // the Any-method silent-miss. `list(deque)` (a direct conversion) is
        // supported for display/inspection; full deque iteration awaits deque
        // element-type tracking.
        _ => None,
    }
}
