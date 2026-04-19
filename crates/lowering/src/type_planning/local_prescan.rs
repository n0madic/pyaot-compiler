//! Local variable pre-scan pass (Area E §E.6).
//!
//! Walks a function body once (iterated to fixed-point, bounded at 3
//! iterations) and collects a unified type for every local `VarId` by
//! merging every observation through [`Type::unify_field_type`]:
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
//! The result is stored in `symbols.prescan_var_types` and consumed later
//! by `get_or_create_local` (which uses the unified type as the MIR
//! local's declared type) and by `lower_assign` / `bind_var_op` (which
//! coerce each RHS to the unified type before the store).

use indexmap::IndexMap;
use indexmap::IndexSet;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Walk every function in the module and store per-function pre-scan
    /// results in `hir_types.per_function_prescan_var_types`. Called from
    /// `run_type_planning` before return-type inference so that
    /// `return x` can see the unified local type.
    pub(crate) fn precompute_all_local_var_types(&mut self, hir_module: &hir::Module) {
        let func_ids = hir_module.functions.clone();
        for func_id in func_ids {
            if let Some(func) = hir_module.func_defs.get(&func_id) {
                if func.body.is_empty() {
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
                self.hir_types
                    .per_function_prescan_var_types
                    .insert(func_id, map);
            }
        }
    }

    /// Run the local type pre-scan for a single function body.
    ///
    /// Writes the resulting map to `self.hir_types.prescan_var_types`.
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
    /// `loop_depth` comes from `HirBlock.loop_depth` directly (populated
    /// by `cfg_build`).
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

    /// Walk a single straight-line statement (no tree-form control flow).
    /// The only binding-producing kinds that appear inside a `HirBlock.stmts`
    /// list are `Bind`, `IterAdvance`, and the tree-form variants that the
    /// bridge leaves behind until S1.17b-f (If/While/ForBind/Try/Match);
    /// the latter are never emitted into blocks by `cfg_build`, so we
    /// treat any occurrence as a programming error.
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
                    None => self.infer_deep_expr_type(rhs_expr, hir_module, scratch),
                };
                absorb_into_targets(target, &rhs_ty, scratch, loop_only, loop_depth, false);
            }
            hir::StmtKind::IterAdvance { iter, target } => {
                // For-loop target binding: element type of the iterable.
                let iter_expr = &hir_module.exprs[*iter];
                let iter_ty = self.infer_deep_expr_type(iter_expr, hir_module, scratch);
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
            // These should never appear inside a HirBlock.stmts list — the
            // bridge terminates blocks at control-flow boundaries.
            hir::StmtKind::If { .. }
            | hir::StmtKind::While { .. }
            | hir::StmtKind::ForBind { .. }
            | hir::StmtKind::Try { .. }
            | hir::StmtKind::Match { .. } => {
                debug_assert!(
                    false,
                    "local_prescan: control-flow StmtKind must not appear inside HirBlock.stmts"
                );
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
                let merged = Type::unify_field_type(&prev, &new_ty);
                scratch.insert(var_id, merged);
            } else if prev_is_any {
                // Concrete rebind over an `Any` seed — adopt the new
                // type directly (§G.13: unannotated param narrowed via
                // `x = x if isinstance(x, T) else T(x)` rebind).
                //
                // NOTE: narrowing a `Union` seed (e.g. dunder
                // `other: Union[Self, Int, Float, Bool]` narrowed to
                // `Self`) is NOT applied here: the MIR local is
                // allocated once at the signature type, and the caller
                // already boxed the argument at that width. Changing
                // the local's type to a narrower member would break
                // the caller ABI. The dunder-narrowing case needs a
                // second local + unbox; tracked as a known limitation.
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
        Type::Tuple(types) if !types.is_empty() => Type::normalize_union(types.clone()),
        Type::Tuple(_) => Type::Any,
        Type::TupleVar(e) => (**e).clone(),
        Type::Dict(k, _) | Type::DefaultDict(k, _) => (**k).clone(),
        Type::Str => Type::Str,
        Type::Bytes => Type::Int,
        _ => Type::Any,
    }
}
