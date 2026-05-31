//! Shared type inference helpers
//!
//! Pure functions for method return types, binary operations, container
//! element unification, builtin call types, and index resolution.
//! Used by both `compute_expr_type` (codegen) and `seed_infer_expr_type`
//! (return type inference) to ensure consistent behavior.

use pyaot_hir as hir;
use pyaot_types::{Type, TypeLattice, TypeTagKind};

use super::infer::extract_iterable_element_type;

/// Callback signature for "does class `id` define dunder `name`?".
/// Passed by the `Lowering` context to free helper functions so they can
/// consult class metadata without depending on the full lowering state.
/// `None` means callers without class info (free-function callers).
type ClassHasDunderFn<'a> = Option<&'a dyn Fn(pyaot_utils::ClassId, &str) -> bool>;

/// Result-type class of a numeric reduction (`sum`/`min`/`max`).
///
/// The element type of the iterable determines how the reduction
/// accumulates: a definitely-int element folds in a raw `i64`, a
/// definitely-float element in a raw `f64`, and a `Type::Any` element
/// (a tagged `Value` whose numeric shape is only known at runtime) folds
/// through the runtime (`rt_obj_add` / `rt_obj_cmp`), keeping the result
/// a tagged `Value`. The tagged path is what lets `sum`/`min`/`max` over
/// an `Any`-element iterator preserve `int` vs `float` exactly as CPython
/// does, instead of forcing every such reduction onto the float path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReductionResult {
    /// Element resolves to int/bool — accumulate as a raw `i64`.
    Int,
    /// Element resolves to float — accumulate as a raw `f64`.
    Float,
    /// Element is `Type::Any` — accumulate via `rt_obj_*`; result stays tagged.
    Tagged,
}

impl ReductionResult {
    /// Logical `Type` of the reduction result.
    pub(crate) fn result_type(self) -> Type {
        match self {
            ReductionResult::Int => Type::Int,
            ReductionResult::Float => Type::Float,
            ReductionResult::Tagged => Type::Any,
        }
    }

    /// Combine the element class with the start-value class. `Tagged`
    /// dominates — when either side is a tagged Value (`Any` or a Union
    /// that carries `Float` as a tagged-FloatObj-pointer variant), the
    /// only safe accumulation is through `rt_obj_*`, which dispatches by
    /// runtime tag and preserves int vs float exactly as CPython does.
    /// `Float` dominates `Int` (numeric tower). `Int` is the identity.
    ///
    /// Previously Float dominated Tagged; that order caused
    /// `sum(any_iter, 1.5)` to route element=Any (Tagged) results through
    /// the Float fast-path's `IntToFloat` on tagged-Value bits, which
    /// the verifier correctly rejects (#3). The new order routes the
    /// reduction through `rt_obj_add` instead.
    pub(crate) fn join(self, other: ReductionResult) -> ReductionResult {
        match (self, other) {
            (ReductionResult::Tagged, _) | (_, ReductionResult::Tagged) => ReductionResult::Tagged,
            (ReductionResult::Float, _) | (_, ReductionResult::Float) => ReductionResult::Float,
            _ => ReductionResult::Int,
        }
    }
}

/// Classify a reduction's iterator element type into a [`ReductionResult`].
///
/// Returns `Float` ONLY for concrete `Type::Float` — the only shape that
/// is guaranteed to be a raw `f64` in an XMM register. Returns `Tagged`
/// for bare `Type::Any` AND for a `Union` that contains `Float` (the
/// polymorphic-dunder seed `Union[Float, Class[Self]]` from
/// `pyaot_types::dunders::polymorphic_other_type` — autograd-style
/// `Value.data`). Every Union-with-Float yield site is a tagged
/// `Value`, NOT a raw f64: the runtime tag at that site may be a
/// `FloatObj` pointer (autograd) or an instance pointer (when the dunder
/// returns Self). Tagged dispatch through `rt_obj_add` handles both
/// correctly. Returns `Int` for everything else (`Int`/`Bool`/`Class[T]`/...).
pub(crate) fn classify_reduction_elem(elem_ty: &Type) -> ReductionResult {
    match elem_ty {
        Type::Float => ReductionResult::Float,
        Type::Any => ReductionResult::Tagged,
        Type::Union(variants) if variants.iter().any(|v| matches!(v, Type::Float)) => {
            ReductionResult::Tagged
        }
        _ => ReductionResult::Int,
    }
}

