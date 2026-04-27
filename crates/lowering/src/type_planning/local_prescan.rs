//! Local variable pre-scan pass (Area E §E.6).
//!
//! Walks a function body once (iterated to fixed-point, bounded at 3
//! iterations) and collects a unified type for every local `VarId` by
//! merging every observation through [`TypeLattice::join`]:
//!
//! - `Bind { target, value }` — each `Var` leaf in `target` absorbs the
//!   inferred type of `value`.
//! - `ForBind { target, iter }` — each `Var` leaf absorbs the iterable's
//!   element type.
//!
//! Post-loop rebind heuristic (§A.6 #3): when a variable is first written
//! inside a for-loop (e.g. `for _, c in pairs: ...`) and then written
//! again at the outer scope (`c = Class()`), the outer-scope write
//! *replaces* the loop-bound type instead of forming a union. This is a
//! pragmatic divergence from strict Python semantics — it matches the
//! idiomatic "reuse the for-var name" pattern while keeping attribute
//! access on the rebound class compileable.
//!
//! The result is stored in `lowering_seed_info.current_local_seed_types` and consumed later
//! by `get_or_create_local` (which uses the unified type as the MIR
//! local's declared type) and by `lower_assign` / `bind_var_op` (which
//! coerce each RHS to the unified type before the store).

