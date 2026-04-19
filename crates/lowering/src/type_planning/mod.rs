//! Unified type planning system
//!
//! Single module for all type inference in lowering:
//! - `infer`: bottom-up type synthesis (`compute_expr_type`)
//! - `pre_scan`: closure/lambda/decorator discovery before codegen
//! - `check`: top-down type validation + error reporting

mod check;
mod closure_scan;
mod container_refine;
pub(crate) mod helpers;
pub(crate) mod infer;
mod lambda_inference;
mod local_prescan;
pub(crate) mod ni_analysis;
mod validate;

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Run type planning: pre-scan + return type inference for all functions.
    pub(crate) fn run_type_planning(&mut self, hir_module: &hir::Module) {
        self.precompute_closure_capture_types(hir_module);
        self.process_module_decorated_functions(hir_module);
        // First-pass refinement — handles `x = [1, 2, 3]` /
        // `x = {k: v, …}` literal cases that need no var-type
        // context. Runs before prescan so prescan sees the refined
        // type when walking `x.append(…)`.
        self.refine_empty_container_types(hir_module);
        self.infer_nested_function_param_types(hir_module);
        self.infer_all_return_types(hir_module);
        self.precompute_all_local_var_types(hir_module);
        // Area E §E.6 — re-infer return types for unannotated functions
        // after prescan has widened any numeric locals (e.g.
        // `x = 0; x += 0.5; return x` → `Float`, not `Int`).
        self.reinfer_return_types_with_prescan(hir_module);
        // Second-pass refinement — re-runs after prescan so
        // `topo = []; topo.append(root)` where `root`'s type comes
        // from prescan (not the declared HIR annotation) can still
        // refine `topo` to `List[Value]`. The underlying scan uses
        // `infer_deep_expr_type` with a prescan-sourced overlay so
        // any intermediate local gets resolved.
        self.refine_empty_container_types(hir_module);
        self.validate_type_annotations(hir_module);
        // §1.4u-b step 3 — populate the stable per-module Var→Type
        // base map. Never mutated during lowering.
        self.populate_base_var_types(hir_module);
        // §1.4u-b step 5 — populate `hir_types.expr_types` eagerly for
        // every non-Var ExprId. Lowering-side queries become cache hits
        // for stable (non-narrowing-sensitive) expressions.
        self.eagerly_populate_expr_types(hir_module);
    }

    /// §1.4u-b step 5: walk every `ExprId` in the module and force
    /// `get_type_of_expr_id` to compute+cache the result. `Var` arms
    /// skip the cache (effective type is context-sensitive); all
    /// other arms populate `hir_types.expr_types`.
    fn eagerly_populate_expr_types(&mut self, hir_module: &hir::Module) {
        let ids: Vec<hir::ExprId> = hir_module.exprs.iter().map(|(id, _)| id).collect();
        for expr_id in ids {
            let _ = self.get_type_of_expr_id(expr_id, hir_module);
        }
    }

    /// §1.4u-b persistent `base_var_types` builder — populates
    /// `HirTypeInference::base_var_types` from three stable sources
    /// without walking any expression:
    ///
    /// 1. `per_function_prescan_var_types` — Area E §E.6 prescan
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
    fn populate_base_var_types(&mut self, hir_module: &hir::Module) {
        let from_prescan: Vec<(pyaot_utils::VarId, Type)> = self
            .hir_types
            .per_function_prescan_var_types
            .values()
            .flat_map(|m| m.iter().map(|(k, v)| (*k, v.clone())))
            .collect();
        for (var_id, ty) in from_prescan {
            self.hir_types.base_var_types.insert(var_id, ty);
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
            self.hir_types.base_var_types.insert(var_id, ty);
        }
        let handler_binds = collect_handler_binds(hir_module);
        for (var_id, ty) in handler_binds {
            self.hir_types.base_var_types.insert(var_id, ty);
        }
    }

    /// Get the type of an expression by its ID (memoized for non-Var).
    ///
    /// Post-§1.4u-b: this is the single entry point that combines
    /// narrowing-aware `Var` resolution with a pure-function cache
    /// for every other `ExprKind`. The two paths:
    ///
    /// - **`Var`**: reads `get_var_type(v)` (chain: `symbols.var_types`
    ///   → `refined_var_types` → `global_var_types`) with a fallback
    ///   to `get_base_var_type(v)` and then to `expr.ty`. At lowering
    ///   time this sees any narrowing that `push_narrowing_frame` has
    ///   installed; at type-planning / eager-cache time
    ///   `symbols.var_types` is empty so the fallback chain returns
    ///   the base type. Never writes to the cache — Var types are
    ///   context-sensitive.
    /// - **Non-`Var`**: cache hit when available, otherwise call
    ///   `compute_expr_type` (which is now a pure function of HIR +
    ///   F/M state — it does not read `symbols.var_types`) and cache.
    ///
    /// The cache is populated eagerly by `eagerly_populate_expr_types`
    /// at the end of `run_type_planning`, so during lowering this
    /// function is typically a pure cache hit for non-Var queries and
    /// a cheap `symbols.var_types` read for Vars.
    pub(crate) fn get_type_of_expr_id(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
    ) -> Type {
        let expr = &hir_module.exprs[expr_id];
        if let hir::ExprKind::Var(var_id) = &expr.kind {
            // Effective-type fast path. `get_var_type` returns the
            // narrowed type inside an active `push_narrowing_frame`
            // scope, else the function-local type from the prologue,
            // else falls through to stable sources.
            return self
                .get_var_type(var_id)
                .cloned()
                .or_else(|| self.get_base_var_type(var_id).cloned())
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any);
        }
        if let Some(cached) = self.hir_types.lookup(expr_id).cloned() {
            return cached;
        }
        let result = self.compute_expr_type(expr, hir_module);
        // Do NOT cache `Any` or `Union` results — they signal narrowing
        // sensitivity. At eager-pass time no narrowing frame is active,
        // so a contained `Var` reads its *base* Union/Any type; a later
        // lowering-time query inside an `isinstance`-dominated block may
        // narrow that `Var` and produce a concrete result. Caching the
        // pre-narrowing Union would poison the cache. Concrete types
        // (Int, Str, Class { … }, Tuple, …) are stable and safe to
        // cache: narrowing never widens them.
        if !matches!(result, Type::Any) && !result.is_union() {
            self.hir_types.insert_type(expr_id, result.clone());
        }
        result
    }

    /// Get the effective type of an expression (uncached).
    ///
    /// Prefer `get_type_of_expr_id` when the `ExprId` is available —
    /// it uses the `expr_types` cache. This method exists only for callers
    /// that receive `&hir::Expr` without an ExprId (e.g., the current
    /// expression being lowered in `lower_expr`).
    pub(crate) fn get_expr_type(&mut self, expr: &hir::Expr, hir_module: &hir::Module) -> Type {
        self.compute_expr_type(expr, hir_module)
    }
}