/// Resolve the return type of a method call based on the object type and method name.
/// Returns `None` if the method is not recognized (caller should apply its own fallback).
pub(crate) fn resolve_method_return_type(obj_ty: &Type, method_name: &str) -> Option<Type> {
    if let Some(elem_ty) = obj_ty.list_elem() {
        return match method_name {
            "pop" => Some(elem_ty.clone()),
            "copy" => Some(Type::list_of(elem_ty.clone())),
            "index" | "count" => Some(Type::Int),
            "append" | "insert" | "remove" | "clear" | "reverse" | "extend" | "sort" => {
                Some(Type::None)
            }
            _ => None,
        };
    }
    if let Some((key_ty, val_ty)) = obj_ty.dict_kv() {
        return match method_name {
            // Note: get() can return None when key is missing, but returning
            // Optional[V] here would change the runtime representation (boxing),
            // which causes crashes in the AOT compiler. Keep as V for safety.
            "get" | "pop" | "setdefault" => Some(val_ty.clone()),
            "copy" => Some(Type::dict_of(key_ty.clone(), val_ty.clone())),
            "keys" => Some(Type::list_of(key_ty.clone())),
            "values" => Some(Type::list_of(val_ty.clone())),
            "items" => {
                let tuple_ty = Type::tuple_of(vec![key_ty.clone(), val_ty.clone()]);
                Some(Type::list_of(tuple_ty))
            }
            "popitem" => Some(Type::tuple_of(vec![key_ty.clone(), val_ty.clone()])),
            "clear" | "update" | "move_to_end" => Some(Type::None),
            _ => None,
        };
    }
    if let Some(elem_ty) = obj_ty.set_elem() {
        return match method_name {
            "copy" | "union" | "intersection" | "difference" | "symmetric_difference" => {
                Some(Type::set_of(elem_ty.clone()))
            }
            "add" | "remove" | "discard" | "clear" => Some(Type::None),
            "issubset" | "issuperset" | "isdisjoint" => Some(Type::Bool),
            _ => None,
        };
    }
    if let Some(elem_ty) = obj_ty.deque_elem() {
        // deque methods (see DEQUE_METHODS in stdlib-defs). `pop`/`popleft`
        // yield the element type; `copy` preserves the deque type; `count`
        // is Int; the mutators return None.
        return match method_name {
            "pop" | "popleft" => Some(elem_ty.clone()),
            "copy" => Some(Type::deque_of(elem_ty.clone())),
            "count" | "index" => Some(Type::Int),
            "append" | "appendleft" | "extend" | "extendleft" | "rotate" | "clear" | "reverse"
            | "insert" | "remove" => Some(Type::None),
            _ => None,
        };
    }
    match obj_ty {
        // int / bool methods (bool is an int subtype). All currently
        // supported ones yield int. Keep this name list in sync with
        // `lower_int_method` in expressions/access/method/int.rs (the lowering
        // side that emits the runtime call / identity widening).
        Type::Int | Type::Bool => match method_name {
            "bit_length" | "bit_count" | "conjugate" | "__int__" | "__index__" | "__trunc__" => {
                Some(Type::Int)
            }
            _ => None,
        },
        Type::Str => match method_name {
            // String transformation methods
            "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace" | "title"
            | "capitalize" | "swapcase" | "center" | "ljust" | "rjust" | "zfill" | "join"
            | "format" | "removeprefix" | "removesuffix" | "expandtabs" => Some(Type::Str),
            // Methods returning list
            "split" | "splitlines" | "rsplit" => Some(Type::list_of(Type::Str)),
            // Methods returning tuple
            "partition" | "rpartition" => {
                Some(Type::tuple_of(vec![Type::Str, Type::Str, Type::Str]))
            }
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
        Type::Bytes => match method_name {
            // Bytes transformation methods
            "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace" | "join" | "fromhex" => {
                Some(Type::Bytes)
            }
            // Decode returns str
            "decode" => Some(Type::Str),
            // Methods returning list
            "split" | "rsplit" => Some(Type::list_of(Type::Bytes)),
            // Integer methods
            "find" | "rfind" | "index" | "rindex" | "count" => Some(Type::Int),
            // Boolean predicates
            "startswith" | "endswith" => Some(Type::Bool),
            // Concatenation/repetition (handled via operators, but for completeness)
            _ => None,
        },
        Type::File(binary) => {
            let str_or_bytes = if *binary { Type::Bytes } else { Type::Str };
            match method_name {
                "read" | "readline" => Some(str_or_bytes),
                "readlines" => Some(Type::list_of(str_or_bytes)),
                "write" => Some(Type::Int),
                "close" | "flush" => Some(Type::None),
                // Context-manager protocol — `with open(...) as f:` desugars
                // to `f = <mgr>.__enter__()`, so __enter__ must return the
                // same File flavour so the binary/text distinction propagates
                // through the `as f` binding.
                "__enter__" => Some(Type::File(*binary)),
                "__exit__" => Some(Type::Bool),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Distribute a BinOp element-wise over the variants of a right-hand
/// `Union`, then join the per-variant results into a canonical type. The
/// helper is conservative: any variant whose result we can't compute
/// (operand combination has no rule) contributes its own type to the join,
/// matching the previous "return the Union as-is" fallback for that arm.
///
/// `class_has_dunder`: optional callback for class-aware filtering. When
/// provided, a `Class { class_id }` variant is DROPPED from the join if
/// the class has neither the forward dunder (when on the same side as
/// the operand here) nor the reflected dunder (when on the opposite
/// side from a non-Class operand). This addresses the polymorphic-
/// dunder-seed pollution: `Float ** Union[Self, int, float, bool]`
/// distributing `Class[Self]` into the result via the structural
/// "Class on side → class type" fallback even when `Self` defines no
/// `__rpow__`. The structural fallback is preserved when the callback
/// is `None` (free-function callers without `&Lowering`).
fn distribute_binop_over_union(
    op: &hir::BinOp,
    left_ty: &Type,
    variants: &[Type],
    class_has_dunder: ClassHasDunderFn<'_>,
) -> Type {
    let mut acc = Type::Never;
    let mut had_any = false;
    for v in variants {
        // Class-on-the-right variant: when we have a class-info callback,
        // drop the variant if neither the forward dunder (on left's
        // class — only meaningful when left_ty is also Class) nor the
        // reflected dunder (on this variant's class) is defined. The
        // structural fallback returns the variant as-is, which is
        // correct only when at least one dispatched dunder applies.
        if let (Some(check), Type::Class { class_id, .. }) = (class_has_dunder, v) {
            if !class_pair_handles_op(
                op, left_ty, *class_id, check, /* variant_is_right = */ true,
            ) {
                continue;
            }
        }
        let part = resolve_binop_type(op, left_ty, v).unwrap_or_else(|| v.clone());
        acc = acc.join(&part);
        had_any = true;
    }
    if !had_any {
        // Every variant was dropped by the class-aware filter — no dispatched
        // dunder applies. Re-join the original variants via `Type::join`,
        // which folds through `make_canonical_union` and collapses the
        // numeric tower (Int/Bool subsumed by Float). The exact Union shape
        // is NOT preserved; what we preserve is "result stays a tagged-
        // Value-dispatchable type" instead of widening to `Any`, which
        // would silence the runtime class-binop dispatch the resolver still
        // expects when the variant set carries Class entries.
        variants.iter().fold(Type::Never, |acc, v| acc.join(v))
    } else {
        // had_any → acc != Never: resolve_binop_type never returns
        // Some(Never); the .unwrap_or_else(|| v.clone()) substitutes the
        // variant itself (never Never) when no rule applies. The previous
        // `else if matches!(acc, Type::Never) { Type::Any }` arm was dead
        // and has been removed.
        acc
    }
}

/// Mirror of `distribute_binop_over_union` for the left-hand `Union` case.
fn distribute_binop_over_union_left(
    op: &hir::BinOp,
    variants: &[Type],
    right_ty: &Type,
    class_has_dunder: ClassHasDunderFn<'_>,
) -> Type {
    let mut acc = Type::Never;
    let mut had_any = false;
    for v in variants {
        if let (Some(check), Type::Class { class_id, .. }) = (class_has_dunder, v) {
            if !class_pair_handles_op(
                op, right_ty, *class_id, check, /* variant_is_right = */ false,
            ) {
                continue;
            }
        }
        let part = resolve_binop_type(op, v, right_ty).unwrap_or_else(|| v.clone());
        acc = acc.join(&part);
        had_any = true;
    }
    if !had_any {
        // Mirror of `distribute_binop_over_union`'s no-variant-kept branch:
        // re-join via `Type::join` (numeric tower collapses Int/Bool into
        // Float). See that function for the full rationale.
        variants.iter().fold(Type::Never, |acc, v| acc.join(v))
    } else {
        // had_any → acc != Never (see `distribute_binop_over_union`).
        acc
    }
}

/// Returns true if at least one dispatched dunder applies to a binop
/// where one side is a `Class { class_id: variant_class }` Union variant
/// and the other side is `other_ty`. `variant_is_right` selects whether
/// the variant is on the right (forward = `other_ty`'s class — only
/// meaningful if Class — and reflected = `variant_class.<reflected>`)
/// or the left (forward = `variant_class.<forward>` and reflected =
/// `other_ty`'s class). When neither candidate exists, the binop has no
/// valid dispatch — runtime would raise `TypeError` — so the variant is
/// dropped from the result join.
fn class_pair_handles_op(
    op: &hir::BinOp,
    other_ty: &Type,
    variant_class: pyaot_utils::ClassId,
    class_has_dunder: &dyn Fn(pyaot_utils::ClassId, &str) -> bool,
    variant_is_right: bool,
) -> bool {
    let forward = op.forward_dunder();
    let reflected = op.reflected_dunder();
    let (forward_class, reflected_class) = if variant_is_right {
        // op = other_ty (left) <op> variant_class (right)
        (
            if let Type::Class { class_id, .. } = other_ty {
                Some(*class_id)
            } else {
                None
            },
            Some(variant_class),
        )
    } else {
        // op = variant_class (left) <op> other_ty (right)
        (
            Some(variant_class),
            if let Type::Class { class_id, .. } = other_ty {
                Some(*class_id)
            } else {
                None
            },
        )
    };
    let forward_ok = forward_class.is_some_and(|c| class_has_dunder(c, forward));
    let reflected_ok = reflected_class.is_some_and(|c| class_has_dunder(c, reflected));
    forward_ok || reflected_ok
}

pub(crate) fn resolve_binop_type(op: &hir::BinOp, left_ty: &Type, right_ty: &Type) -> Option<Type> {
    resolve_binop_type_inner(op, left_ty, right_ty, None)
}

/// Class-aware version: when a `Class` variant appears inside a
/// `Union`, the callback lets the resolver consult the class's dunder
/// table and DROP the variant if no dispatched dunder applies.
/// Free-function callers pass `None` and get the legacy structural
/// behaviour where every Class variant is preserved.
pub(crate) fn resolve_binop_type_class_aware(
    op: &hir::BinOp,
    left_ty: &Type,
    right_ty: &Type,
    class_has_dunder: &dyn Fn(pyaot_utils::ClassId, &str) -> bool,
) -> Option<Type> {
    resolve_binop_type_inner(op, left_ty, right_ty, Some(class_has_dunder))
}

fn resolve_binop_type_inner(
    op: &hir::BinOp,
    left_ty: &Type,
    right_ty: &Type,
    class_has_dunder: ClassHasDunderFn<'_>,
) -> Option<Type> {
    // Union arithmetic: result is Union since the actual type depends on
    // runtime values. Division always returns float even for Union (Python
    // 3 semantics).
    if left_ty.is_union() || right_ty.is_union() {
        if matches!(op, hir::BinOp::Div) {
            return Some(Type::Float);
        }
        // Distribute the BinOp element-wise over Union variants and join
        // the per-variant results. The previous behaviour returned the
        // Union side untouched, which propagates an irrelevant `Self`
        // variant into the result whenever an unannotated numeric dunder
        // param carries the auto-generated `Union[Self, int, float, bool]`
        // seed: e.g. inside `Value.__pow__`, `self.data**other` with
        // `self.data: Float` and `other: Union[Value, int, float, bool]`
        // returned the full Union (including `Class[Value]`), which then
        // polluted the harvested `data` field type and degraded runtime
        // dispatch.  Distributing instead yields `Float` for the numeric
        // variants — the only ones that legitimately participate in
        // Float-side arithmetic — and only retains the class variant if
        // the dunder genuinely returns it.
        if let Type::Union(variants) = right_ty {
            return Some(distribute_binop_over_union(
                op,
                left_ty,
                variants,
                class_has_dunder,
            ));
        }
        if let Type::Union(variants) = left_ty {
            return Some(distribute_binop_over_union_left(
                op,
                variants,
                right_ty,
                class_has_dunder,
            ));
        }
    }

    // Class types with arithmetic dunders return the class type
    if matches!(left_ty, Type::Class { .. }) {
        return Some(left_ty.clone());
    }
    // Reverse dunder case: right operand is a class, result is that class type
    if matches!(right_ty, Type::Class { .. }) {
        return Some(right_ty.clone());
    }
    // Set operations (|, &, -, ^) return Set type
    if let Some(elem_ty) = left_ty.set_elem() {
        if matches!(
            op,
            hir::BinOp::BitOr | hir::BinOp::BitAnd | hir::BinOp::Sub | hir::BinOp::BitXor
        ) {
            return Some(Type::set_of(elem_ty.clone()));
        }
    }
    // List concatenation (+) returns List type
    if left_ty.is_list_like() && matches!(op, hir::BinOp::Add) {
        return Some(left_ty.clone());
    }
    // Dict merge (|) returns Dict type
    if left_ty.is_dict_like() && matches!(op, hir::BinOp::BitOr) {
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
    // Sequence repetition: list/bytes * int (and reflected int * sequence).
    // Python's `[0.0] * n` returns the same sequence type. Tuple repetition
    // is intentionally NOT typed here yet — there's no `rt_tuple_repeat`
    // runtime helper, and lowering would otherwise fall through to a raw
    // `mir::BinOp::Mul` which corrupts the tuple pointer. Falling back to
    // `expr.ty.unwrap_or(Any)` for tuples keeps the program compiling
    // (Any → boxed slot) until tuple-repeat runtime support is added.
    if matches!(op, hir::BinOp::Mul) {
        if *right_ty == Type::Int && (left_ty.is_list_like() || *left_ty == Type::Bytes) {
            return Some(left_ty.clone());
        }
        if *left_ty == Type::Int && (right_ty.is_list_like() || *right_ty == Type::Bytes) {
            return Some(right_ty.clone());
        }
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
        left.join(&right)
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
    types
        .into_iter()
        .reduce(|a, b| a.join(&b))
        .unwrap_or(Type::Never)
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
    if matches!(obj_ty, Type::Str) {
        return Type::Str;
    }
    if matches!(obj_ty, Type::Bytes) {
        return Type::Int;
    }
    if let Some(elem) = obj_ty.list_elem() {
        // List elements with Any type are heap pointers from ListGet
        let t = elem.clone();
        return if matches!(t, Type::Any) { Type::Any } else { t };
    }
    if let Some((_, val)) = obj_ty.dict_kv() {
        return val.clone();
    }
    if let Some(elems) = obj_ty.tuple_elems() {
        if !elems.is_empty() {
            // Try compile-time index resolution for Int literals
            if let hir::ExprKind::Int(idx) = &index_expr.kind {
                let len = elems.len() as i64;
                let actual_idx = if *idx < 0 { len + idx } else { *idx };
                if actual_idx >= 0 && (actual_idx as usize) < elems.len() {
                    let t = elems[actual_idx as usize].clone();
                    // Tuple slots are tagged Values; Any elements are promoted to HeapAny.
                    return if matches!(t, Type::Any) { Type::Any } else { t };
                }
            }
            // Fallback: homogeneous → single type, heterogeneous → union
            let t = if elems.iter().all(|t| t == &elems[0]) {
                elems[0].clone()
            } else {
                elems
                    .iter()
                    .cloned()
                    .reduce(|a, b| a.join(&b))
                    .unwrap_or(Type::Never)
            };
            return if matches!(t, Type::Any) { Type::Any } else { t };
        }
    }
    // Variable-length tuple — indexing always returns the element type.
    // Bounds-checked at runtime via rt_tuple_get.
    if let Some(elem) = obj_ty.tuple_var_elem() {
        let t = elem.clone();
        return if matches!(t, Type::Any) { Type::Any } else { t };
    }
    // deque[T] — `dq[i]` returns the element type (lowered via rt_deque_get).
    if let Some(elem) = obj_ty.deque_elem() {
        let t = elem.clone();
        return if matches!(t, Type::Any) { Type::Any } else { t };
    }
    Type::Any
}

/// Single source of truth for a `deque(...)` element type.
///
/// Shared by the builtin-return-type reducer (`resolve_builtin_call_type`)
/// and `lower_deque` so the seeded type and the lowered local type agree.
/// `deque()` (no args) and `deque(maxlen=N)` (the frontend pads `args[0]`
/// with a `None` whose static type is `Type::None`) seed `deque[Never]` —
/// the empty bootstrap the solver narrows through observed appends; boundary
/// coercion demotes an unrefined `Never` to `Any`. Otherwise the element type
/// comes from the iterable argument.
pub(crate) fn deque_elem_from_arg_types(arg_types: &[Type]) -> Type {
    match arg_types.first() {
        None | Some(Type::None) => Type::Never,
        Some(t) => extract_iterable_element_type(t),
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
        // `Builtin::Open`'s binary/text flag is stamped on the Expr's `ty`
        // slot by `ast_to_hir/builtins.rs` (it has the interner and can
        // resolve the mode string). Return `None` here so the caller falls
        // back to `expr.ty`, keeping the frontend as the single source of
        // truth — otherwise this fallback would always overwrite it with
        // text-mode and defeat the whole detection.
        Builtin::Open => None,
        Builtin::Getattr => Some(Type::Any),

        // === Abs: preserves input type ===
        Builtin::Abs => {
            if let Some(ty) = arg_types.first() {
                Some(ty.clone())
            } else {
                Some(Type::Int)
            }
        }

        // === Sum: int, float, or user class (Area C §C.3) ===
        Builtin::Sum => {
            if arg_types.is_empty() {
                return Some(Type::Int);
            }
            let element_type = arg_types[0]
                .list_elem()
                .or_else(|| arg_types[0].set_elem())
                .or_else(|| arg_types[0].iter_elem())
                .cloned()
                .unwrap_or(Type::Int);
            // Never-defer: an element type of `Never` means the iterable's
            // element hasn't been resolved yet (e.g. a generator expression
            // mid-solve whose yield type is still bottom). Returning `Int`
            // here — the default-start type — would, under the solver's
            // MONOTONE join, permanently pollute the result with `Int`: once
            // the element later resolves to a class `V`, the accumulated type
            // becomes `Int | V` instead of `V` (`0 + v0 + …` actually
            // reduces to `V` via `__radd__`). Defer instead so only the
            // resolved element type is recorded. (The legacy planner avoided
            // this via REPLACE-style updates; the constraint solver needs the
            // explicit defer.)
            if matches!(element_type, Type::Never) {
                return None;
            }
            // User class elements: sum returns an instance of the class
            // (matches CPython when `__add__`/`__radd__` are defined).
            if matches!(element_type, Type::Class { .. }) {
                return Some(element_type);
            }
            // 3-way classification (must agree with `lower_sum`): a float
            // element/start → `Float`; an `Any` element/start → `Any`
            // (tagged accumulation preserves int vs float at runtime);
            // otherwise → `Int`.
            let start_type = arg_types.get(1).cloned().unwrap_or(Type::Int);
            let kind =
                classify_reduction_elem(&element_type).join(classify_reduction_elem(&start_type));
            Some(kind.result_type())
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
                return Some(extract_iterable_element_type(&arg_types[0]));
            }
            // Multi-arg form: min(a, b, c). All-string arguments compare
            // lexicographically and return a str (the lowering routes the
            // comparison through the runtime string comparator). Otherwise
            // only the numeric common type is inferred — typing other
            // non-numeric results (e.g. mixed) is a separate feature.
            if arg_types.iter().all(|t| matches!(t, Type::Str)) {
                return Some(Type::Str);
            }
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
            Some(Type::tuple_of(vec![result_ty.clone(), result_ty]))
        }

        // === Enumerate ===
        Builtin::Enumerate => {
            if arg_types.is_empty() {
                return Some(Type::Iterator(Box::new(Type::Any)));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::Iterator(Box::new(Type::tuple_of(vec![
                Type::Int,
                elem_type,
            ]))))
        }

        // === Zip ===
        Builtin::Zip => {
            // Drive the per-position element types off `arg_types`, NOT the
            // `args` ExprId slice: the constraint solver resolves builtins
            // with an EMPTY ExprId slice (it has no ExprIds at the reducer
            // layer), so iterating `args` produced a zero-length
            // `tuple_of([])` — `zip(a, b)` typed `Iterator[tuple[]]`, and the
            // loop targets `a, b in zip(...)` then projected `Never`. The
            // ExprId slice is consulted only for the `range()` special-case
            // when it happens to be available (legacy path).
            if arg_types.is_empty() {
                return Some(Type::Iterator(Box::new(Type::tuple_of(vec![]))));
            }
            let mut elem_types = Vec::new();
            for (i, ty) in arg_types.iter().enumerate() {
                // Special case: range() yields Int elements (only checkable
                // when the ExprId is present).
                if let Some(arg_id) = args.get(i) {
                    if let hir::ExprKind::BuiltinCall {
                        builtin: hir::Builtin::Range,
                        ..
                    } = &module.exprs[*arg_id].kind
                    {
                        elem_types.push(Type::Int);
                        continue;
                    }
                }
                elem_types.push(extract_iterable_element_type(ty));
            }
            Some(Type::Iterator(Box::new(Type::tuple_of(elem_types))))
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
                return Some(Type::list_of(Type::Never));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::list_of(elem_type))
        }

        // === List constructor ===
        Builtin::List => {
            if arg_types.is_empty() {
                return Some(Type::list_of(Type::Never));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::list_of(elem_type))
        }

        // === Tuple constructor ===
        Builtin::Tuple => {
            if arg_types.is_empty() {
                return Some(Type::tuple_of(vec![]));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::tuple_of(vec![elem_type]))
        }

        // === Dict constructor ===
        Builtin::Dict => Some(Type::dict_of(Type::Never, Type::Never)),

        // === Set constructor ===
        Builtin::Set => {
            if arg_types.is_empty() {
                return Some(Type::set_of(Type::Never));
            }
            let elem_type = extract_iterable_element_type(&arg_types[0]);
            Some(Type::set_of(elem_type))
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
        Builtin::Chain => {
            // Derive elem type from the first arg so for-loop unpack picks
            // the right unwrap path (mirrors lower_chain).
            let elem_type = arg_types
                .first()
                .map(extract_iterable_element_type)
                .unwrap_or(Type::Any);
            Some(Type::Iterator(Box::new(elem_type)))
        }

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

        // Collections — type inferred from factory argument
        Builtin::DefaultDict => {
            // args[0] is Int(factory_tag) set by the frontend
            if args.is_empty() {
                // No factory — behaves like regular dict
                Some(Type::dict_of(Type::Any, Type::Any))
            } else {
                let factory_expr = &module.exprs[args[0]];
                let value_type = match &factory_expr.kind {
                    hir::ExprKind::Int(tag) => match *tag {
                        0 => Type::Int,
                        1 => Type::Float,
                        2 => Type::Str,
                        3 => Type::Bool,
                        4 => Type::list_of(Type::Any),
                        5 => Type::dict_of(Type::Any, Type::Any),
                        6 => Type::set_of(Type::Any),
                        _ => Type::Any,
                    },
                    _ => Type::Any,
                };
                Some(Type::DefaultDict(Box::new(Type::Any), Box::new(value_type)))
            }
        }
        Builtin::Counter => Some(Type::RuntimeObject(TypeTagKind::Counter)),
        // Deque — element type from the iterable argument (mirror of
        // `Builtin::List`). `deque()` (no args) and `deque(maxlen=N)` (the
        // frontend pads args[0] with `None`) seed `deque[Never]`, the empty
        // bootstrap the solver narrows through observed appends; boundary
        // coercion demotes an unrefined `Never` to `Any`.
        Builtin::Deque => Some(Type::deque_of(deque_elem_from_arg_types(arg_types))),
        Builtin::ObjectNew => Some(Type::Any),
    }
}

// =============================================================================
// Container Type Inference Helpers
// =============================================================================

/// Infer list type from pre-computed element types.
/// Empty lists use the expression's type annotation if available, else
/// seed `list[Never]` so prescan-merging narrows through usage observation.
/// Boundary coercion in lowering demotes `Never` → `Any` for unrefined empties.
pub(crate) fn infer_list_type(elem_types: Vec<Type>, expr_ty: Option<&Type>) -> Type {
    if elem_types.is_empty() {
        expr_ty
            .cloned()
            .unwrap_or_else(|| Type::list_of(Type::Never))
    } else {
        Type::list_of(unify_element_types(elem_types))
    }
}

/// Infer dict type from pre-computed key and value types.
/// Empty dicts seed `dict[Never, Never]`; see `infer_list_type` rationale.
pub(crate) fn infer_dict_type(key_types: Vec<Type>, val_types: Vec<Type>) -> Type {
    if key_types.is_empty() {
        Type::dict_of(Type::Never, Type::Never)
    } else {
        Type::dict_of(
            unify_element_types(key_types),
            unify_element_types(val_types),
        )
    }
}

/// Infer set type from pre-computed element types.
/// Empty sets seed `set[Never]`; see `infer_list_type` rationale.
pub(crate) fn infer_set_type(elem_types: Vec<Type>) -> Type {
    if elem_types.is_empty() {
        Type::set_of(Type::Never)
    } else {
        Type::set_of(unify_element_types(elem_types))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === resolve_binop_type ===

    #[test]
    fn test_binop_int_add() {
        let result = resolve_binop_type(&hir::BinOp::Add, &Type::Int, &Type::Int);
        assert_eq!(result, Some(Type::Int));
    }

    #[test]
    fn test_binop_float_add() {
        let result = resolve_binop_type(&hir::BinOp::Add, &Type::Float, &Type::Float);
        assert_eq!(result, Some(Type::Float));
    }

    #[test]
    fn test_binop_int_float_promotion() {
        let result = resolve_binop_type(&hir::BinOp::Add, &Type::Int, &Type::Float);
        assert_eq!(result, Some(Type::Float));
    }

    #[test]
    fn test_binop_div_always_float() {
        let result = resolve_binop_type(&hir::BinOp::Div, &Type::Int, &Type::Int);
        assert_eq!(result, Some(Type::Float));
    }

    #[test]
    fn test_binop_str_concat() {
        let result = resolve_binop_type(&hir::BinOp::Add, &Type::Str, &Type::Str);
        assert_eq!(result, Some(Type::Str));
    }

    #[test]
    fn test_binop_str_mul() {
        let result = resolve_binop_type(&hir::BinOp::Mul, &Type::Str, &Type::Int);
        assert_eq!(result, Some(Type::Str));
        let result = resolve_binop_type(&hir::BinOp::Mul, &Type::Int, &Type::Str);
        assert_eq!(result, Some(Type::Str));
    }

    #[test]
    fn test_binop_str_format() {
        let result = resolve_binop_type(&hir::BinOp::Mod, &Type::Str, &Type::Int);
        assert_eq!(result, Some(Type::Str));
    }

    #[test]
    fn test_binop_bool_promoted_to_int() {
        let result = resolve_binop_type(&hir::BinOp::Add, &Type::Bool, &Type::Bool);
        assert_eq!(result, Some(Type::Int));
    }

    #[test]
    fn test_binop_list_concat() {
        let list_int = Type::list_of(Type::Int);
        let result = resolve_binop_type(&hir::BinOp::Add, &list_int, &list_int);
        assert_eq!(result, Some(list_int));
    }

    // === union_or_any ===

    #[test]
    fn test_union_or_any_same_types() {
        assert_eq!(union_or_any(Type::Int, Type::Int), Type::Int);
    }

    #[test]
    fn test_union_or_any_with_any() {
        assert_eq!(union_or_any(Type::Int, Type::Any), Type::Any);
        assert_eq!(union_or_any(Type::Any, Type::Str), Type::Any);
    }

    #[test]
    fn test_union_or_any_different_types() {
        let result = union_or_any(Type::Int, Type::Str);
        assert!(matches!(result, Type::Union(_)));
    }

    // === unify_element_types ===

    #[test]
    fn test_unify_empty() {
        assert_eq!(unify_element_types(vec![]), Type::Any);
    }

    #[test]
    fn test_unify_homogeneous() {
        assert_eq!(
            unify_element_types(vec![Type::Int, Type::Int, Type::Int]),
            Type::Int
        );
    }

    #[test]
    fn test_unify_heterogeneous() {
        let result = unify_element_types(vec![Type::Int, Type::Str]);
        assert!(matches!(result, Type::Union(_)));
    }

    // === unwrap_optional ===

    #[test]
    fn test_unwrap_optional_union() {
        let optional_int = Type::Union(vec![Type::Int, Type::None]);
        assert_eq!(unwrap_optional(&optional_int), Type::Int);
    }

    #[test]
    fn test_unwrap_optional_non_optional() {
        assert_eq!(unwrap_optional(&Type::Int), Type::Int);
    }

    // === infer_list_type ===

    #[test]
    fn test_infer_list_type_empty() {
        let result = infer_list_type(vec![], None);
        assert_eq!(result, Type::list_of(Type::Never));
    }

    #[test]
    fn test_infer_list_type_homogeneous() {
        let result = infer_list_type(vec![Type::Int, Type::Int], None);
        assert_eq!(result, Type::list_of(Type::Int));
    }

    // === infer_dict_type ===

    #[test]
    fn test_infer_dict_type_empty() {
        let result = infer_dict_type(vec![], vec![]);
        assert_eq!(result, Type::dict_of(Type::Never, Type::Never));
    }

    #[test]
    fn test_infer_dict_type_str_int() {
        let result = infer_dict_type(vec![Type::Str], vec![Type::Int]);
        assert_eq!(result, Type::dict_of(Type::Str, Type::Int));
    }

    // === infer_set_type ===

    #[test]
    fn test_infer_set_type_empty() {
        assert_eq!(infer_set_type(vec![]), Type::set_of(Type::Never));
    }

    #[test]
    fn test_infer_set_type_int() {
        assert_eq!(
            infer_set_type(vec![Type::Int, Type::Int]),
            Type::set_of(Type::Int)
        );
    }

    // === resolve_method_return_type ===

    #[test]
    fn test_str_method_types() {
        assert_eq!(
            resolve_method_return_type(&Type::Str, "upper"),
            Some(Type::Str)
        );
        assert_eq!(
            resolve_method_return_type(&Type::Str, "split"),
            Some(Type::list_of(Type::Str))
        );
        assert_eq!(
            resolve_method_return_type(&Type::Str, "find"),
            Some(Type::Int)
        );
        assert_eq!(
            resolve_method_return_type(&Type::Str, "startswith"),
            Some(Type::Bool)
        );
    }

    #[test]
    fn test_list_method_types() {
        let list_int = Type::list_of(Type::Int);
        assert_eq!(
            resolve_method_return_type(&list_int, "pop"),
            Some(Type::Int)
        );
        assert_eq!(
            resolve_method_return_type(&list_int, "index"),
            Some(Type::Int)
        );
        assert_eq!(
            resolve_method_return_type(&list_int, "append"),
            Some(Type::None)
        );
    }

    #[test]
    fn test_dict_method_types() {
        let dict = Type::dict_of(Type::Str, Type::Int);
        assert_eq!(resolve_method_return_type(&dict, "get"), Some(Type::Int));
        assert_eq!(
            resolve_method_return_type(&dict, "keys"),
            Some(Type::list_of(Type::Str))
        );
    }

    #[test]
    fn test_unknown_method() {
        assert_eq!(resolve_method_return_type(&Type::Int, "nonexistent"), None);
    }

    // === resolve_index_type ===

    #[test]
    fn test_index_str() {
        let expr = hir::Expr {
            kind: hir::ExprKind::Int(0),
            ty: Some(Type::Int),
            span: pyaot_utils::Span::dummy(),
        };
        assert_eq!(resolve_index_type(&Type::Str, &expr), Type::Str);
    }

    #[test]
    fn test_index_list() {
        let list = Type::list_of(Type::Int);
        let expr = hir::Expr {
            kind: hir::ExprKind::Int(0),
            ty: Some(Type::Int),
            span: pyaot_utils::Span::dummy(),
        };
        assert_eq!(resolve_index_type(&list, &expr), Type::Int);
    }

    #[test]
    fn test_index_dict() {
        let dict = Type::dict_of(Type::Str, Type::Int);
        let expr = hir::Expr {
            kind: hir::ExprKind::Int(0),
            ty: Some(Type::Int),
            span: pyaot_utils::Span::dummy(),
        };
        assert_eq!(resolve_index_type(&dict, &expr), Type::Int);
    }

    #[test]
    fn test_index_tuple_const() {
        let tuple = Type::tuple_of(vec![Type::Int, Type::Str, Type::Bool]);
        let expr = hir::Expr {
            kind: hir::ExprKind::Int(1),
            ty: Some(Type::Int),
            span: pyaot_utils::Span::dummy(),
        };
        assert_eq!(resolve_index_type(&tuple, &expr), Type::Str);
    }

    #[test]
    fn test_index_tuple_negative() {
        let tuple = Type::tuple_of(vec![Type::Int, Type::Str, Type::Bool]);
        let expr = hir::Expr {
            kind: hir::ExprKind::Int(-1),
            ty: Some(Type::Int),
            span: pyaot_utils::Span::dummy(),
        };
        assert_eq!(resolve_index_type(&tuple, &expr), Type::Bool);
    }
}