use indexmap::IndexMap;
use indexmap::IndexSet;
use pyaot_hir as hir;
use pyaot_types::{Type, TypeLattice};
use pyaot_utils::VarId;
use std::collections::HashSet;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Split a `VarId` into fresh versions when later writes would force a
    /// raw local to hold a heap value (or the reverse). This runs on HIR
    /// before the Area E §E.6 local prescan so each version can keep a
    /// storage-compatible prescan type and lowering never has to retarget a
    /// live MIR local mid-CFG.
    pub(crate) fn split_storage_conflicting_var_rebinds(&self, hir_module: &mut hir::Module) {
        let mut next_var_id = next_fresh_var_id(hir_module);
        let func_ids = hir_module.functions.clone();
        for func_id in func_ids {
            let Some(func) = hir_module.func_defs.get(&func_id) else {
                continue;
            };
            if func.has_no_blocks() {
                continue;
            }

            let mut param_seed: IndexMap<VarId, Type> = IndexMap::new();
            for param in &func.params {
                if let Some(ty) = param.ty.clone() {
                    param_seed.insert(param.var, ty);
                }
            }
            let block_ids: Vec<_> = func.blocks.keys().copied().collect();
            let cell_vars = func.cell_vars.clone();
            let nonlocal_vars = func.nonlocal_vars.clone();

            let mut state = VarVersionState::new(next_var_id, &param_seed);
            for block_id in block_ids {
                let stmt_ids = hir_module
                    .func_defs
                    .get(&func_id)
                    .and_then(|f| f.blocks.get(&block_id))
                    .map(|block| block.stmts.clone())
                    .unwrap_or_default();
                for stmt_id in stmt_ids {
                    self.rewrite_stmt_storage_versions(
                        stmt_id,
                        hir_module,
                        &mut state,
                        &cell_vars,
                        &nonlocal_vars,
                    );
                }
                let term = hir_module
                    .func_defs
                    .get(&func_id)
                    .and_then(|f| f.blocks.get(&block_id))
                    .map(|block| block.terminator.clone());
                if let Some(term) = term {
                    rewrite_terminator_exprs(&term, hir_module, &state);
                }
            }
            next_var_id = state.next_var_id;
        }
    }

    /// Walk every function in the module and store per-function pre-scan
    /// results in `lowering_seed_info.per_function_local_seed_types`. Called from
    /// `build_lowering_seed_info` before return-type inference so that
    /// `return x` can see the unified local type.
    pub(crate) fn precompute_all_local_var_types(&mut self, hir_module: &hir::Module) {
        let func_ids = hir_module.functions.clone();
        for func_id in func_ids {
            if let Some(func) = hir_module.func_defs.get(&func_id) {
                if func.has_no_blocks() {
                    continue;
                }
                // Seed from annotations, then layer in any
                // `lambda_param_type_hints` produced by
                // `infer_nested_function_param_types` /
                // HOF scanning — hints fill in types for otherwise
                // unannotated lambda / nested-function params so
                // prescan-visible body observations see the inferred
                // type for call-site-derived parameters.
                let hints = self.get_lambda_param_type_hints(&func_id).cloned();
                let mut param_seed: IndexMap<VarId, Type> = IndexMap::new();
                for (i, p) in func.params.iter().enumerate() {
                    if let Some(ty) = p.ty.clone() {
                        param_seed.insert(p.var, ty);
                    } else if let Some(ref h) = hints {
                        if let Some(ty) = h.get(i).cloned() {
                            if !matches!(ty, Type::Any) {
                                param_seed.insert(p.var, ty);
                            }
                        }
                    }
                }
                let mut map = self.precompute_var_types(func, hir_module, &param_seed);
                // Drop entries for cell / nonlocal variables — those
                // are accessed via cell storage, not as plain locals,
                // so baking a prescan type for them would cause
                // `get_or_create_local` to allocate a real local and
                // bypass the cell path.
                map.retain(|var_id, _| {
                    !func.cell_vars.contains(var_id) && !func.nonlocal_vars.contains(var_id)
                });
                self.lowering_seed_info
                    .per_function_local_seed_types
                    .insert(func_id, map);
            }
        }
    }

    /// Run the local type pre-scan for a single function body.
    ///
    /// Writes the resulting map to `self.lowering_seed_info.current_local_seed_types`.
    /// Call this once per function right before parameters are processed
    /// and statements are lowered. `param_seed` should contain the
    /// parameter `VarId → Type` map already computed by the caller.
    ///
    /// §1.17b-d — walks the CFG (not the legacy tree). Blocks are iterated
    /// in IndexMap insertion order, which is the bridge's allocation
    /// order — a pre-order DFS over the source-form statements. This
    /// preserves the semantics of the tree-walker: for-body stmts are
    /// visited before post-loop stmts, so the post-loop rebind heuristic
    /// (§A.6 #3) sees the loop-only write before the outer-scope rebind.
    /// `loop_depth` comes from `HirBlock.loop_depth` directly.
    pub(crate) fn precompute_var_types(
        &self,
        func: &hir::Function,
        hir_module: &hir::Module,
        param_seed: &IndexMap<VarId, Type>,
    ) -> IndexMap<VarId, Type> {
        let mut scratch: IndexMap<VarId, Type> = param_seed.clone();
        let mut loop_only: IndexSet<VarId> = IndexSet::new();
        for block in func.blocks.values() {
            let loop_depth = block.loop_depth as usize;
            for &stmt_id in &block.stmts {
                let stmt = &hir_module.stmts[stmt_id];
                self.walk_flat_stmt(stmt, hir_module, &mut scratch, &mut loop_only, loop_depth);
            }
        }
        scratch
    }

    /// Walk a single straight-line statement inside a HIR CFG block.
    fn walk_flat_stmt(
        &self,
        stmt: &hir::Stmt,
        hir_module: &hir::Module,
        scratch: &mut IndexMap<VarId, Type>,
        loop_only: &mut IndexSet<VarId>,
        loop_depth: usize,
    ) {
        match &stmt.kind {
            hir::StmtKind::Bind {
                target,
                value,
                type_hint,
            } => {
                let rhs_expr = &hir_module.exprs[*value];
                // `def inner(): ...` becomes a Bind with a FuncRef or
                // Closure value — those synthesize the enclosing
                // function's *return* type under `infer_expr_type_inner`,
                // which is nonsense for the binding target.
                if matches!(
                    rhs_expr.kind,
                    hir::ExprKind::FuncRef(_) | hir::ExprKind::Closure { .. }
                ) {
                    return;
                }
                // Explicit annotation wins — `x: T = value` establishes
                // the declared type irrespective of the value's inferred
                // type.
                let rhs_ty = match type_hint {
                    Some(ann) => ann.clone(),
                    None => self.seed_infer_expr_type(rhs_expr, hir_module, scratch),
                };
                absorb_into_targets(target, &rhs_ty, scratch, loop_only, loop_depth, false);
            }
            hir::StmtKind::IterAdvance { iter, target } => {
                // For-loop target binding: element type of the iterable.
                let iter_expr = &hir_module.exprs[*iter];
                let iter_ty = self.seed_infer_expr_type(iter_expr, hir_module, scratch);
                let elem_ty = elem_type_of_iterable(&iter_ty);
                absorb_into_targets(target, &elem_ty, scratch, loop_only, loop_depth, true);
            }
            // Nothing to absorb for straight-line non-binding stmts.
            hir::StmtKind::Expr(_)
            | hir::StmtKind::Return(_)
            | hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Raise { .. }
            | hir::StmtKind::Pass
            | hir::StmtKind::Assert { .. }
            | hir::StmtKind::IndexDelete { .. } => {}
            // IterSetup is a pre-block iter-protocol initializer — no
            // local-var binding to absorb.
            hir::StmtKind::IterSetup { .. } => {}
        }
    }

    fn rewrite_stmt_storage_versions(
        &self,
        stmt_id: hir::StmtId,
        hir_module: &mut hir::Module,
        state: &mut VarVersionState,
        cell_vars: &HashSet<VarId>,
        nonlocal_vars: &HashSet<VarId>,
    ) {
        let stmt_kind = hir_module.stmts[stmt_id].kind.clone();
        match &stmt_kind {
            hir::StmtKind::Bind { target, value, .. } => {
                rewrite_binding_target_uses(target, hir_module, state);
                rewrite_expr_vars(*value, hir_module, state);
            }
            hir::StmtKind::IterAdvance { iter, target } => {
                rewrite_expr_vars(*iter, hir_module, state);
                rewrite_binding_target_uses(target, hir_module, state);
            }
            hir::StmtKind::Expr(expr_id) => rewrite_expr_vars(*expr_id, hir_module, state),
            hir::StmtKind::Return(Some(expr_id)) => rewrite_expr_vars(*expr_id, hir_module, state),
            hir::StmtKind::Return(None)
            | hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Pass
            | hir::StmtKind::IterSetup { .. } => {}
            hir::StmtKind::Raise { exc, cause } => {
                if let Some(exc_id) = exc {
                    rewrite_expr_vars(*exc_id, hir_module, state);
                }
                if let Some(cause_id) = cause {
                    rewrite_expr_vars(*cause_id, hir_module, state);
                }
            }
            hir::StmtKind::Assert { cond, msg } => {
                rewrite_expr_vars(*cond, hir_module, state);
                if let Some(msg_id) = msg {
                    rewrite_expr_vars(*msg_id, hir_module, state);
                }
            }
            hir::StmtKind::IndexDelete { obj, index } => {
                rewrite_expr_vars(*obj, hir_module, state);
                rewrite_expr_vars(*index, hir_module, state);
            }
        }

        let rhs_ty = match stmt_kind {
            hir::StmtKind::Bind {
                value, type_hint, ..
            } => {
                let expr = &hir_module.exprs[value];
                match (&expr.kind, type_hint) {
                    (_, Some(ann)) => ann,
                    (hir::ExprKind::FuncRef(_) | hir::ExprKind::Closure { .. }, None) => Type::Any,
                    _ => self.seed_infer_expr_type(expr, hir_module, &state.current_types),
                }
            }
            hir::StmtKind::IterAdvance { iter, .. } => {
                let iter_ty = self.seed_infer_expr_type(
                    &hir_module.exprs[iter],
                    hir_module,
                    &state.current_types,
                );
                elem_type_of_iterable(&iter_ty)
            }
            _ => return,
        };

        let mut rewritten_target = match &hir_module.stmts[stmt_id].kind {
            hir::StmtKind::Bind { target, .. } | hir::StmtKind::IterAdvance { target, .. } => {
                target.clone()
            }
            _ => return,
        };
        rewrite_binding_target_defs(
            &mut rewritten_target,
            &rhs_ty,
            hir_module,
            state,
            cell_vars,
            nonlocal_vars,
        );
        match &mut hir_module.stmts[stmt_id].kind {
            hir::StmtKind::Bind { target, .. } | hir::StmtKind::IterAdvance { target, .. } => {
                *target = rewritten_target;
            }
            _ => {}
        }
    }
}