// =============================================================================
// Return Type Inference Pass
// =============================================================================

impl<'a> Lowering<'a> {
    /// Infer return types for ALL functions without explicit annotations.
    /// Runs before codegen so that `compute_expr_type` for Call expressions
    /// can look up return types in `func_return_types`.
    fn infer_all_return_types(&mut self, hir_module: &hir::Module) {
        // Collect func_ids to avoid borrow issues
        let func_ids = hir_module.functions.to_vec();

        // Pass 1: Collect explicitly annotated return types so they are available
        // for cross-function inference (fixes forward-reference ordering).
        // This includes `-> None` annotations — the HIR distinguishes
        // `Option::None` (no annotation) from `Some(Type::None)` (explicit `-> None`).
        for func_id in &func_ids {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                if let Some(ref return_type) = func.return_type {
                    self.func_return_types
                        .inner
                        .insert(*func_id, return_type.clone());
                }
            }
        }

        // Pass 2: Infer return types for unannotated functions
        for func_id in &func_ids {
            // Skip functions already resolved in pass 1
            if self.func_return_types.inner.contains_key(func_id) {
                continue;
            }

            if let Some(func) = hir_module.func_defs.get(func_id) {
                // Skip empty functions
                if func.body.is_empty() {
                    continue;
                }

                // Build param type map for this function
                let mut param_types: IndexMap<VarId, Type> = IndexMap::new();
                // Use lambda_param_type_hints if available (from map/filter/reduce pre-scan)
                let hints = self.closures.lambda_param_type_hints.get(func_id).cloned();
                for (i, param) in func.params.iter().enumerate() {
                    let ty = param.ty.clone().unwrap_or_else(|| {
                        hints
                            .as_ref()
                            .and_then(|h| h.get(i).cloned())
                            .unwrap_or(Type::Any)
                    });
                    param_types.insert(param.var, ty);
                }
                // Area E §E.6 — layer in pre-scanned local types so `return x`
                // sees the unified type for a local that was widened across
                // multiple writes.
                if let Some(prescanned) = self.hir_types.per_function_prescan_var_types.get(func_id)
                {
                    for (var_id, ty) in prescanned {
                        // Don't clobber param types (param annotations win).
                        param_types.entry(*var_id).or_insert_with(|| ty.clone());
                    }
                }

                // Scan body for return statements (§1.17b-d — CFG-based)
                let return_type = self.infer_return_type_from_func(func, hir_module, &param_types);

                // Check for closure-returning functions (decorators)
                let return_type = if return_type == Type::None {
                    if self.find_returned_closure(func, hir_module).is_some() {
                        Type::Any
                    } else {
                        Type::None
                    }
                } else {
                    return_type
                };

                self.func_return_types.inner.insert(*func_id, return_type);
            }
        }
    }

    /// Re-infer return types for functions whose local types widened
    /// through the Area E §E.6 prescan (e.g. `x = 0; x += 0.5; return x`
    /// returns `Float`, not `Int`). Only touches functions that have a
    /// prescan entry and are NOT explicitly annotated — annotated
    /// signatures are authoritative.
    fn reinfer_return_types_with_prescan(&mut self, hir_module: &hir::Module) {
        let func_ids = hir_module.functions.to_vec();
        for func_id in &func_ids {
            let Some(func) = hir_module.func_defs.get(func_id) else {
                continue;
            };
            // Skip explicitly annotated functions.
            if let Some(ref rt) = func.return_type {
                if *rt != Type::None {
                    continue;
                }
            }
            if func.body.is_empty() {
                continue;
            }
            let Some(prescanned) = self
                .hir_types
                .per_function_prescan_var_types
                .get(func_id)
                .cloned()
            else {
                continue;
            };
            // Build param_types merging param annotations + prescan.
            let hints = self.closures.lambda_param_type_hints.get(func_id).cloned();
            let mut param_types: IndexMap<VarId, Type> = IndexMap::new();
            for (i, param) in func.params.iter().enumerate() {
                let ty = param.ty.clone().unwrap_or_else(|| {
                    hints
                        .as_ref()
                        .and_then(|h| h.get(i).cloned())
                        .unwrap_or(Type::Any)
                });
                param_types.insert(param.var, ty);
            }
            for (var_id, ty) in prescanned {
                param_types.entry(var_id).or_insert(ty);
            }
            let new_rt = self.infer_return_type_from_func(func, hir_module, &param_types);
            let final_rt = if new_rt == Type::None {
                if self.find_returned_closure(func, hir_module).is_some() {
                    Type::Any
                } else {
                    Type::None
                }
            } else {
                new_rt
            };
            self.func_return_types.inner.insert(*func_id, final_rt);
        }
    }

    /// Scan a function's CFG for return terminators/statements and infer
    /// the joined return type. §1.17b-d — prefers `func.blocks` over the
    /// legacy tree walker.
    fn infer_return_type_from_func(
        &self,
        func: &hir::Function,
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        let mut return_types = Vec::new();
        for block in func.blocks.values() {
            // `Return(expr)` can appear as a block terminator (normal path)
            // or as a straight-line stmt (defensive — the bridge always
            // lifts `Return` to the terminator).
            match &block.terminator {
                hir::HirTerminator::Return(Some(expr_id)) => {
                    let expr = &module.exprs[*expr_id];
                    return_types.push(self.infer_deep_expr_type(expr, module, param_types));
                }
                hir::HirTerminator::Return(None) => {
                    return_types.push(Type::None);
                }
                _ => {}
            }
            for &stmt_id in &block.stmts {
                let stmt = &module.stmts[stmt_id];
                match &stmt.kind {
                    hir::StmtKind::Return(Some(expr_id)) => {
                        let expr = &module.exprs[*expr_id];
                        return_types.push(self.infer_deep_expr_type(expr, module, param_types));
                    }
                    hir::StmtKind::Return(None) => {
                        return_types.push(Type::None);
                    }
                    _ => {}
                }
            }
        }
        Self::join_return_types(return_types)
    }

    /// Shared return-type joiner. See `infer_return_type_from_body` for
    /// rationale on keeping `NotImplemented` / `Any` handling.
    fn join_return_types(return_types: Vec<Type>) -> Type {
        if return_types.is_empty() {
            Type::None
        } else if return_types.len() == 1 {
            return_types
                .into_iter()
                .next()
                .expect("checked: return_types.len() == 1")
        } else {
            let concrete: Vec<Type> = return_types
                .into_iter()
                .filter(|t| *t != Type::Any)
                .collect();
            if concrete.is_empty() {
                Type::Any
            } else {
                Type::normalize_union(concrete)
            }
        }
    }

    // `infer_deep_expr_type` is now defined in `infer.rs` as part of the
    // unified `infer_expr_type_inner` engine.
}

