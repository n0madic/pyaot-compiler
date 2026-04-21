//! Inter-procedural `NotImplemented` analysis.
//!
//! Computes `func_may_return_not_implemented(f)` — a predicate that is
//! `true` iff `f` (a dunder or any helper it delegates to) can return the
//! `NotImplemented` sentinel on some reachable path. Used by:
//!
//! - `expressions::operators::binary_ops` — gate §3.3.8 fallback branch
//!   in `lower_binop` / `dispatch_class_binop`.
//! - `expressions::builtins::reductions` (Area C §C.3) — decide whether
//!   the fold-loop body should emit the compare-and-branch for every
//!   iteration.
//!
//! The analysis is lazy and memoised on `NiAnalysis::cache` (see
//! `context/mod.rs`). Call graphs are traversed through direct
//! `Return(Call(..))` / `Return(MethodCall(..))` tail calls; cycles are
//! broken by the `Computing` marker (treated as `No` on re-entry — a
//! tentative answer that is finalised when the outermost call unwinds).
//! Unresolved callees (cross-module, virtual/Union receivers) are
//! conservatively assumed to return `NotImplemented` — one compare+branch
//! at the call site is cheap, a false negative would silently produce
//! wrong results.

use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId};

use crate::context::{Lowering, NiState};

impl<'a> Lowering<'a> {
    /// Returns true iff `func_id` can return the `NotImplemented` sentinel.
    ///
    /// Memoised: the first call populates the cache for all reachable
    /// callees; subsequent queries are O(1) lookups.
    pub(crate) fn func_may_return_not_implemented(
        &mut self,
        func_id: FuncId,
        hir_module: &hir::Module,
    ) -> bool {
        match self.ni_analysis.cache.get(&func_id).copied() {
            Some(NiState::Yes) => return true,
            Some(NiState::No) => return false,
            // Cycle: tentatively `No` to let the walk unwind. The
            // outermost frame commits the final state.
            Some(NiState::Computing) => return false,
            None => {}
        }

        let Some(func) = hir_module.func_defs.get(&func_id) else {
            // Unknown callee (cross-module, imported, etc.) — conservative.
            self.ni_analysis.cache.insert(func_id, NiState::Yes);
            return true;
        };

        self.ni_analysis.cache.insert(func_id, NiState::Computing);
        let mut may_ni = false;
        // §1.17b-d — walk the CFG directly. Any `Return(expr)` terminator
        // whose `expr` can produce `NotImplemented` flags the whole
        // function. Handler/finally blocks are included because a handler
        // can itself contain `return NotImplemented`.
        'outer: for block in func.blocks.values() {
            // Inspect straight-line statements first (defensive — today
            // these are always Bind/Expr/Assert/etc.; `Return` lives on
            // the terminator).
            for &stmt_id in &block.stmts {
                let stmt = &hir_module.stmts[stmt_id];
                if let hir::StmtKind::Return(Some(expr_id)) = &stmt.kind {
                    if self.scan_return_expr_for_ni(*expr_id, hir_module) {
                        may_ni = true;
                        break 'outer;
                    }
                }
            }
            if let hir::HirTerminator::Return(Some(expr_id)) = &block.terminator {
                if self.scan_return_expr_for_ni(*expr_id, hir_module) {
                    may_ni = true;
                    break 'outer;
                }
            }
        }
        let final_state = if may_ni { NiState::Yes } else { NiState::No };
        self.ni_analysis.cache.insert(func_id, final_state);
        may_ni
    }

    /// Handle the RHS of `return <expr>`. Recognises:
    /// - bare `NotImplemented` literal (direct NI)
    /// - `Call { func: Var(v), .. }` where `v` resolves to a known FuncId
    /// - `MethodCall { obj: <Class instance>, method, .. }` where the
    ///   method resolves to a class-local function
    ///
    /// Unresolved calls → conservative true.
    fn scan_return_expr_for_ni(&mut self, expr_id: hir::ExprId, module: &hir::Module) -> bool {
        let expr = &module.exprs[expr_id];
        match &expr.kind {
            hir::ExprKind::NotImplemented => true,
            hir::ExprKind::Call { func, .. } => {
                let callee_expr = &module.exprs[*func];
                if matches!(callee_expr.kind, hir::ExprKind::ClassRef(_)) {
                    // Class constructors materialize instances; they do not
                    // participate in the NotImplemented protocol.
                    false
                } else {
                    match self.resolve_call_target(*func, module) {
                        Some(callee) => self.func_may_return_not_implemented(callee, module),
                        None => true, // unresolved — conservative
                    }
                }
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                match self.resolve_method_call_target(*obj, *method, module) {
                    Some(callee) => self.func_may_return_not_implemented(callee, module),
                    None => true, // unresolved — conservative
                }
            }
            // IfExpr: `return a if cond else b` — either branch may yield NI.
            hir::ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                self.scan_return_expr_for_ni(*then_val, module)
                    || self.scan_return_expr_for_ni(*else_val, module)
            }
            _ => false,
        }
    }

    /// Resolve `Call { func: Var(v), .. }` to a known FuncId via the
    /// symbol table. Returns `None` if `func` is not a direct variable
    /// reference (e.g. attribute access, dynamic dispatch).
    fn resolve_call_target(&self, func_expr: hir::ExprId, module: &hir::Module) -> Option<FuncId> {
        let expr = &module.exprs[func_expr];
        match &expr.kind {
            hir::ExprKind::Var(v) => self.symbols.var_to_func.get(v).copied(),
            _ => None,
        }
    }

    /// Resolve `MethodCall { obj, method, .. }` to a specific FuncId by
    /// looking up the method on the receiver's class. Only monomorphic
    /// `Type::Class` receivers are resolved; Union/Any receivers return
    /// `None` (conservative).
    fn resolve_method_call_target(
        &mut self,
        obj: hir::ExprId,
        method: pyaot_utils::InternedString,
        module: &hir::Module,
    ) -> Option<FuncId> {
        let obj_ty = module.exprs[obj].ty.clone().unwrap_or(Type::Any);
        let class_id = match obj_ty {
            Type::Class { class_id, .. } => class_id,
            _ => return None,
        };
        // Prefer lowered class info when available; fall back to raw HIR
        // (class_defs) because this analysis may run before every class
        // has been registered in the class_info map.
        if let Some(func_id) = self
            .get_class_info(&class_id)
            .and_then(|ci| lookup_method_on_class_info(ci, method, self.interner))
        {
            return Some(func_id);
        }
        lookup_method_on_hir_class(module, class_id, method)
    }
}

fn lookup_method_on_class_info(
    ci: &crate::context::LoweredClassInfo,
    method: pyaot_utils::InternedString,
    interner: &pyaot_utils::StringInterner,
) -> Option<FuncId> {
    if let Some(func_id) = ci.method_funcs.get(&method).copied() {
        return Some(func_id);
    }
    ci.get_dunder_func(interner.resolve(method))
}

fn lookup_method_on_hir_class(
    module: &hir::Module,
    class_id: ClassId,
    method: pyaot_utils::InternedString,
) -> Option<FuncId> {
    let class_def = module.class_defs.get(&class_id)?;
    for fid in &class_def.methods {
        let Some(func) = module.func_defs.get(fid) else {
            continue;
        };
        if func.name == method {
            return Some(*fid);
        }
    }
    None
}