struct VarVersionState {
    next_var_id: u32,
    current_version: IndexMap<VarId, VarId>,
    version_root: IndexMap<VarId, VarId>,
    current_types: IndexMap<VarId, Type>,
}

impl VarVersionState {
    fn new(next_var_id: u32, seed_types: &IndexMap<VarId, Type>) -> Self {
        Self {
            next_var_id,
            current_version: IndexMap::new(),
            version_root: IndexMap::new(),
            current_types: seed_types.clone(),
        }
    }

    fn root_of(&self, var_id: VarId) -> VarId {
        self.version_root.get(&var_id).copied().unwrap_or(var_id)
    }

    fn current_of(&self, var_id: VarId) -> VarId {
        let root = self.root_of(var_id);
        self.current_version.get(&root).copied().unwrap_or(root)
    }

    fn alloc_split(&mut self, root: VarId) -> VarId {
        let fresh = VarId::new(self.next_var_id);
        self.next_var_id += 1;
        self.version_root.insert(fresh, root);
        self.current_version.insert(root, fresh);
        fresh
    }

    fn record_write(&mut self, var_id: VarId, ty: &Type) {
        if !matches!(ty, Type::Any) {
            self.current_types.insert(var_id, ty.clone());
        }
    }

    fn storage_conflict(&self, current_var: VarId, new_ty: &Type) -> bool {
        let Some(prev_ty) = self.current_types.get(&current_var) else {
            return false;
        };
        if prev_ty == new_ty || prev_ty.is_heap() == new_ty.is_heap() {
            return false;
        }
        if matches!(new_ty, Type::Any | Type::HeapAny) {
            return false;
        }
        // Dynamic heap locals can safely accept raw primitives by boxing at
        // store time; concrete heap locals cannot.
        let prev_accepts_boxed_primitive =
            matches!(prev_ty, Type::Any | Type::HeapAny | Type::Union(_))
                && matches!(new_ty, Type::Int | Type::Bool | Type::Float | Type::None);
        !prev_accepts_boxed_primitive
    }
}

