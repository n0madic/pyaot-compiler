//! Container escape / alias analysis — the soundness side of element-type
//! inference (KNOWN_BUGS Phase 2, Root A).
//!
//! An inferred container local's element type (`xs = [1.0]` → `list[float]`) is
//! the join of the writes typeck can *see*: the literal, plus `xs.append(v)` /
//! `xs[k] = v` / … on that very local. That is only sound while every write to
//! the container object goes through the local. Once the object is reachable
//! another way — bound to a second name, passed to a function, stored in a
//! field / global / cell / other container, returned — writes through that
//! other path are invisible, and a precise element type (a `Raw(F64)` slot!)
//! would be falsified at runtime (`b = xs; b.append(7)` makes `xs[1]` an int).
//!
//! This module classifies every read of a local by its *context*:
//!
//! - **non-escaping** contexts read the container in place without retaining
//!   it or storing foreign values into it behind typeck's back (subscript,
//!   iteration, `len`, comparisons, `print`, the element-level writes typeck
//!   records itself, …);
//! - `y = x` (a plain local-to-local copy) is an **alias edge**: the two names
//!   share one object, so their element writes are merged and an escape of
//!   either escapes both;
//! - **every other context escapes** (the conservative default).
//!
//! An escaped container local keeps its container *family* but its element /
//! key / value slots are widened to `Dyn` (the always-correct `Tagged`). In
//! addition, a mutable container held *inside* another container can always be
//! aliased by reading it out (`row = m[0]; row.append(3)`), so nested mutable
//! containers in an inferred slot type always have their own element types
//! widened ([`seal_nested`]). Both are pure precision losses — invariant 2: a
//! narrower representation is an optimization, never a correctness default.

use la_arena::Idx;
use pyaot_hir::{
    ContainerOp, HirExpr, HirExprKind, HirFunction, HirRaise, HirStmt, HirTerminator,
    ResolveResult, Symbol, SymbolRef,
};
use pyaot_types::builtin_classes::{
    BUILTIN_DEFAULTDICT_CLASS_ID, BUILTIN_DEQUE_CLASS_ID, BUILTIN_DICT_CLASS_ID,
    BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID,
    BUILTIN_TUPLE_VAR_CLASS_ID,
};
use pyaot_types::SemTy;
use pyaot_utils::StringInterner;

/// Container methods whose *receiver* is used in place: they read it, or
/// mutate it only with values typeck records as element writes itself
/// (`append`/`add`/`insert`/`extend`/`setdefault`/`update`), or only remove /
/// permute existing elements.
const IN_PLACE_RECV: &[&str] = &[
    "append",
    "add",
    "insert",
    "extend",
    "setdefault",
    "update",
    "pop",
    "popitem",
    "remove",
    "discard",
    "clear",
    "index",
    "count",
    "get",
    "keys",
    "values",
    "items",
    "copy",
    "sort",
    "reverse",
    "union",
    "intersection",
    "difference",
    "symmetric_difference",
    "issubset",
    "issuperset",
    "isdisjoint",
    "difference_update",
    "intersection_update",
    "symmetric_difference_update",
    "__contains__",
    "__len__",
];

/// Methods whose positional *arguments* are only read element-wise (their
/// elements are copied or compared, the argument object itself is never
/// retained).
const READ_ONLY_ARGS: &[&str] = &[
    "extend",
    "update",
    "union",
    "intersection",
    "difference",
    "symmetric_difference",
    "issubset",
    "issuperset",
    "isdisjoint",
    "difference_update",
    "intersection_update",
    "symmetric_difference_update",
    "join",
    "index",
    "count",
    "remove",
    "discard",
    "__contains__",
];

