//! Shared type inference helpers
//!
//! Pure functions for method return types, binary operations, and container
//! element unification. Used by both `compute_expr_type` (codegen) and
//! `infer_deep_expr_type` (return type inference) to ensure consistent behavior.

use pyaot_hir as hir;
use pyaot_types::Type;

/// Resolve the return type of a method call based on the object type and method name.
/// Returns `None` if the method is not recognized (caller should apply its own fallback).
pub(crate) fn resolve_method_return_type(obj_ty: &Type, method_name: &str) -> Option<Type> {
    match obj_ty {
        Type::Str => match method_name {
            // String transformation methods
            "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace" | "title"
            | "capitalize" | "swapcase" | "center" | "ljust" | "rjust" | "zfill" | "join"
            | "format" | "removeprefix" | "removesuffix" | "expandtabs" => Some(Type::Str),
            // Methods returning list
            "split" | "splitlines" | "rsplit" => Some(Type::List(Box::new(Type::Str))),
            // Methods returning tuple
            "partition" | "rpartition" => {
                Some(Type::Tuple(vec![Type::Str, Type::Str, Type::Str]))
            }
            // Integer methods
            "find" | "rfind" | "index" | "rindex" | "count" => Some(Type::Int),
            // Boolean predicates
            "startswith" | "endswith" | "isdigit" | "isalpha" | "isalnum" | "isspace"
            | "isupper" | "islower" | "isnumeric" | "isdecimal" | "isascii"
            | "isprintable" | "istitle" | "isidentifier" => Some(Type::Bool),
            // Encoding
            "encode" => Some(Type::Bytes),
            _ => None,
        },
        Type::List(elem_ty) => match method_name {
            "pop" => Some((**elem_ty).clone()),
            "copy" => Some(Type::List(elem_ty.clone())),
            "index" | "count" => Some(Type::Int),
            "append" | "insert" | "remove" | "clear" | "reverse" | "extend" | "sort" => {
                Some(Type::None)
            }
            _ => None,
        },
        Type::Dict(key_ty, val_ty) => match method_name {
            "get" | "pop" | "setdefault" => Some((**val_ty).clone()),
            "copy" => Some(Type::Dict(key_ty.clone(), val_ty.clone())),
            "keys" => Some(Type::List(key_ty.clone())),
            "values" => Some(Type::List(val_ty.clone())),
            "items" => {
                let tuple_ty = Type::Tuple(vec![(**key_ty).clone(), (**val_ty).clone()]);
                Some(Type::List(Box::new(tuple_ty)))
            }
            "popitem" => Some(Type::Tuple(vec![(**key_ty).clone(), (**val_ty).clone()])),
            "clear" | "update" => Some(Type::None),
            _ => None,
        },
        Type::Set(elem_ty) => match method_name {
            "copy" | "union" | "intersection" | "difference" | "symmetric_difference" => {
                Some(Type::Set(elem_ty.clone()))
            }
            "add" | "remove" | "discard" | "clear" => Some(Type::None),
            "issubset" | "issuperset" | "isdisjoint" => Some(Type::Bool),
            _ => None,
        },
        Type::File => match method_name {
            "read" | "readline" => Some(Type::Str),
            "readlines" => Some(Type::List(Box::new(Type::Str))),
            "write" => Some(Type::Int),
            "close" | "flush" => Some(Type::None),
            _ => None,
        },
        _ => None,
    }
}

/// Infer the type of a binary operation from operand types.
/// Returns `None` if the type cannot be determined (caller should apply fallback).
pub(crate) fn resolve_binop_type(
    op: &hir::BinOp,
    left_ty: &Type,
    right_ty: &Type,
) -> Option<Type> {
    // Class types with arithmetic dunders return the class type
    if matches!(left_ty, Type::Class { .. }) {
        return Some(left_ty.clone());
    }
    // Set operations (|, &, -, ^) return Set type
    if let Type::Set(elem_ty) = left_ty {
        if matches!(
            op,
            hir::BinOp::BitOr | hir::BinOp::BitAnd | hir::BinOp::Sub | hir::BinOp::BitXor
        ) {
            return Some(Type::Set(elem_ty.clone()));
        }
    }
    // List concatenation (+) returns List type
    if matches!(left_ty, Type::List(_)) && matches!(op, hir::BinOp::Add) {
        return Some(left_ty.clone());
    }
    // Dict merge (|) returns Dict type
    if matches!(left_ty, Type::Dict(_, _)) && matches!(op, hir::BinOp::BitOr) {
        return Some(left_ty.clone());
    }
    // Python 3: true division (/) always returns float
    if matches!(op, hir::BinOp::Div) {
        return Some(Type::Float);
    }
    // String operations (both operand orders: "a" + "b" and 3 * "abc")
    if *left_ty == Type::Str || *right_ty == Type::Str {
        return Some(Type::Str);
    }
    // Float promotion
    if *left_ty == Type::Float || *right_ty == Type::Float {
        return Some(Type::Float);
    }
    // Integer arithmetic
    if *left_ty == Type::Int && *right_ty == Type::Int {
        return Some(Type::Int);
    }
    None
}

/// Unify a list of types into a single type.
/// If all types are the same, returns that type. Otherwise returns a Union.
pub(crate) fn unify_element_types(types: Vec<Type>) -> Type {
    if types.is_empty() {
        return Type::Any;
    }
    let first = &types[0];
    if types.iter().all(|t| t == first) {
        return first.clone();
    }
    let mut unique = Vec::new();
    for ty in types {
        if !unique.contains(&ty) {
            unique.push(ty);
        }
    }
    Type::Union(unique)
}