fn next_fresh_var_id(hir_module: &hir::Module) -> u32 {
    let mut max_id = 0u32;

    for var_id in hir_module.globals.iter().copied() {
        max_id = max_id.max(var_id.0);
    }
    for var_id in hir_module.module_var_map.values().copied() {
        max_id = max_id.max(var_id.0);
    }
    for func in hir_module.func_defs.values() {
        for param in &func.params {
            max_id = max_id.max(param.var.0);
        }
        for var_id in func.cell_vars.iter().copied() {
            max_id = max_id.max(var_id.0);
        }
        for var_id in func.nonlocal_vars.iter().copied() {
            max_id = max_id.max(var_id.0);
        }
    }
    for (_, stmt) in hir_module.stmts.iter() {
        max_var_in_stmt(stmt, &mut max_id);
    }
    for (_, expr) in hir_module.exprs.iter() {
        max_var_in_expr(expr, &mut max_id);
    }
    max_id + 1
}

fn max_var_in_stmt(stmt: &hir::Stmt, max_id: &mut u32) {
    match &stmt.kind {
        hir::StmtKind::Bind { target, .. } | hir::StmtKind::IterAdvance { target, .. } => {
            max_var_in_target(target, max_id);
        }
        _ => {}
    }
}

fn max_var_in_target(target: &hir::BindingTarget, max_id: &mut u32) {
    match target {
        hir::BindingTarget::Var(var_id) => *max_id = (*max_id).max(var_id.0),
        hir::BindingTarget::Tuple { elts, .. } => {
            for elt in elts {
                max_var_in_target(elt, max_id);
            }
        }
        hir::BindingTarget::Starred { inner, .. } => max_var_in_target(inner, max_id),
        hir::BindingTarget::Attr { .. }
        | hir::BindingTarget::Index { .. }
        | hir::BindingTarget::ClassAttr { .. } => {}
    }
}

fn max_var_in_expr(expr: &hir::Expr, max_id: &mut u32) {
    if let hir::ExprKind::Var(var_id) = expr.kind {
        *max_id = (*max_id).max(var_id.0);
    }
}

fn rewrite_terminator_exprs(
    term: &hir::HirTerminator,
    hir_module: &mut hir::Module,
    state: &VarVersionState,
) {
    match term {
        hir::HirTerminator::Jump(_)
        | hir::HirTerminator::Unreachable
        | hir::HirTerminator::Reraise => {}
        hir::HirTerminator::Branch { cond, .. } => rewrite_expr_vars(*cond, hir_module, state),
        hir::HirTerminator::Return(Some(expr_id))
        | hir::HirTerminator::Yield { value: expr_id, .. } => {
            rewrite_expr_vars(*expr_id, hir_module, state)
        }
        hir::HirTerminator::Return(None) => {}
        hir::HirTerminator::Raise { exc, cause } => {
            rewrite_expr_vars(*exc, hir_module, state);
            if let Some(cause_id) = cause {
                rewrite_expr_vars(*cause_id, hir_module, state);
            }
        }
    }
}

