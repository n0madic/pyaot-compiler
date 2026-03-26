//! Shared type inference helpers
//!
//! Pure functions for method return types, binary operations, container
//! element unification, builtin call types, and index resolution.
//! Used by both `compute_expr_type` (codegen) and `infer_deep_expr_type`
//! (return type inference) to ensure consistent behavior.

use pyaot_hir as hir;
use pyaot_types::Type;

use super::infer::extract_iterable_element_type;

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
            "partition" | "rpartition" => Some(Type::Tuple(vec![Type::Str, Type::Str, Type::Str])),
            // Integer methods
            "find" | "rfind" | "index" | "rindex" | "count" => Some(Type::Int),
            // Boolean predicates
            "startswith" | "endswith" | "isdigit" | "isalpha" | "isalnum" | "isspace"
            | "isupper" | "islower" | "isnumeric" | "isdecimal" | "isascii" | "isprintable"
            | "istitle" | "isidentifier" => Some(Type::Bool),
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
            // Note: get() can return None when key is missing, but returning
            // Optional[V] here would change the runtime representation (boxing),
            // which causes crashes in the AOT compiler. Keep as V for safety.
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
pub(crate) fn resolve_binop_type(op: &hir::BinOp, left_ty: &Type, right_ty: &Type) -> Option<Type> {
    // Class types with arithmetic dunders return the class type
    if matches!(left_ty, Type::Class { .. }) {
        return Some(left_ty.clone());
    }
    // Reverse dunder case: right operand is a class, result is that class type
    if matches!(right_ty, Type::Class { .. }) {
        return Some(right_ty.clone());
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
    // String operations:
    // - Add: str + str (concatenation) — both sides must be Str
    // - Mul: str * int or int * str (repeat)
    // - Mod: str % ... (formatting) — left side must be Str
    if *left_ty == Type::Str && *right_ty == Type::Str && matches!(op, hir::BinOp::Add) {
        return Some(Type::Str);
    }
    if matches!(op, hir::BinOp::Mul)
        && ((*left_ty == Type::Str && *right_ty == Type::Int)
            || (*left_ty == Type::Int && *right_ty == Type::Str))
    {
        return Some(Type::Str);
    }
    if *left_ty == Type::Str && matches!(op, hir::BinOp::Mod) {
        return Some(Type::Str);
    }
    // Bool is subtype of Int in Python (True + True == 2, True + 1.0 == 2.0)
    let left_ty = if *left_ty == Type::Bool {
        &Type::Int
    } else {
        left_ty
    };
    let right_ty = if *right_ty == Type::Bool {
        &Type::Int
    } else {
        right_ty
    };
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

/// Return the common type of two branches (LogicalOp, IfExpr).
/// Same type → that type; one is Any → Any; otherwise → Union.
pub(crate) fn union_or_any(left: Type, right: Type) -> Type {
    if left == right {
        left
    } else if left == Type::Any || right == Type::Any {
        Type::Any
    } else {
        Type::normalize_union(vec![left, right])
    }
}

/// Unify a list of types into a single type.
/// If all types are the same, returns that type. Otherwise returns a normalized Union.
pub(crate) fn unify_element_types(types: Vec<Type>) -> Type {
    if types.is_empty() {
        return Type::Any;
    }
    let first = &types[0];
    if types.iter().all(|t| t == first) {
        return first.clone();
    }
    Type::normalize_union(types)
}

/// Strip `None` from a Union type (unwrap `Optional[T]`).
/// Returns the inner type if `Union[T, None]`, or the original type otherwise.
pub(crate) fn unwrap_optional(ty: &Type) -> Type {
    match ty {
        Type::Union(variants) if variants.contains(&Type::None) => {
            let non_none: Vec<Type> = variants
                .iter()
                .filter(|t| **t != Type::None)
                .cloned()
                .collect();
            match non_none.len() {
                0 => ty.clone(),
                1 => non_none
                    .into_iter()
                    .next()
                    .expect("checked: non_none.len() == 1"),
                _ => Type::Union(non_none),
            }
        }
        _ => ty.clone(),
    }
}

/// Resolve the type of an indexing operation on a known container type.
/// Returns `Type::Any` for unrecognized types (caller handles Class `__getitem__` locally).
pub(crate) fn resolve_index_type(obj_ty: &Type, index_expr: &hir::Expr) -> Type {
    match obj_ty {
        Type::Str => Type::Str,
        Type::Bytes => Type::Int,
        Type::List(elem) => (**elem).clone(),
        Type::Dict(_, val) => (**val).clone(),
        Type::Tuple(elems) if !elems.is_empty() => {
            // Try compile-time index resolution for Int literals
            if let hir::ExprKind::Int(idx) = &index_expr.kind {
                let len = elems.len() as i64;
                let actual_idx = if *idx < 0 { len + idx } else { *idx };
                if actual_idx >= 0 && (actual_idx as usize) < elems.len() {
                    return elems[actual_idx as usize].clone();
                }
            }
            // Fallback: homogeneous → single type, heterogeneous → union
            if elems.iter().all(|t| t == &elems[0]) {
                elems[0].clone()
            } else {
                Type::normalize_union(elems.clone())
            }
        }
        _ => Type::Any,
    }
}

/// Resolve the return type of a builtin function call.
///
/// `arg_types` must be pre-computed by the caller (one entry per element in `args`).
/// Returns `None` if the builtin requires caller-specific context (e.g., `Map` needs
/// `func_return_types`) or is not recognized.
pub(crate) fn resolve_builtin_call_type(
    builtin: &hir::Builtin,
    args: &[hir::ExprId],
    arg_types: &[Type],
    module: &hir::Module,
) -> Option<Type> {
    use hir::Builtin;
    match builtin {
        // === Type conversions ===
        Builtin::Int => Some(Type::Int),
        Builtin::Float => Some(Type::Float),
        Builtin::Bool => Some(Type::Bool),
        Builtin::Str => Some(Type::Str),
        Builtin::Bytes => Some(Type::Bytes),

        // === Integer-returning builtins ===
        Builtin::Len | Builtin::Hash | Builtin::Id | Builtin::Ord => Some(Type::Int),

        // === String-returning builtins ===
        Builtin::Chr
        | Builtin::Repr
        | Builtin::Ascii
        | Builtin::Format
        | Builtin::Input
        | Builtin::Bin
        | Builtin::Hex
        | Builtin::Oct
        | Builtin::FmtBin
        | Builtin::FmtHex
        | Builtin::FmtHexUpper
        | Builtin::FmtOct
        | Builtin::FmtIntGrouped
        | Builtin::FmtFloatGrouped
        | Builtin::Type => Some(Type::Str),

        // === Boolean-returning builtins ===
        Builtin::Isinstance
        | Builtin::Issubclass
        | Builtin::All
        | Builtin::Any
        | Builtin::Callable
        | Builtin::Hasattr => Some(Type::Bool),

        // === Other fixed types ===
        Builtin::Print | Builtin::Setattr => Some(Type::None),
        Builtin::Range => Some(Type::Iterator(Box::new(Type::Int))),
        Builtin::Pow => Some(Type::Float),
        Builtin::Open => Some(Type::File),
        Builtin::Getattr => Some(Type::Any),

        // === Abs: preserves input type ===
        Builtin::Abs => {
            if let Some(ty) = arg_types.first() {
                Some(ty.clone())
            } else {
                Some(Type::Int)
            }
        }

        // === Sum: int or float ===
        Builtin::Sum => {
            if arg_types.is_empty() {
                return Some(Type::Int);
            }
            let element_type = match &arg_types[0] {
                Type::List(elem) => (**elem).clone(),
                _ => Type::Int,
            };
            let start_type = arg_types.get(1).cloned().unwrap_or(Type::Int);
            if element_type == Type::Float || start_type == Type::Float {
                Some(Type::Float)
            } else {
                Some(Type::Int)
            }
        }

        // === Round ===
        Builtin::Round => {
            if arg_types.len() > 1 {
                Some(arg_types[0].clone())
            } else {
                Some(Type::Int)
            }
        }

        // === Min/Max ===
        Builtin::Min | Builtin::Max => {
            if arg_types.is_empty() {
                return Some(Type::Int);
            }
            // Single-arg form: min(iterable) / max(iterable) — returns element type
            if arg_types.len() == 1 {
                let elem_type = extract_iterable_element_type(&arg_types[0]);
                if elem_type != Type::Any {
                    return Some(elem_type);
                }
            }
            // Multi-arg form: min(a, b, c) — returns the common type
            let has_float = arg_types.contains(&Type::Float);
            Some(if has_float { Type::Float } else { Type::Int })
        }

        // === Divmod ===
        Builtin::Divmod => {
            let result_ty = if !arg_types.is_empty() {
                let a_ty = &arg_types[0];
                let b_ty = arg_types.get(1).unwrap_or(&Type::Int);
                if matches!(a_ty, Type::Float) || matches!(b_ty, Type::Float) {
                    Type::Float
                } else {
                    Type::Int
                }
            } else {
                Type::Int
            };
            Some(Type::Tuple(vec![result_ty.clone(), result_ty]))
        }

        // === Enumerate ===
        Builtin::Enumerate => {
            if arg_types.is_empty() {
                return Some(Type::Iterator(Box::new(Type::Any)));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::Iterator(Box::new(Type::Tuple(vec![
                Type::Int,
                elem_type,
            ]))))
        }

        // === Zip ===
        Builtin::Zip => {
            if args.is_empty() {
                return Some(Type::Iterator(Box::new(Type::Tuple(vec![]))));
            }
            let mut elem_types = Vec::new();
            for (i, arg_id) in args.iter().enumerate() {
                // Special case: range() returns Int elements
                let arg_expr = &module.exprs[*arg_id];
                if let hir::ExprKind::BuiltinCall {
                    builtin: hir::Builtin::Range,
                    ..
                } = &arg_expr.kind
                {
                    elem_types.push(Type::Int);
                    continue;
                }
                if let Some(ty) = arg_types.get(i) {
                    elem_types.push(extract_iterable_element_type(ty));
                } else {
                    elem_types.push(Type::Any);
                }
            }
            Some(Type::Iterator(Box::new(Type::Tuple(elem_types))))
        }

        // === Iter ===
        // Note: Class __iter__ override must be handled by the caller
        Builtin::Iter => {
            if arg_types.is_empty() {
                return Some(Type::Iterator(Box::new(Type::Any)));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::Iterator(Box::new(elem_type)))
        }

        // === Reversed ===
        Builtin::Reversed => {
            if arg_types.is_empty() {
                return Some(Type::Iterator(Box::new(Type::Any)));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::Iterator(Box::new(elem_type)))
        }

        // === Next ===
        // Note: Class __next__ override must be handled by the caller
        Builtin::Next => {
            if arg_types.is_empty() {
                return Some(Type::Any);
            }
            match &arg_types[0] {
                Type::Iterator(elem) => Some((**elem).clone()),
                _ => Some(Type::Any),
            }
        }

        // === Sorted ===
        Builtin::Sorted => {
            if arg_types.is_empty() {
                return Some(Type::List(Box::new(Type::Any)));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::List(Box::new(elem_type)))
        }

        // === List constructor ===
        Builtin::List => {
            if arg_types.is_empty() {
                return Some(Type::List(Box::new(Type::Any)));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::List(Box::new(elem_type)))
        }

        // === Tuple constructor ===
        Builtin::Tuple => {
            if arg_types.is_empty() {
                return Some(Type::Tuple(vec![]));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::Tuple(vec![elem_type]))
        }

        // === Dict constructor ===
        Builtin::Dict => Some(Type::Dict(Box::new(Type::Any), Box::new(Type::Any))),

        // === Set constructor ===
        Builtin::Set => {
            if arg_types.is_empty() {
                return Some(Type::Set(Box::new(Type::Any)));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::Set(Box::new(elem_type)))
        }

        // === Filter ===
        Builtin::Filter => {
            if arg_types.len() >= 2 {
                let elem_type = extract_iterable_element_type(&arg_types[1]);
                Some(Type::Iterator(Box::new(elem_type)))
            } else {
                Some(Type::Iterator(Box::new(Type::Any)))
            }
        }

        // === Chain ===
        Builtin::Chain => Some(Type::Iterator(Box::new(Type::Any))),

        // === ISlice ===
        Builtin::ISlice => {
            if !arg_types.is_empty() {
                let elem_type = extract_iterable_element_type(&arg_types[0]);
                Some(Type::Iterator(Box::new(elem_type)))
            } else {
                Some(Type::Iterator(Box::new(Type::Any)))
            }
        }

        // === Reduce ===
        Builtin::Reduce => {
            if arg_types.len() >= 2 {
                Some(extract_iterable_element_type(&arg_types[1]))
            } else {
                Some(Type::Any)
            }
        }

        // Map needs func_return_types — handled by caller
        Builtin::Map => None,

        // BuiltinException — complex, handled by caller
        Builtin::BuiltinException(_) => None,
    }
}

// =============================================================================
// Container Type Inference Helpers
// =============================================================================

/// Infer list type from pre-computed element types.
/// Empty lists use the expression's type annotation if available.
pub(crate) fn infer_list_type(elem_types: Vec<Type>, expr_ty: Option<&Type>) -> Type {
    if elem_types.is_empty() {
        expr_ty.cloned().unwrap_or(Type::List(Box::new(Type::Any)))
    } else {
        Type::List(Box::new(unify_element_types(elem_types)))
    }
}

/// Infer dict type from pre-computed key and value types.
/// Empty dicts default to Dict[Any, Any].
pub(crate) fn infer_dict_type(key_types: Vec<Type>, val_types: Vec<Type>) -> Type {
    if key_types.is_empty() {
        Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
    } else {
        Type::Dict(
            Box::new(unify_element_types(key_types)),
            Box::new(unify_element_types(val_types)),
        )
    }
}

/// Infer set type from pre-computed element types.
/// Empty sets default to Set[Any].
pub(crate) fn infer_set_type(elem_types: Vec<Type>) -> Type {
    if elem_types.is_empty() {
        Type::Set(Box::new(Type::Any))
    } else {
        Type::Set(Box::new(unify_element_types(elem_types)))
    }
}
