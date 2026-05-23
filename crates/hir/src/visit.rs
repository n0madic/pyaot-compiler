//! HIR expression / pattern walker — single exhaustive source of structural
//! recursion shape.
//!
//! ## Why this exists
//!
//! Several lowering passes need to recurse through an HIR expression tree
//! and visit every sub-`ExprId`. Each used to enumerate the 40+ `ExprKind`
//! variants by hand, and several had a trailing `_ => {}` wildcard arm
//! that silently dropped new variants. The code-review series surfaced
//! three concrete misses caused by this pattern:
//!
//! * `closure_scan::scan_expr_for_calls` skipped `Slice` / `SuperCall` /
//!   `StdlibCall` / `Yield` / `IterHasNext` / `MatchPattern` / `Closure`
//!   captures, so inline-call type harvesting missed those positions.
//! * `phase4_safe_scan::mark_escaped_in_expr` had the same wildcard,
//!   though most of its arms were already explicit.
//! * `MatchPattern` arms in three scanners only walked `subject` and
//!   ignored the pattern's nested `ExprId` leaves
//!   (`MatchValue` / `MatchMapping.keys` / `MatchClass.cls`).
//!
//! ## API
//!
//! [`for_each_subexpr_id`] takes a closure and invokes it once per direct
//! `ExprId` child of an expression — every position that points to
//! another HIR expression node. Container variants (`List` / `Tuple` /
//! `Set` / `Dict`) yield each contained `ExprId`; `Call` yields the
//! callee, every positional arg (regular and starred), every keyword
//! value, and the `**kwargs_unpack` operand; leaf variants (`Int` /
//! `FuncRef` / `ClassRef` / ...) yield nothing.
//!
//! [`walk_pattern_subexprs`] does the same for the nested `ExprId`
//! positions inside a `match` `Pattern` (`MatchValue` / `MatchMapping.keys`
//! / `MatchClass.cls`), recursing through sub-patterns.
//!
//! Both functions use an exhaustive `match` (no `_` arm) so adding a new
//! `ExprKind` or `Pattern` variant is a compile error here — every
//! scanner that uses these helpers inherits the fix as soon as the
//! variant is added to this file.
//!
//! ## When to use the helpers vs custom recursion
//!
//! Scanners that need bespoke per-variant logic (e.g., `Call.func` is a
//! direct-call position, not address-taken; `BuiltinCall` `key=` kwarg
//! triggers HOF marking) still need explicit match arms for those
//! variants. They can fall through to `for_each_subexpr_id` for the
//! "default: just recurse" path instead of writing a wildcard. The
//! result is exhaustive *at the helper level* — the scanner's match
//! covers only the variants it actually treats specially, and the
//! helper's exhaustive match covers everything else.

use crate::{Expr, ExprId, ExprKind, GeneratorIntrinsic, Module, Pattern};