fn rewrite_binding_target_uses(
    target: &hir::BindingTarget,
    hir_module: &mut hir::Module,
    state: &VarVersionState,
) {
    match target {
        hir::BindingTarget::Var(_) | hir::BindingTarget::ClassAttr { .. } => {}
        hir::BindingTarget::Attr { obj, .. } => rewrite_expr_vars(*obj, hir_module, state),
        hir::BindingTarget::Index { obj, index, .. } => {
            rewrite_expr_vars(*obj, hir_module, state);
            rewrite_expr_vars(*index, hir_module, state);
        }
        hir::BindingTarget::Tuple { elts, .. } => {
            for elt in elts {
                rewrite_binding_target_uses(elt, hir_module, state);
            }
        }
        hir::BindingTarget::Starred { inner, .. } => {
            rewrite_binding_target_uses(inner, hir_module, state);
        }
    }
}

fn rewrite_binding_target_defs(
    target: &mut hir::BindingTarget,
    rhs_ty: &Type,
    hir_module: &mut hir::Module,
    state: &mut VarVersionState,
    cell_vars: &HashSet<VarId>,
    nonlocal_vars: &HashSet<VarId>,
) {
    match target {
        hir::BindingTarget::Var(slot) => {
            let root = state.root_of(*slot);
            let current = state.current_of(*slot);
            let can_split = !cell_vars.contains(&current)
                && !nonlocal_vars.contains(&current)
                && state.storage_conflict(current, rhs_ty);
            let chosen = if can_split {
                let fresh = state.alloc_split(root);
                if hir_module.globals.contains(&root) {
                    hir_module.globals.insert(fresh);
                    for mapped in hir_module.module_var_map.values_mut() {
                        if *mapped == root || *mapped == current {
                            *mapped = fresh;
                        }
                    }
                }
                fresh
            } else {
                current
            };
            *slot = chosen;
            state.record_write(chosen, rhs_ty);
        }
        hir::BindingTarget::Tuple { elts, .. } => match rhs_ty {
            Type::Tuple(types) if types.len() == elts.len() => {
                for (elt, ty) in elts.iter_mut().zip(types) {
                    rewrite_binding_target_defs(
                        elt,
                        ty,
                        hir_module,
                        state,
                        cell_vars,
                        nonlocal_vars,
                    );
                }
            }
            Type::TupleVar(elem_ty) => {
                for elt in elts {
                    rewrite_binding_target_defs(
                        elt,
                        elem_ty,
                        hir_module,
                        state,
                        cell_vars,
                        nonlocal_vars,
                    );
                }
            }
            _ => {
                for elt in elts {
                    rewrite_binding_target_defs(
                        elt,
                        &Type::Any,
                        hir_module,
                        state,
                        cell_vars,
                        nonlocal_vars,
                    );
                }
            }
        },
        hir::BindingTarget::Starred { inner, .. } => {
            let starred_ty = Type::List(Box::new(rhs_ty.clone()));
            rewrite_binding_target_defs(
                inner,
                &starred_ty,
                hir_module,
                state,
                cell_vars,
                nonlocal_vars,
            );
        }
        hir::BindingTarget::Attr { .. }
        | hir::BindingTarget::Index { .. }
        | hir::BindingTarget::ClassAttr { .. } => {}
    }
}

