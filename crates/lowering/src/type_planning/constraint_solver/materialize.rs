//! Solver env → `LoweringSeedInfo` adapter.
//!
//! The solver's [`Env`] is the source of truth for type inference; the rest
//! of the compiler reads from a handful of legacy contract outputs that the
//! materializer fills from env values. S4 brings the materializer to full
//! contract coverage — the remaining wire-in (S5) only has to plumb these
//! maps into `Lowering`'s fields.
//!
//! ## Contract outputs
//!
//! - [`MaterializeOutput::expr_types`] — per `ExprId` type. Caches every
//!   non-`Any` non-`Union` result, matching the legacy gate at
//!   `mod.rs:395` ("don't cache narrowing-sensitive types").
//! - [`MaterializeOutput::func_return_types`] — per `FuncId` return type.
//! - [`MaterializeOutput::base_var_types`] — per `VarId` base type
//!   (pre-narrowing). `NarrowingAnalysis` at lowering time overlays
//!   path-sensitive isinstance narrowing on top of this.
//! - [`MaterializeOutput::lambda_param_type_hints`] —
//!   `(FuncId, param_ix) → Type`. Powers cross-function inference for
//!   unannotated lambda/closure parameters: collected from call-site
//!   `LambdaParamHint` constraints.
//! - [`MaterializeOutput::closure_capture_types`] —
//!   `(FuncId, slot) → Type`. Per-closure upvalue types used by codegen
//!   when emitting cell stores.
//! - [`MaterializeOutput::refined_class_field_types`] —
//!   `(ClassId, name) → Type`. Cross-instance refined field types from
//!   every observed `self.x = …` store and `ClassName.attr = …` write.
//! - [`MaterializeOutput::func_yield_types`] — per generator-`FuncId`
//!   yielded element type (NOT wrapped in `Iterator(_)` — the S5 layer
//!   wraps when seeding `func_return_types`).
//!
//! ## Not in scope
//!
//! Writing `hir::Expr.ty` in place — handled by `apply_to_module` at S5
//! wire-in time, when the solver replaces the legacy planner.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId, InternedString, VarId};

use super::env::Env;
use super::key::TypeKey;

/// Materialization output — the full `LoweringSeedInfo` contract surface.
/// S5 wire-in pulls these maps into the corresponding `Lowering` fields.
#[derive(Debug, Default)]
pub struct MaterializeOutput {
    /// `ExprId → Type`. Filtered by the cache gate (no `Any`, no `Union`)
    /// so isinstance narrowing at lowering time still works.
    pub expr_types: IndexMap<hir::ExprId, Type>,
    /// `FuncId → Type` for every function the solver computed a return
    /// type for. Unbound functions (whose `FuncReturn(fid)` key never
    /// received a `Return` constraint) are absent from the map.
    pub func_return_types: IndexMap<FuncId, Type>,
    /// `VarId → Type` base (pre-narrowing) types. Maps to the legacy
    /// `LoweringSeedInfo::base_var_types`.
    pub base_var_types: IndexMap<VarId, Type>,
    /// `(FuncId, param_ix) → Type` lambda/closure parameter hints from
    /// call sites. Maps to `Lowering::closures.lambda_param_type_hints`.
    pub lambda_param_type_hints: IndexMap<(FuncId, usize), Type>,
    /// `(FuncId, slot) → Type` closure capture types. Maps to
    /// `Lowering::closures.closure_capture_types`.
    pub closure_capture_types: IndexMap<(FuncId, usize), Type>,
    /// `(ClassId, name) → Type` refined class field types. Maps to
    /// `LoweringSeedInfo::refined_class_field_types`.
    pub refined_class_field_types: IndexMap<(ClassId, InternedString), Type>,
    /// `FuncId → Type` generator yield element type. NOT wrapped in
    /// `Iterator(_)`; S5 wire-in wraps when seeding `func_return_types`.
    pub func_yield_types: IndexMap<FuncId, Type>,
}

