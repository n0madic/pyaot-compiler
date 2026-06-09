//! # typeck — one constraint-based type inference
//!
//! Type inference is ONE algorithm in three phases — never a fixpoint of
//! mutually recursive monotone passes:
//!
//! 1. **collect** — a walk over HIR emits [`Constraint`]s.
//! 2. **solve** — fold the constraints over [`pyaot_types::TypeLattice`].
//! 3. **materialize** — write the solved [`SemTy`] back onto each HIR node,
//!    defaulting any node left unconstrained to [`SemTy::Dyn`].
//!
//! Inference finishes BEFORE lowering and does not leak into it. Representation
//! is NOT decided here — that is `repr_of` at the lowering boundary. Because the
//! tagged baseline is always correct, inference precision is a performance lever,
//! not a correctness requirement: an underpowered solver yields slower code, not
//! wrong code.
//!
//! ## Phase 1 scope
//!
//! `collect` emits a single equality `Eq(expr, Str)` for each string literal;
//! `solve` joins duplicate constraints over the lattice; `materialize` defaults
//! everything else to `Dyn`. This is the decoupling proof in microcosm: even if
//! `"hello"` were left `Dyn` (→ `Tagged` → `rt_print_obj`), the program would
//! still be correct, just slower. Subtype/consistency constraints, parameter and
//! return-flow constraints, and the union-find core are reserved for Phase 3.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use la_arena::Idx;

use pyaot_diagnostics::Result;
use pyaot_hir::{HirExpr, HirExprKind, HirFunction, HirModule, ResolveResult};
use pyaot_types::{SemTy, TypeLattice};

/// A single typing constraint. Phase 1 emits only equalities of an expression
/// node to a concrete type; subtype (`≤`) and gradual `consistent` constraints
/// are reserved for the real solver.
enum Constraint {
    Eq { expr: Idx<HirExpr>, ty: SemTy },
}

/// Run inference over every function, mutating each node's [`SemTy`] in place.
pub fn infer(module: &mut HirModule, _resolve: &ResolveResult) -> Result<()> {
    for func in module.functions.iter_mut() {
        let constraints = collect(func);
        let solution = solve(constraints);
        materialize(func, &solution);
    }
    Ok(())
}

/// Phase 1: a string literal node is constrained equal to `Str`.
fn collect(func: &HirFunction) -> Vec<Constraint> {
    let mut constraints = Vec::new();
    for (idx, expr) in func.exprs.iter() {
        if let HirExprKind::StrLit(_) = expr.kind {
            constraints.push(Constraint::Eq {
                expr: idx,
                ty: SemTy::Str,
            });
        }
    }
    constraints
}

/// Fold constraints into a per-node solution. Multiple equalities on one node
/// are reconciled with the lattice `join` (least upper bound), which is the
/// shape the real worklist solver keeps.
fn solve(constraints: Vec<Constraint>) -> HashMap<Idx<HirExpr>, SemTy> {
    let mut solution: HashMap<Idx<HirExpr>, SemTy> = HashMap::new();
    for Constraint::Eq { expr, ty } in constraints {
        solution
            .entry(expr)
            .and_modify(|existing| *existing = existing.join(&ty))
            .or_insert(ty);
    }
    solution
}

/// Write solved types back; unconstrained nodes default to the always-correct
/// `Dyn` (→ `Tagged`).
fn materialize(func: &mut HirFunction, solution: &HashMap<Idx<HirExpr>, SemTy>) {
    for (idx, expr) in func.exprs.iter_mut() {
        expr.ty = solution.get(&idx).cloned().unwrap_or(SemTy::Dyn);
    }
}