fn rewrite_expr_vars(expr_id: hir::ExprId, hir_module: &mut hir::Module, state: &VarVersionState) {
    let expr_kind = hir_module.exprs[expr_id].kind.clone();
    match expr_kind {
        hir::ExprKind::Var(var_id) => {
            hir_module.exprs[expr_id].kind = hir::ExprKind::Var(state.current_of(var_id));
        }
        hir::ExprKind::BinOp { left, right, .. }
        | hir::ExprKind::Compare { left, right, .. }
        | hir::ExprKind::LogicalOp { left, right, .. } => {
            rewrite_expr_vars(left, hir_module, state);
            rewrite_expr_vars(right, hir_module, state);
        }
        hir::ExprKind::UnOp { operand, .. }
        | hir::ExprKind::Attribute { obj: operand, .. }
        | hir::ExprKind::Yield(Some(operand))
        | hir::ExprKind::IterHasNext(operand) => rewrite_expr_vars(operand, hir_module, state),
        hir::ExprKind::Yield(None)
        | hir::ExprKind::Int(_)
        | hir::ExprKind::Float(_)
        | hir::ExprKind::Bool(_)
        | hir::ExprKind::Str(_)
        | hir::ExprKind::Bytes(_)
        | hir::ExprKind::None
        | hir::ExprKind::NotImplemented
        | hir::ExprKind::FuncRef(_)
        | hir::ExprKind::ClassRef(_)
        | hir::ExprKind::ClassAttrRef { .. }
        | hir::ExprKind::TypeRef(_)
        | hir::ExprKind::ImportedRef { .. }
        | hir::ExprKind::ModuleAttr { .. }
        | hir::ExprKind::BuiltinRef(_)
        | hir::ExprKind::StdlibAttr(_)
        | hir::ExprKind::StdlibConst(_)
        | hir::ExprKind::ExcCurrentValue => {}
        hir::ExprKind::Call {
            func,
            args,
            kwargs,
            kwargs_unpack,
        } => {
            rewrite_expr_vars(func, hir_module, state);
            rewrite_call_args(&args, hir_module, state);
            rewrite_keyword_args(&kwargs, hir_module, state);
            if let Some(expr_id) = kwargs_unpack {
                rewrite_expr_vars(expr_id, hir_module, state);
            }
        }
        hir::ExprKind::BuiltinCall { args, kwargs, .. } => {
            for arg in args {
                rewrite_expr_vars(arg, hir_module, state);
            }
            rewrite_keyword_args(&kwargs, hir_module, state);
        }
        hir::ExprKind::IfExpr {
            cond,
            then_val,
            else_val,
        } => {
            rewrite_expr_vars(cond, hir_module, state);
            rewrite_expr_vars(then_val, hir_module, state);
            rewrite_expr_vars(else_val, hir_module, state);
        }
        hir::ExprKind::List(items)
        | hir::ExprKind::Tuple(items)
        | hir::ExprKind::Set(items)
        | hir::ExprKind::Closure {
            captures: items, ..
        } => {
            for item in items {
                rewrite_expr_vars(item, hir_module, state);
            }
        }
        hir::ExprKind::Dict(entries) => {
            for (key, value) in entries {
                rewrite_expr_vars(key, hir_module, state);
                rewrite_expr_vars(value, hir_module, state);
            }
        }
        hir::ExprKind::Index { obj, index } => {
            rewrite_expr_vars(obj, hir_module, state);
            rewrite_expr_vars(index, hir_module, state);
        }
        hir::ExprKind::Slice {
            obj,
            start,
            end,
            step,
        } => {
            rewrite_expr_vars(obj, hir_module, state);
            if let Some(expr_id) = start {
                rewrite_expr_vars(expr_id, hir_module, state);
            }
            if let Some(expr_id) = end {
                rewrite_expr_vars(expr_id, hir_module, state);
            }
            if let Some(expr_id) = step {
                rewrite_expr_vars(expr_id, hir_module, state);
            }
        }
        hir::ExprKind::MethodCall {
            obj, args, kwargs, ..
        } => {
            rewrite_expr_vars(obj, hir_module, state);
            for arg in args {
                rewrite_expr_vars(arg, hir_module, state);
            }
            rewrite_keyword_args(&kwargs, hir_module, state);
        }
        hir::ExprKind::SuperCall { args, .. } | hir::ExprKind::StdlibCall { args, .. } => {
            for arg in args {
                rewrite_expr_vars(arg, hir_module, state);
            }
        }
        hir::ExprKind::GeneratorIntrinsic(intrinsic) => match intrinsic {
            hir::GeneratorIntrinsic::GetState(expr_id)
            | hir::GeneratorIntrinsic::SetExhausted(expr_id)
            | hir::GeneratorIntrinsic::IsExhausted(expr_id)
            | hir::GeneratorIntrinsic::GetSentValue(expr_id)
            | hir::GeneratorIntrinsic::IterNextNoExc(expr_id)
            | hir::GeneratorIntrinsic::IterIsExhausted(expr_id) => {
                rewrite_expr_vars(expr_id, hir_module, state);
            }
            hir::GeneratorIntrinsic::SetState { gen, .. }
            | hir::GeneratorIntrinsic::GetLocal { gen, .. } => {
                rewrite_expr_vars(gen, hir_module, state);
            }
            hir::GeneratorIntrinsic::SetLocal { gen, value, .. } => {
                rewrite_expr_vars(gen, hir_module, state);
                rewrite_expr_vars(value, hir_module, state);
            }
            hir::GeneratorIntrinsic::Create { .. } => {}
        },
        hir::ExprKind::MatchPattern { subject, pattern } => {
            rewrite_expr_vars(subject, hir_module, state);
            rewrite_pattern_exprs(&pattern, hir_module, state);
        }
    }
}