/// The per-function escape facts.
pub(crate) struct EscapeInfo {
    /// `escaped[i]`: local `i` (or a local it is aliased with) may hold a
    /// container reachable through a path typeck does not see.
    pub(crate) escaped: Vec<bool>,
    /// `alias_class[i]`: the representative of local `i`'s alias class (locals
    /// joined by `y = x` copies). Element writes are shared within a class.
    pub(crate) alias_class: Vec<usize>,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Analyse `func` (see the module docs).
pub(crate) fn analyze(
    func: &HirFunction,
    resolve: &ResolveResult,
    interner: &StringInterner,
) -> EscapeInfo {
    let n = func.locals.len();
    let mut escaped = vec![false; n];
    let mut parent: Vec<usize> = (0..n).collect();

    let as_local = |e: Idx<HirExpr>| -> Option<usize> {
        match func.exprs[e].kind {
            HirExprKind::Local(lid) => Some(lid.index()),
            _ => None,
        }
    };
    let esc = |e: Idx<HirExpr>, escaped: &mut Vec<bool>| {
        if let Some(i) = as_local(e) {
            escaped[i] = true;
        }
    };

    // ── expression contexts ──
    for (_, expr) in func.exprs.iter() {
        match &expr.kind {
            HirExprKind::Call { callee, args } => {
                let builtin = match func.exprs[*callee].kind {
                    HirExprKind::Name(SymbolRef::Resolved(sid)) => match resolve.symbol(sid) {
                        Symbol::BuiltinPrint | Symbol::BuiltinRange | Symbol::Builtin(_) => {
                            Some(None)
                        }
                        Symbol::Container(op) => Some(Some(op)),
                        _ => None,
                    },
                    _ => None,
                };
                match builtin {
                    // A builtin function reads its container arguments; the
                    // populating container ops store args[1..].
                    Some(Some(op)) if is_store_op(op) => {
                        for a in args.iter().skip(1) {
                            esc(*a, &mut escaped);
                        }
                    }
                    Some(_) => {}
                    // A user function / class constructor / callable value may
                    // retain or mutate its arguments.
                    None => {
                        esc(*callee, &mut escaped);
                        for a in args {
                            esc(*a, &mut escaped);
                        }
                    }
                }
            }
            HirExprKind::ContainerExpr { op, args } => {
                if is_store_op(*op) {
                    for a in args.iter().skip(1) {
                        esc(*a, &mut escaped);
                    }
                }
            }
            HirExprKind::MethodCall {
                recv,
                method_name,
                args,
                kwargs,
            } => {
                let name = interner.resolve(*method_name);
                if !IN_PLACE_RECV.contains(&name) {
                    esc(*recv, &mut escaped);
                }
                if !READ_ONLY_ARGS.contains(&name) {
                    for a in args {
                        esc(*a, &mut escaped);
                    }
                }
                for (_, v) in kwargs {
                    esc(*v, &mut escaped);
                }
            }
            HirExprKind::CallValue {
                callee,
                args,
                kwargs,
            } => {
                esc(*callee, &mut escaped);
                esc(*args, &mut escaped);
                if let Some(k) = kwargs {
                    esc(*k, &mut escaped);
                }
            }
            HirExprKind::MethodCallValue {
                recv, args, kwargs, ..
            } => {
                esc(*recv, &mut escaped);
                esc(*args, &mut escaped);
                if let Some(k) = kwargs {
                    esc(*k, &mut escaped);
                }
            }
            HirExprKind::ListLit { elems }
            | HirExprKind::TupleLit { elems }
            | HirExprKind::SetLit { elems } => {
                for e in elems {
                    esc(*e, &mut escaped);
                }
            }
            HirExprKind::DictLit { pairs } => {
                for (k, v) in pairs {
                    esc(*k, &mut escaped);
                    esc(*v, &mut escaped);
                }
            }
            HirExprKind::CallRuntime { args, .. } => {
                for a in args.iter().flatten() {
                    esc(*a, &mut escaped);
                }
            }
            HirExprKind::GenericConstruct { args, .. } => {
                for a in args {
                    esc(*a, &mut escaped);
                }
            }
            HirExprKind::MakeClosure { captures, .. } => {
                for c in captures {
                    esc(*c, &mut escaped);
                }
            }
            HirExprKind::MakeCell { init: Some(i) } => esc(*i, &mut escaped),
            HirExprKind::GetAttrByName {
                default: Some(d), ..
            } => esc(*d, &mut escaped),
            HirExprKind::GenQuery { value: Some(v), .. } => esc(*v, &mut escaped),
            // Read-only / in-place contexts: operators, comparisons, subscript
            // and slice reads, attribute reads, identity / isinstance tests,
            // formatting, `sum`.
            _ => {}
        }
    }

    // ── statement / terminator contexts ──
    for (_, block) in func.blocks.iter() {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Assign { target, value } => {
                    if let Some(src) = as_local(*value) {
                        let (a, b) = (find(&mut parent, target.index()), find(&mut parent, src));
                        if a != b {
                            parent[a] = b;
                        }
                    }
                }
                HirStmt::SetItem { index, value, .. } => {
                    esc(*index, &mut escaped);
                    esc(*value, &mut escaped);
                }
                HirStmt::SetAttr { value, .. }
                | HirStmt::CellSet { value, .. }
                | HirStmt::GlobalSet { value, .. }
                | HirStmt::ContainerPush { value, .. }
                | HirStmt::GenSetLocal { value, .. } => esc(*value, &mut escaped),
                HirStmt::ContainerInsert { key, value, .. } => {
                    esc(*key, &mut escaped);
                    esc(*value, &mut escaped);
                }
                HirStmt::Raise(r) => match r {
                    HirRaise::Custom { args, .. } => {
                        for a in args {
                            esc(*a, &mut escaped);
                        }
                    }
                    HirRaise::Instance { value } => esc(*value, &mut escaped),
                    _ => {}
                },
                // `SetSlice` copies the value's elements; `Expr` / `Print` /
                // `Assert` / `DelItem` / `DelSlice` only read.
                _ => {}
            }
        }
        if let HirTerminator::Return(Some(v)) = block.term {
            esc(v, &mut escaped);
        }
    }

    // An escape anywhere in an alias class escapes the whole class.
    let alias_class: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    let mut class_escaped = vec![false; n];
    for i in 0..n {
        if escaped[i] {
            class_escaped[alias_class[i]] = true;
        }
    }
    let escaped = (0..n).map(|i| class_escaped[alias_class[i]]).collect();
    EscapeInfo {
        escaped,
        alias_class,
    }
}