/// §1.4u-b helper: collect `(handler.name, handler.ty)` pairs where both
/// are present, for every exception handler in the module. Used to seed
/// `HirTypeInference::base_var_types` with handler binding types.
///
/// §1.17b-d — reads `Function::try_scopes` directly instead of walking
/// the statement tree. Module-level init statements still walk the tree
/// (no containing CFG function).
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
    // Module init stmts are a flat Vec<StmtId>, not a CFG function. Walk
    // the tree to find `Try`-embedded handlers.
    collect_handler_binds_in_stmts(&module.module_init_stmts, module, &mut out);
    out
}

fn collect_handler_binds_in_stmts(
    stmts: &[hir::StmtId],
    module: &hir::Module,
    out: &mut Vec<(VarId, Type)>,
) {
    for sid in stmts {
        let stmt = &module.stmts[*sid];
        match &stmt.kind {
            hir::StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                collect_handler_binds_in_stmts(body, module, out);
                for h in handlers {
                    if let (Some(name), Some(ty)) = (h.name, h.ty.clone()) {
                        out.push((name, ty));
                    }
                    collect_handler_binds_in_stmts(&h.body, module, out);
                }
                collect_handler_binds_in_stmts(else_block, module, out);
                collect_handler_binds_in_stmts(finally_block, module, out);
            }
            hir::StmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                collect_handler_binds_in_stmts(then_block, module, out);
                collect_handler_binds_in_stmts(else_block, module, out);
            }
            hir::StmtKind::While {
                body, else_block, ..
            }
            | hir::StmtKind::ForBind {
                body, else_block, ..
            } => {
                collect_handler_binds_in_stmts(body, module, out);
                collect_handler_binds_in_stmts(else_block, module, out);
            }
            hir::StmtKind::Match { cases, .. } => {
                for case in cases {
                    collect_handler_binds_in_stmts(&case.body, module, out);
                }
            }
            _ => {}
        }
    }
}