fn rewrite_call_args(args: &[hir::CallArg], hir_module: &mut hir::Module, state: &VarVersionState) {
    for arg in args {
        match arg {
            hir::CallArg::Regular(expr_id) | hir::CallArg::Starred(expr_id) => {
                rewrite_expr_vars(*expr_id, hir_module, state);
            }
        }
    }
}

fn rewrite_keyword_args(
    kwargs: &[hir::KeywordArg],
    hir_module: &mut hir::Module,
    state: &VarVersionState,
) {
    for kw in kwargs {
        rewrite_expr_vars(kw.value, hir_module, state);
    }
}

fn rewrite_pattern_exprs(
    pattern: &hir::Pattern,
    hir_module: &mut hir::Module,
    state: &VarVersionState,
) {
    match pattern {
        hir::Pattern::MatchValue(expr_id) => rewrite_expr_vars(*expr_id, hir_module, state),
        hir::Pattern::MatchSingleton(_) | hir::Pattern::MatchStar(_) => {}
        hir::Pattern::MatchAs { pattern, .. } => {
            if let Some(inner) = pattern {
                rewrite_pattern_exprs(inner, hir_module, state);
            }
        }
        hir::Pattern::MatchSequence { patterns } | hir::Pattern::MatchOr(patterns) => {
            for inner in patterns {
                rewrite_pattern_exprs(inner, hir_module, state);
            }
        }
        hir::Pattern::MatchMapping { keys, patterns, .. } => {
            for key in keys {
                rewrite_expr_vars(*key, hir_module, state);
            }
            for inner in patterns {
                rewrite_pattern_exprs(inner, hir_module, state);
            }
        }
        hir::Pattern::MatchClass {
            cls,
            patterns,
            kwd_patterns,
            ..
        } => {
            rewrite_expr_vars(*cls, hir_module, state);
            for inner in patterns {
                rewrite_pattern_exprs(inner, hir_module, state);
            }
            for inner in kwd_patterns {
                rewrite_pattern_exprs(inner, hir_module, state);
            }
        }
    }
}

/// For every `Var` leaf in `target`, merge `rhs_ty` into `scratch`.
/// `from_for_bind` is true when the write comes from a for-loop target.
/// `loop_depth` tracks how deeply nested inside loops the write happens.
fn absorb_into_targets(
    target: &hir::BindingTarget,
    rhs_ty: &Type,
    scratch: &mut IndexMap<VarId, Type>,
    loop_only: &mut IndexSet<VarId>,
    loop_depth: usize,
    from_for_bind: bool,
) {
    match target {
        hir::BindingTarget::Var(var_id) => merge_var(
            scratch,
            loop_only,
            *var_id,
            rhs_ty.clone(),
            loop_depth,
            from_for_bind,
        ),
        hir::BindingTarget::Tuple { elts, .. } => match rhs_ty {
            Type::Tuple(types) if types.len() == elts.len() => {
                for (elt, t) in elts.iter().zip(types) {
                    absorb_into_targets(elt, t, scratch, loop_only, loop_depth, from_for_bind);
                }
            }
            Type::TupleVar(elem) => {
                for elt in elts {
                    absorb_into_targets(elt, elem, scratch, loop_only, loop_depth, from_for_bind);
                }
            }
            _ => {
                for elt in elts {
                    absorb_into_targets(
                        elt,
                        &Type::Any,
                        scratch,
                        loop_only,
                        loop_depth,
                        from_for_bind,
                    );
                }
            }
        },
        hir::BindingTarget::Starred { inner, .. } => {
            // Starred captures a list of the outer element type.
            absorb_into_targets(
                inner,
                &Type::List(Box::new(rhs_ty.clone())),
                scratch,
                loop_only,
                loop_depth,
                from_for_bind,
            );
        }
        hir::BindingTarget::Attr { .. }
        | hir::BindingTarget::Index { .. }
        | hir::BindingTarget::ClassAttr { .. } => {
            // Not a variable binding — nothing to record.
        }
    }
}

