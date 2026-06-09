//! # typeck — one constraint-based type inference
//!
//! Type inference is ONE algorithm in three phases — never a fixpoint of
//! mutually recursive monotone passes:
//!
//! 1. **collect** — a walk over HIR emits [`Constraint`]s.
//! 2. **solve** — fold the constraints over [`pyaot_types::TypeLattice`].
//! 3. **materialize** — write the solved [`SemTy`] back onto each HIR node,
//!    *preserving* any concrete type already present (frontend-assigned literal
//!    types) and defaulting only genuinely-unconstrained nodes to `Dyn`.
//!
//! Inference finishes BEFORE lowering and does not leak into it. Representation
//! is decided by `repr_of` at the lowering boundary. Because the tagged baseline
//! is always correct, inference precision is a performance lever, not a
//! correctness requirement: a node left `Dyn` (→ `Tagged` → `rt_*` dispatch)
//! still compiles correctly, just to slower code.
//!
//! ## Scope
//!
//! Phase 2 emits equalities for literals and the structurally-determined results
//! (`Compare`/`not` → `Bool`). Arithmetic and call result types are left `Dyn`:
//! they flow through the tagged baseline (`rt_obj_*` / `rt_print_obj`), which is
//! correct for every operand mix. Sharpening them is a later optimization.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use la_arena::Idx;

use pyaot_diagnostics::Result;
use pyaot_hir::{HirExpr, HirExprKind, HirFunction, HirModule, ResolveResult, UnaryOp};
use pyaot_types::{SemTy, TypeLattice};

/// A single typing constraint. Phase 2 emits only equalities of an expression
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

/// Emit an equality for every node whose type is structurally determined.
fn collect(func: &HirFunction) -> Vec<Constraint> {
    let mut constraints = Vec::new();
    for (idx, expr) in func.exprs.iter() {
        let ty = match &expr.kind {
            HirExprKind::StrLit(_) => Some(SemTy::Str),
            HirExprKind::IntLit(_) | HirExprKind::BigIntLit(_) => Some(SemTy::Int),
            HirExprKind::FloatLit(_) => Some(SemTy::Float),
            HirExprKind::BoolLit(_) => Some(SemTy::Bool),
            HirExprKind::NoneLit => Some(SemTy::NoneTy),
            HirExprKind::Compare { .. } => Some(SemTy::Bool),
            HirExprKind::Unary { op: UnaryOp::Not, .. } => Some(SemTy::Bool),
            _ => None,
        };
        if let Some(ty) = ty {
            constraints.push(Constraint::Eq { expr: idx, ty });
        }
    }
    constraints
}

/// Fold constraints into a per-node solution. Multiple equalities on one node
/// are reconciled with the lattice `join` (least upper bound).
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

/// Write solved types back; nodes without a solution keep their existing type
/// (frontend-assigned for literals), so this never clobbers a concrete type
/// down to `Dyn`.
fn materialize(func: &mut HirFunction, solution: &HashMap<Idx<HirExpr>, SemTy>) {
    for (idx, expr) in func.exprs.iter_mut() {
        if let Some(ty) = solution.get(&idx) {
            expr.ty = ty.clone();
        }
    }
}
