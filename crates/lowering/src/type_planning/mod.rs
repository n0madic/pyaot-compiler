//! Unified type planning system
//!
//! Single module for all type inference in lowering:
//! - `infer`: bottom-up type synthesis (`compute_expr_type`)
//! - `pre_scan`: closure/lambda/decorator discovery before codegen
//! - `check`: top-down type validation + error reporting

mod check;
mod constraint_solver;
pub(crate) mod helpers;
pub(crate) mod infer;
mod lambda_inference;
mod local_prescan;
pub(crate) mod ni_analysis;
mod phase4_safe_scan;
mod validate;

use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Run type planning: pre-scan + return type inference for all functions.
    ///
    /// # Architecture (S5 constraint-solver rewrite)
    ///
    /// Replaces the legacy 22-pass + 10-iteration fixpoint planner with a
    /// constraint-based solver. The solver collects type constraints from
    /// the HIR in one walk, drives them to a monotone-JOIN fixpoint over
    /// the `TypeLattice`, then materializes results into the existing
    /// `LoweringSeedInfo` / `func_return_types` / `closures.*` contracts.
    ///
    /// Structural pre-passes that compute non-type information still run
    /// before the solver:
    ///   1. `precompute_phase4_unsafe_funcs` — HOF callback discovery.
    ///   2. `process_module_decorated_functions` — decorator wrap tracking.
    ///
    /// Post-pass adapters that read solver output:
    ///   3. `validate_type_annotations` — diagnostic-only checks.
    ///   4. `fold_refined_field_types_into_storage` — projects
    ///      `refined_class_field_types` into `LoweredClassInfo.field_types`.
    ///   5. `populate_generator_return_types_on_funcdef` — mirrors
    ///      generator return types onto HIR `func_defs[fid].return_type`
    ///      so `desugar_generators` is a pure structural rewrite.
    pub(crate) fn build_lowering_seed_info(&mut self, hir_module: &mut hir::Module) {
        self.precompute_phase4_unsafe_funcs(hir_module);
        self.process_module_decorated_functions(hir_module);

        // Solver-based type planning (replaces 22 legacy passes).
        constraint_solver::run_constraint_solver(self, hir_module);

        // Diagnostic-only annotation validation. Runs after the solver
        // because it consults solver-materialized types for warnings.
        self.validate_type_annotations(hir_module);

        // Project refined class-field types into LoweredClassInfo.
        self.fold_refined_field_types_into_storage();

        // Mirror generator yield types onto HIR func_defs so
        // `desugar_generators` is a pure structural rewrite.
        self.populate_generator_return_types_on_funcdef(hir_module);
    }

    /// Write the converged `Iterator(yield_type)` from `func_return_types` onto
    /// each generator's HIR `FuncDef.return_type` field. This lets
    /// `closure_result_type` (which reads `func_defs[fid].return_type`) resolve
    /// the generator's effective return type at any point AFTER
    /// `build_lowering_seed_info` finishes — in particular before
    /// `desugar_generators` runs, making desugar a pure structural rewrite
    /// that only reads already-finalised types rather than computing them.
    pub(crate) fn populate_generator_return_types_on_funcdef(&self, hir_module: &mut hir::Module) {
        let gen_func_ids: Vec<pyaot_utils::FuncId> = hir_module
            .func_defs
            .iter()
            .filter(|(_, f)| f.is_generator && f.return_type.is_none())
            .map(|(id, _)| *id)
            .collect();
        for func_id in gen_func_ids {
            if let Some(ty) = self.func_return_types.inner.get(&func_id).cloned() {
                if let Some(func) = hir_module.func_defs.get_mut(&func_id) {
                    func.return_type = Some(ty);
                }
            }
        }
    }

    /// §1.4u-b step 5: walk every `ExprId` in the module and force
    /// `seed_expr_type_by_id` to compute+cache the result. `Var` arms
    /// skip the cache (effective type is context-sensitive); all
    /// other arms populate `lowering_seed_info.expr_types`.
    pub(crate) fn eagerly_populate_expr_types(&mut self, hir_module: &hir::Module) {
        let ids: Vec<hir::ExprId> = hir_module.exprs.iter().map(|(id, _)| id).collect();
        for expr_id in ids {
            let _ = self.seed_expr_type_by_id(expr_id, hir_module);
        }
    }

    /// §1.4u-b persistent `base_var_types` builder — populates
    /// `LoweringSeedInfo::base_var_types` from three stable sources
    /// without walking any expression:
    ///
    /// 1. `per_function_local_seed_types` — Area E §E.6 prescan
    ///    output for every function (inferred locals + seeded params).
    /// 2. `hir_module.func_defs[*].params[*].ty` — declared parameter
    ///    annotations. Covers empty-body functions that
    ///    `precompute_all_local_var_types` skipped.
    /// 3. Exception handler binding names (`except E as name:`) —
    ///    collected via `collect_handler_binds`.
    ///
    /// VarIds are globally unique per HIR module, so cross-function
    /// flattening is collision-free. The map is never mutated by
    /// `lower_function` or by narrowing.
    pub(crate) fn populate_base_var_types(&mut self, hir_module: &hir::Module) {
        let from_prescan: Vec<(pyaot_utils::VarId, Type)> = self
            .lowering_seed_info
            .per_function_local_seed_types
            .values()
            .flat_map(|m| m.iter().map(|(k, v)| (*k, v.clone())))
            .collect();
        for (var_id, ty) in from_prescan {
            self.lowering_seed_info.base_var_types.insert(var_id, ty);
        }
        let from_params: Vec<(pyaot_utils::VarId, Type)> = hir_module
            .func_defs
            .values()
            .flat_map(|f| {
                f.params
                    .iter()
                    .filter_map(|p| p.ty.clone().map(|t| (p.var, t)))
            })
            .collect();
        for (var_id, ty) in from_params {
            self.lowering_seed_info.base_var_types.insert(var_id, ty);
        }
        let handler_binds = collect_handler_binds(hir_module);
        for (var_id, ty) in handler_binds {
            self.lowering_seed_info.base_var_types.insert(var_id, ty);
        }
    }

    /// Get the type of an expression by its ID (memoized for non-Var).
    ///
    /// Post-§1.4u-b: this is the single entry point that combines
    /// narrowing-aware `Var` resolution with a pure-function cache
    /// for every other `ExprKind`. The two paths:
    ///
    /// - **`Var`**: reads `get_var_type(v)` (chain: `symbols.var_types`
    ///   → `refined_container_types` → `global_var_types`) with a fallback
    ///   to `get_base_var_type(v)` and then to `expr.ty`. During
    ///   lowering this sees the current function-local overlay in
    ///   `symbols.var_types`; during seed-building / eager-cache time
    ///   that map is empty, so the fallback chain returns the base
    ///   type. Never writes to the cache — Var types are
    ///   context-sensitive.
    /// - **Non-`Var`**: cache hit when available, otherwise call
    ///   `compute_seed_expr_type` (which is now a pure function of HIR
    ///   + F/M state — it does not read `symbols.var_types`) and cache.
    ///
    /// The cache is populated eagerly by `eagerly_populate_expr_types`
    /// at the end of `build_lowering_seed_info`, so during lowering this
    /// function is typically a pure cache hit for non-Var queries and
    /// a cheap `symbols.var_types` read for Vars.
    pub(crate) fn seed_expr_type_by_id(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
    ) -> Type {
        let expr = &hir_module.exprs[expr_id];
        if let hir::ExprKind::Var(var_id) = &expr.kind {
            // Effective-type fast path. `get_var_type` returns the
            // current function-local overlay when present, else the
            // function-local type from the prologue, else falls
            // through to stable sources.
            return self
                .get_var_type(var_id)
                .cloned()
                .or_else(|| self.get_base_var_type(var_id).cloned())
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any);
        }
        if let Some(cached) = self.lowering_seed_info.lookup(expr_id).cloned() {
            return cached;
        }
        let result = self.compute_seed_expr_type(expr, hir_module);
        // Do NOT cache `Any` or `Union` results — they signal narrowing
        // sensitivity. At eager-pass time no narrowing frame is active,
        // so a contained `Var` reads its *base* Union/Any type; a later
        // lowering-time query inside an `isinstance`-dominated block may
        // narrow that `Var` and produce a concrete result. Caching the
        // pre-narrowing Union would poison the cache. Concrete types
        // (Int, Str, Class { … }, Tuple, …) are stable and safe to
        // cache: narrowing never widens them.
        if !matches!(result, Type::Any) && !result.is_union() {
            self.lowering_seed_info.insert_type(expr_id, result.clone());
        }
        result
    }
}

/// §1.4u-b helper: collect `(handler.name, handler.ty)` pairs where both
/// are present, for every exception handler in the module. Used to seed
/// `LoweringSeedInfo::base_var_types` with handler binding types.
///
/// §1.17b-d — reads `Function::try_scopes` directly instead of walking
/// the statement tree, including the synthetic module-init function.
fn collect_handler_binds(module: &hir::Module) -> Vec<(VarId, Type)> {
    let mut out = Vec::new();
    for func in module.func_defs.values() {
        for scope in &func.try_scopes {
            for h in &scope.handlers {
                if let (Some(name), Some(ty)) = (h.name, h.ty.clone()) {
                    out.push((name, ty));
                }
            }
        }
    }
    out
}