/// Materialize solver env into a fresh [`MaterializeOutput`]. Pure function
/// of the env; does not consult the HIR module.
pub fn materialize(env: &Env) -> MaterializeOutput {
    let mut out = MaterializeOutput::default();

    for (key, ty) in env.iter() {
        // Universally skip bottom (`Never`) entries: they signal that the
        // key was registered as a constraint target but never received a
        // value. Downstream callers fall back to safe defaults
        // (`Type::None`, `Type::Any`) on absence — emitting `Never`
        // would mistype the program.
        if matches!(ty, Type::Never) {
            continue;
        }

        match *key {
            TypeKey::Expr(eid) => {
                // Apply the legacy cache gate. `Any` and `Union` types
                // are narrowing-sensitive — caching them as an `Expr.ty`
                // would force later isinstance frames to widen back to
                // the pre-narrowing Union. Concrete types are stable
                // (narrowing never widens them) and safe to cache.
                if !matches!(ty, Type::Any) && !ty.is_union() {
                    out.expr_types.insert(eid, ty.clone());
                }
            }
            TypeKey::FuncReturn(fid) => {
                out.func_return_types.insert(fid, ty.clone());
            }
            TypeKey::Var(v) => {
                out.base_var_types.insert(v, ty.clone());
            }
            TypeKey::LambdaParam(fid, ix) => {
                out.lambda_param_type_hints.insert((fid, ix), ty.clone());
            }
            TypeKey::Capture(fid, slot) => {
                out.closure_capture_types.insert((fid, slot), ty.clone());
            }
            TypeKey::ClassField(cid, name) => {
                out.refined_class_field_types
                    .insert((cid, name), ty.clone());
            }
            TypeKey::FuncYield(fid) => {
                out.func_yield_types.insert(fid, ty.clone());
            }
            // Internal solver scaffolding — never materialized.
            TypeKey::Comp(_, _) | TypeKey::Meta(_) => {}
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::super::collect::collect_with_empty_interner;
    use super::super::solve::{PermissiveCtx, Solver};
    use super::*;
    use pyaot_hir as hir;
    use pyaot_utils::{FuncId, HirBlockId, Span, StringInterner, VarId};

    fn expr(kind: hir::ExprKind) -> hir::Expr {
        hir::Expr {
            kind,
            ty: None,
            span: Span::dummy(),
        }
    }

    fn stmt(kind: hir::StmtKind) -> hir::Stmt {
        hir::Stmt {
            kind,
            span: Span::dummy(),
        }
    }

    fn make_function(
        name: pyaot_utils::InternedString,
        fid: FuncId,
        params: Vec<hir::Param>,
        return_type: Option<Type>,
        entry_block: HirBlockId,
        block: hir::HirBlock,
    ) -> hir::Function {
        let mut blocks = indexmap::IndexMap::new();
        blocks.insert(entry_block, block);
        hir::Function {
            id: fid,
            name,
            params,
            return_type,
            span: Span::dummy(),
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: false,
            method_kind: hir::MethodKind::Static,
            is_abstract: false,
            blocks,
            entry_block,
            try_scopes: Vec::new(),
        }
    }

    fn make_block(
        bid: HirBlockId,
        stmts: Vec<hir::StmtId>,
        term: hir::HirTerminator,
    ) -> hir::HirBlock {
        hir::HirBlock {
            id: bid,
            stmts,
            terminator: term,
            loop_depth: 0,
            handler_depth: 0,
        }
    }

    /// Full pipeline test: `def f(x: int) -> int: y = x + 1; return y`.
    /// Verifies materialize produces the right `expr.ty` cache + return type.
    #[test]
    fn materialize_simple_int_function() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let x_name = interner.intern("x");

        let mut m = hir::Module::new(interner.intern("test"));
        let x_var = VarId::new(0);
        let y_var = VarId::new(1);

        let e_x = m.exprs.alloc(expr(hir::ExprKind::Var(x_var)));
        let e_one = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_add = m.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left: e_x,
            right: e_one,
        }));
        let e_y = m.exprs.alloc(expr(hir::ExprKind::Var(y_var)));
        let s_bind = m.stmts.alloc(stmt(hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(y_var),
            value: e_add,
            type_hint: None,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![s_bind], hir::HirTerminator::Return(Some(e_y)));
        let fid = FuncId::new(0);
        let func = make_function(
            f_name,
            fid,
            vec![hir::Param {
                name: x_name,
                var: x_var,
                ty: Some(Type::Int),
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            Some(Type::Int),
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        let out = materialize(solver.env());
        assert_eq!(out.expr_types.get(&e_x), Some(&Type::Int));
        assert_eq!(out.expr_types.get(&e_one), Some(&Type::Int));
        assert_eq!(out.expr_types.get(&e_add), Some(&Type::Int));
        assert_eq!(out.expr_types.get(&e_y), Some(&Type::Int));
        assert_eq!(out.func_return_types.get(&fid), Some(&Type::Int));
    }

    /// Cache gate: Union expressions are NOT inserted into `expr_types`.
    #[test]
    fn materialize_skips_union_in_expr_cache() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        let s = interner.intern("a");
        let e_cond = m.exprs.alloc(expr(hir::ExprKind::Bool(true)));
        let e_then = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_else = m.exprs.alloc(expr(hir::ExprKind::Str(s)));
        let e_if = m.exprs.alloc(expr(hir::ExprKind::IfExpr {
            cond: e_cond,
            then_val: e_then,
            else_val: e_else,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], hir::HirTerminator::Return(Some(e_if)));
        let fid = FuncId::new(1);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        let out = materialize(solver.env());
        // The IfExpr key is a Union — skipped from cache by the gate.
        assert!(
            !out.expr_types.contains_key(&e_if),
            "Union types must be skipped by the cache gate"
        );
        // Branches themselves are concrete and DO appear.
        assert_eq!(out.expr_types.get(&e_then), Some(&Type::Int));
        assert_eq!(out.expr_types.get(&e_else), Some(&Type::Str));
        // But func_return_types DOES record the Union (it's not narrowing-sensitive
        // there).
        match out.func_return_types.get(&fid) {
            Some(Type::Union(_)) => {}
            other => panic!("expected Union return type, got {other:?}"),
        }
    }

    /// Cache gate: Any expressions are NOT inserted into `expr_types`.
    #[test]
    fn materialize_skips_any_in_expr_cache() {
        // Construct a function whose return ends up as Any via the
        // FlowsInto Any → expr edge.
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        let e_any = m.exprs.alloc(expr(hir::ExprKind::Int(0))); // placeholder; we override its type below by hand
        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], hir::HirTerminator::Return(Some(e_any)));
        let fid = FuncId::new(2);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        // Manually seed Expr(e_any) = Any to verify the gate.
        solver.add(super::super::vocab::Constraint::Concrete(
            TypeKey::Expr(e_any),
            Type::Any,
        ));
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        let out = materialize(solver.env());
        assert!(
            !out.expr_types.contains_key(&e_any),
            "Any types must be skipped by the cache gate"
        );
    }

    /// FuncReturn unbound (no Return constraint) is absent from output.
    #[test]
    fn materialize_skips_never_func_return() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        // Block has no Return terminator — Reraise as a stand-in for
        // "no return path".
        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], hir::HirTerminator::Reraise);
        let fid = FuncId::new(3);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        let out = materialize(solver.env());
        assert!(
            !out.func_return_types.contains_key(&fid),
            "functions with no Return path must not appear in func_return_types"
        );
    }
}
