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
        self.refine_empty_container_types(hir_module);
        self.infer_all_return_types(hir_module);
        self.precompute_all_local_var_types(hir_module);
        // Area E §E.6 — re-infer return types for unannotated functions
        // after prescan has widened any numeric locals (e.g.
        // `x = 0; x += 0.5; return x` → `Float`, not `Int`).
        self.reinfer_return_types_with_prescan(hir_module);
        self.validate_type_annotations(hir_module);
        // §1.4u-b step 3 — populate the stable per-module Var→Type
        // base map. `get_base_var_type` reads this alongside
        // `symbols.var_types` so the type-query API does not need to
        // thread a "current function" context. The cache is populated
        // once from annotated params, prescan locals, and exception-
        // handler binding types; never mutated afterwards.
        //
        // The eager expr_types pre-pass that would pair with this map
        // (caching every non-Var expression type up front) is NOT
        // wired in yet — empirical testing showed the existing
        // narrowing model depends on non-Var composition types being
        // computed live inside the narrowing frame, not served from a
        // pre-narrowing cache. Fixing that requires the dispatch-site
        // audit planned for §1.4u-b step 4.
        self.populate_base_var_types(hir_module);
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

    /// Get the type of an expression by its ID (memoized).
    ///
    /// Post-§1.4u-b: non-`Var` expression types are pure functions of
    /// the HIR and F/M (function/module) state. They are cached eagerly
    /// at the end of `run_type_planning` via `eagerly_populate_expr_types`,
    /// so this call is typically a pure cache hit during lowering.
    ///
    /// `Var` expressions still bypass the cache — their **effective**
    /// type at a use site may include isinstance narrowing applied via
    /// `push_narrowing_frame`, and that narrowing only lives in
    /// `symbols.var_types` which the base-type cache intentionally does
    /// not read. Callers that care about the effective narrowed type
    /// must use `get_var_type` at emission time. Callers that want the
    /// base/declared type can rely on the cache here.
    pub(crate) fn get_type_of_expr_id(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
    ) -> Type {
        let expr = &hir_module.exprs[expr_id];
        if matches!(expr.kind, hir::ExprKind::Var(_)) {
            return self.compute_expr_type(expr, hir_module);
        }
        if let Some(cached) = self.hir_types.lookup(expr_id).cloned() {
            return cached;
        }
        let result = self.compute_expr_type(expr, hir_module);
        self.hir_types.insert_type(expr_id, result.clone());
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

                // Scan body for return statements
                let return_type =
                    self.infer_return_type_from_body(&func.body, hir_module, &param_types);

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
            let new_rt = self.infer_return_type_from_body(&func.body, hir_module, &param_types);
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

    /// Scan a function body for return statements and infer the return type.
    fn infer_return_type_from_body(
        &self,
        body: &[hir::StmtId],
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        let mut return_types = Vec::new();

        for stmt_id in body {
            self.collect_return_types(*stmt_id, module, param_types, &mut return_types);
        }

        // `NotImplemented` is a control-flow sentinel. We KEEP it in the
        // return-type union so the compiled function's Cranelift signature
        // returns a pointer (NotImplementedT is heap-allocated) and the
        // operator dispatch can identity-compare the result against the
        // singleton. Without this, a dunder that ONLY returns NotImplemented
        // would have signature returning `None` (i8) and the i64 pointer
        // would be silently truncated.
        if return_types.is_empty() {
            Type::None
        } else if return_types.len() == 1 {
            return_types
                .into_iter()
                .next()
                .expect("checked: return_types.len() == 1")
        } else {
            // Multiple return types — union concrete types
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

    /// Recursively collect return types from statements.
    fn collect_return_types(
        &self,
        stmt_id: hir::StmtId,
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
        return_types: &mut Vec<Type>,
    ) {
        let stmt = &module.stmts[stmt_id];
        match &stmt.kind {
            hir::StmtKind::Return(Some(expr_id)) => {
                let expr = &module.exprs[*expr_id];
                let ty = self.infer_deep_expr_type(expr, module, param_types);
                return_types.push(ty);
            }
            hir::StmtKind::Return(None) => {
                return_types.push(Type::None);
            }
            hir::StmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                for s in then_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for s in else_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
            }
            hir::StmtKind::ForBind {
                body, else_block, ..
            }
            | hir::StmtKind::While {
                body, else_block, ..
            } => {
                for s in body {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for s in else_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
            }
            hir::StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                for s in body {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for handler in handlers {
                    for s in &handler.body {
                        self.collect_return_types(*s, module, param_types, return_types);
                    }
                }
                for s in else_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for s in finally_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
            }
            hir::StmtKind::Match { cases, .. } => {
                for case in cases {
                    for s in &case.body {
                        self.collect_return_types(*s, module, param_types, return_types);
                    }
                }
            }
            _ => {}
        }
    }

    // `infer_deep_expr_type` is now defined in `infer.rs` as part of the
    // unified `infer_expr_type_inner` engine.
}

/// §1.4u-b helper: walk every `StmtKind::Try` in the module (including
/// nested function bodies, class bodies, and other control-flow
/// contexts) and collect `(handler.name, handler.ty)` pairs where
/// both are present. Used to seed `HirTypeInference::base_var_types`
/// with exception-handler binding types, which are otherwise only
/// populated at lowering time via `insert_var_type`.
fn collect_handler_binds(module: &hir::Module) -> Vec<(VarId, Type)> {
    let mut out = Vec::new();
    for func in module.func_defs.values() {
        collect_handler_binds_in_stmts(&func.body, module, &mut out);
    }
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