fn merge_var(
    scratch: &mut IndexMap<VarId, Type>,
    loop_only: &mut IndexSet<VarId>,
    var_id: VarId,
    new_ty: Type,
    loop_depth: usize,
    from_for_bind: bool,
) {
    // Ignore Any — it adds no information and would widen every local
    // immediately to Any, defeating the pre-scan.
    if matches!(new_ty, Type::Any) {
        return;
    }
    let write_is_loop_scoped = from_for_bind || loop_depth > 0;
    match scratch.get(&var_id).cloned() {
        Some(prev) => {
            // Post-loop rebind heuristic (§A.6 #3): if the variable was
            // previously only observed inside a loop and this write is
            // OUTSIDE any loop, replace rather than union.
            if loop_only.contains(&var_id) && !write_is_loop_scoped {
                scratch.insert(var_id, new_ty);
                loop_only.shift_remove(&var_id);
                return;
            }
            // Only widen across rebinds when the merge is meaningful:
            // either (a) numeric-tower promotion (Bool ⊂ Int ⊂ Float —
            // the §E.6 headline case `x = 0; x += 0.5`), or (b) tuple-
            // shape unification (Area D rule), or (c) the previous type
            // is `Any`/`HeapAny` and the new type is concrete — §G.13
            // needs this so unannotated params narrowed via
            // `x = x if isinstance(x, T) else T(x)` pick up the
            // concrete `T` instead of staying `Any`. For all other type
            // combinations, preserve the first-write type — this keeps
            // compatibility with the prior "first-write wins" behaviour
            // that Union-aware narrowing and other passes depend on.
            let is_numeric_pair = matches!(
                (&prev, &new_ty),
                (
                    Type::Int | Type::Bool | Type::Float,
                    Type::Int | Type::Bool | Type::Float
                )
            );
            let is_tuple_pair = matches!(
                (&prev, &new_ty),
                (
                    Type::Tuple(_) | Type::TupleVar(_),
                    Type::Tuple(_) | Type::TupleVar(_)
                )
            );
            let prev_is_any = matches!(prev, Type::Any | Type::HeapAny);
            if is_numeric_pair || is_tuple_pair {
                let merged = prev.join(&new_ty);
                scratch.insert(var_id, merged);
            } else if prev_is_any {
                // Concrete rebind over an `Any` seed — adopt the new
                // type directly (§G.13: unannotated param narrowed via
                // `x = x if isinstance(x, T) else T(x)` rebind).
                //
                // NOTE: for `Union` seeds (e.g. dunder
                // `other: Union[Self, Int, Float, Bool]` narrowed to
                // `Self`) prescan still keeps the storage ABI at the
                // wider signature type. Lowering materializes a shadow
                // narrowed local after the rebind so post-assign reads
                // see `Self` without changing the caller ABI.
                scratch.insert(var_id, new_ty);
            }
            if !write_is_loop_scoped {
                loop_only.shift_remove(&var_id);
            }
        }
        None => {
            scratch.insert(var_id, new_ty);
            if write_is_loop_scoped {
                loop_only.insert(var_id);
            }
        }
    }
}

/// Approximate element type of an iterable. Handles the common cases;
/// anything else returns `Type::Any` and the caller treats the loop var
/// as untyped.
fn elem_type_of_iterable(ty: &Type) -> Type {
    match ty {
        Type::List(e) | Type::Set(e) | Type::Iterator(e) => (**e).clone(),
        Type::Tuple(types) if !types.is_empty() => types
            .iter()
            .cloned()
            .reduce(|a, b| a.join(&b))
            .unwrap_or(Type::Never),
        Type::Tuple(_) => Type::Any,
        Type::TupleVar(e) => (**e).clone(),
        Type::Dict(k, _) | Type::DefaultDict(k, _) => (**k).clone(),
        Type::Str => Type::Str,
        Type::Bytes => Type::Int,
        _ => Type::Any,
    }
}
