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
        self.validate_type_annotations(hir_module);
    }

    /// Get the type of an expression by its ID (memoized).
    pub(crate) fn get_type_of_expr_id(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
    ) -> Type {
        if let Some(cached) = self.types.expr_types.get(&expr_id).cloned() {
            return cached;
        }
        let expr = &hir_module.exprs[expr_id];
        let result = self.compute_expr_type(expr, hir_module);
        self.types.expr_types.insert(expr_id, result.clone());
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
            hir::StmtKind::For {
                body, else_block, ..
            }
            | hir::StmtKind::ForUnpack {
                body, else_block, ..
            }
            | hir::StmtKind::ForUnpackStarred {
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