/// The populating container ops: arg 0 is the container, the rest are values
/// stored into it.
fn is_store_op(op: ContainerOp) -> bool {
    matches!(
        op,
        ContainerOp::ListPush
            | ContainerOp::ListSet
            | ContainerOp::DictSet
            | ContainerOp::SetAdd
            | ContainerOp::TupleSet
    )
}

fn is_mutable_container(base: pyaot_utils::ClassId) -> bool {
    base == BUILTIN_LIST_CLASS_ID
        || base == BUILTIN_DICT_CLASS_ID
        || base == BUILTIN_SET_CLASS_ID
        || base == BUILTIN_DEFAULTDICT_CLASS_ID
        || base == BUILTIN_DEQUE_CLASS_ID
}

fn is_tuple(base: pyaot_utils::ClassId) -> bool {
    base == BUILTIN_TUPLE_CLASS_ID || base == BUILTIN_TUPLE_VAR_CLASS_ID
}

/// Widen the element / key / value slots of every mutable container in `t`
/// (reached through tuples and unions) to `Dyn` — the type of a container
/// whose writes typeck cannot all see. `Never` slots (an empty literal not
/// yet joined) stay `Never` so the in-progress join is not poisoned; they
/// become `Dyn` at materialization like any unconstrained slot.
pub(crate) fn erase_elems(t: &SemTy) -> SemTy {
    match t {
        SemTy::Generic { base, args } if is_mutable_container(*base) => SemTy::Generic {
            base: *base,
            args: args
                .iter()
                .map(|a| if *a == SemTy::Never { SemTy::Never } else { SemTy::Dyn })
                .collect(),
        },
        SemTy::Generic { base, args } if is_tuple(*base) => SemTy::Generic {
            base: *base,
            args: args.iter().map(erase_elems).collect(),
        },
        SemTy::Union(ms) => SemTy::Union(ms.iter().map(erase_elems).collect()),
        other => other.clone(),
    }
}

/// Widen every mutable container held *inside* a container slot of `t`
/// (see the module docs): `list[list[float]]` → `list[list[Dyn]]`, while
/// `list[float]` and `tuple[float, int]` are unchanged.
pub(crate) fn seal_nested(t: &SemTy) -> SemTy {
    match t {
        SemTy::Generic { base, args } if is_mutable_container(*base) || is_tuple(*base) => {
            SemTy::Generic {
                base: *base,
                args: args.iter().map(erase_elems).collect(),
            }
        }
        SemTy::Union(ms) => SemTy::Union(ms.iter().map(seal_nested).collect()),
        other => other.clone(),
    }
}