/// Invoke `f` once for every direct `ExprId` child of `expr`. The
/// recursion is single-layer — `f` is given each child's `ExprId` and
/// is responsible for further descent (typically by looking up
/// `&module.exprs[id]` and calling `for_each_subexpr_id` again).
///
/// Exhaustive on `ExprKind` — adding a new variant requires updating
/// this function, which surfaces as a compile error.
pub fn for_each_subexpr_id<F: FnMut(ExprId)>(expr: &Expr, _module: &Module, mut f: F) {
    match &expr.kind {
        // Leaves — no sub-expressions.
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Bytes(_)
        | ExprKind::None
        | ExprKind::NotImplemented
        | ExprKind::Var(_)
        | ExprKind::FuncRef(_)
        | ExprKind::ClassRef(_)
        | ExprKind::ClassAttrRef { .. }
        | ExprKind::TypeRef(_)
        | ExprKind::ImportedRef { .. }
        | ExprKind::ModuleAttr { .. }
        | ExprKind::BuiltinRef(_)
        | ExprKind::StdlibAttr(_)
        | ExprKind::StdlibConst(_)
        | ExprKind::ExcCurrentValue => {}

        // Single sub-expression.
        ExprKind::UnOp { operand, .. } => f(*operand),
        ExprKind::Attribute { obj, .. } => f(*obj),
        ExprKind::FormatSpec { value, .. } => f(*value),
        ExprKind::IterHasNext(id) => f(*id),
        ExprKind::Yield(opt) => {
            if let Some(id) = opt {
                f(*id);
            }
        }

        // Two sub-expressions.
        ExprKind::BinOp { left, right, .. }
        | ExprKind::Compare { left, right, .. }
        | ExprKind::LogicalOp { left, right, .. } => {
            f(*left);
            f(*right);
        }
        ExprKind::Index { obj, index } => {
            f(*obj);
            f(*index);
        }

        // Three sub-expressions.
        ExprKind::IfExpr {
            cond,
            then_val,
            else_val,
        } => {
            f(*cond);
            f(*then_val);
            f(*else_val);
        }

        // Slice — obj + up to three optional sub-expressions.
        ExprKind::Slice {
            obj,
            start,
            end,
            step,
        } => {
            f(*obj);
            for id in [start, end, step].into_iter().flatten() {
                f(*id);
            }
        }

        // Container literals.
        ExprKind::List(items) | ExprKind::Tuple(items) | ExprKind::Set(items) => {
            for id in items {
                f(*id);
            }
        }
        ExprKind::Dict(pairs) => {
            for (k, v) in pairs {
                f(*k);
                f(*v);
            }
        }

        // Calls — callee, positional args (regular + starred), kwarg
        // values, and the **kwargs_unpack operand.
        ExprKind::Call {
            func,
            args,
            kwargs,
            kwargs_unpack,
        } => {
            f(*func);
            for arg in args {
                match arg {
                    crate::CallArg::Regular(id) | crate::CallArg::Starred(id) => f(*id),
                }
            }
            for kw in kwargs {
                f(kw.value);
            }
            if let Some(id) = kwargs_unpack {
                f(*id);
            }
        }
        ExprKind::BuiltinCall { args, kwargs, .. } => {
            for id in args {
                f(*id);
            }
            for kw in kwargs {
                f(kw.value);
            }
        }
        ExprKind::MethodCall {
            obj, args, kwargs, ..
        } => {
            f(*obj);
            for id in args {
                f(*id);
            }
            for kw in kwargs {
                f(kw.value);
            }
        }
        ExprKind::SuperCall { args, .. } => {
            for id in args {
                f(*id);
            }
        }
        ExprKind::StdlibCall { args, .. } => {
            for id in args {
                f(*id);
            }
        }

        // Closure — captures only. The `func` body is a separate
        // function; scanners that want to descend into it must look up
        // `module.func_defs[func]` explicitly.
        ExprKind::Closure { captures, .. } => {
            for id in captures {
                f(*id);
            }
        }

        // Match pattern — subject only at this layer. Nested ExprIds
        // inside `pattern` are exposed via `walk_pattern_subexprs`,
        // which callers should invoke separately when they need to
        // reach pattern leaves.
        ExprKind::MatchPattern { subject, .. } => f(*subject),

        // Generator intrinsics — exhaustively cover every variant
        // that carries an ExprId. Post-desugar artifact; pre-desugar
        // scanners never see these.
        ExprKind::GeneratorIntrinsic(intr) => match intr {
            GeneratorIntrinsic::GetState(id)
            | GeneratorIntrinsic::SetExhausted(id)
            | GeneratorIntrinsic::IsExhausted(id)
            | GeneratorIntrinsic::GetSentValue(id)
            | GeneratorIntrinsic::IterNextNoExc(id)
            | GeneratorIntrinsic::IterIsExhausted(id) => f(*id),
            GeneratorIntrinsic::SetState { gen, .. } | GeneratorIntrinsic::GetLocal { gen, .. } => {
                f(*gen)
            }
            GeneratorIntrinsic::SetLocal { gen, value, .. } => {
                f(*gen);
                f(*value);
            }
            GeneratorIntrinsic::Create { .. } => {}
        },
    }
}

/// Invoke `f` once for every direct `ExprId` leaf inside `pattern`,
/// recursing through sub-patterns. Useful for forward-looking
/// `MatchPattern` walkers — currently the frontend does not emit
/// `MatchPattern` (Stage 2 of §1.11 S1.17b), but every scanner that
/// uses this helper is exhaustive against future emission.
///
/// Exhaustive on `Pattern` — adding a new variant requires updating
/// this function.
pub fn walk_pattern_subexprs<F: FnMut(ExprId)>(pattern: &Pattern, mut f: F) {
    walk_pattern_subexprs_inner(pattern, &mut f)
}

fn walk_pattern_subexprs_inner<F: FnMut(ExprId)>(pattern: &Pattern, f: &mut F) {
    match pattern {
        Pattern::MatchValue(id) => f(*id),
        Pattern::MatchSingleton(_) | Pattern::MatchStar(_) => {}
        Pattern::MatchAs { pattern, .. } => {
            if let Some(inner) = pattern {
                walk_pattern_subexprs_inner(inner, f);
            }
        }
        Pattern::MatchSequence { patterns } | Pattern::MatchOr(patterns) => {
            for p in patterns {
                walk_pattern_subexprs_inner(p, f);
            }
        }
        Pattern::MatchMapping { keys, patterns, .. } => {
            for k in keys {
                f(*k);
            }
            for p in patterns {
                walk_pattern_subexprs_inner(p, f);
            }
        }
        Pattern::MatchClass {
            cls,
            patterns,
            kwd_patterns,
            ..
        } => {
            f(*cls);
            for p in patterns {
                walk_pattern_subexprs_inner(p, f);
            }
            for p in kwd_patterns {
                walk_pattern_subexprs_inner(p, f);
            }
        }
    }
}
