//! Lambda type inference
//!
//! Infers parameter types and return types for lambda functions and callbacks
//! used with HOFs (map/filter/reduce/sorted/min/max key=).

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::Lowering;

impl<'a> Lowering<'a> {
    // ==================== Lambda Parameter Type Inference ====================

    /// Infer parameter types for a lambda function from its body
    pub(crate) fn infer_lambda_param_types(
        &self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Vec<Type> {
        // Check if we have caller-provided parameter type hints (e.g., from reduce)
        if let Some(hints) = self.get_lambda_param_type_hints(&func.id) {
            if hints.len() == func.params.len() {
                return hints.clone();
            }
        }

        // Check if we have pre-computed capture types for this lambda
        let capture_types = self.get_closure_capture_types(&func.id).cloned();
        // Build a map of param var_id to param index
        let mut var_to_index: IndexMap<VarId, usize> = IndexMap::new();
        for (i, param) in func.params.iter().enumerate() {
            var_to_index.insert(param.var, i);
        }

        let mut inferred_types: Vec<Option<Type>> = vec![None; func.params.len()];

        // For closure capture parameters, use the pre-computed capture types
        if let Some(ref capture_types) = capture_types {
            for (i, ty) in capture_types.iter().enumerate() {
                if i < func.params.len() {
                    inferred_types[i] = Some(ty.clone());
                }
            }
        }

        // §1.17b-d — lambda bodies are a single CFG block whose terminator
        // is `Return(Some(expr))`. Walk from `entry_block`.
        if let Some(entry) = func.blocks.get(&func.entry_block) {
            if let hir::HirTerminator::Return(Some(expr_id)) = &entry.terminator {
                let expr = &hir_module.exprs[*expr_id];
                self.infer_types_from_expr(expr, hir_module, &var_to_index, &mut inferred_types);
            }
        }

        // Convert to Vec<Type>, using Type::Any for unresolved parameters
        inferred_types
            .into_iter()
            .map(|opt| opt.unwrap_or(Type::Any))
            .collect()
    }

    /// Recursively infer parameter types from an expression
    fn infer_types_from_expr(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        var_to_index: &IndexMap<VarId, usize>,
        inferred_types: &mut Vec<Option<Type>>,
    ) {
        match &expr.kind {
            hir::ExprKind::BinOp { left, right, op } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                // If one side is a literal, infer type for the other side
                let left_type = self.get_literal_type(left_expr);
                let right_type = self.get_literal_type(right_expr);

                // For string operations, infer string types
                if matches!(left_type, Some(Type::Str)) || matches!(right_type, Some(Type::Str)) {
                    if let hir::ExprKind::Var(var_id) = &left_expr.kind {
                        if let Some(&idx) = var_to_index.get(var_id) {
                            if inferred_types[idx].is_none() {
                                inferred_types[idx] = Some(Type::Str);
                            }
                        }
                    }
                    if let hir::ExprKind::Var(var_id) = &right_expr.kind {
                        if let Some(&idx) = var_to_index.get(var_id) {
                            if inferred_types[idx].is_none() {
                                inferred_types[idx] = Some(Type::Str);
                            }
                        }
                    }
                } else if matches!(left_type, Some(Type::Float))
                    || matches!(right_type, Some(Type::Float))
                    || matches!(op, hir::BinOp::Div)
                {
                    // Float operations
                    if let hir::ExprKind::Var(var_id) = &left_expr.kind {
                        if let Some(&idx) = var_to_index.get(var_id) {
                            if inferred_types[idx].is_none() {
                                inferred_types[idx] = Some(Type::Float);
                            }
                        }
                    }
                    if let hir::ExprKind::Var(var_id) = &right_expr.kind {
                        if let Some(&idx) = var_to_index.get(var_id) {
                            if inferred_types[idx].is_none() {
                                inferred_types[idx] = Some(Type::Float);
                            }
                        }
                    }
                } else {
                    // No literal context — leave as None (becomes Type::Any)
                    // Cannot assume Int: could be string concatenation, float arithmetic, etc.
                }

                // Recurse into subexpressions
                self.infer_types_from_expr(left_expr, hir_module, var_to_index, inferred_types);
                self.infer_types_from_expr(right_expr, hir_module, var_to_index, inferred_types);
            }
            hir::ExprKind::Compare { left, right, op } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                // Infer types from comparison - if one side is literal or already known, infer for the other
                let left_type = self.get_literal_type(left_expr);
                let right_type = self.get_literal_type(right_expr);

                // Also check for already-inferred types from captures
                let left_known_type = if let hir::ExprKind::Var(var_id) = &left_expr.kind {
                    if let Some(&idx) = var_to_index.get(var_id) {
                        inferred_types[idx].clone()
                    } else {
                        left_type.clone()
                    }
                } else {
                    left_type.clone()
                };

                let right_known_type = if let hir::ExprKind::Var(var_id) = &right_expr.kind {
                    if let Some(&idx) = var_to_index.get(var_id) {
                        inferred_types[idx].clone()
                    } else {
                        right_type.clone()
                    }
                } else {
                    right_type.clone()
                };

                // For "in" operator, the element type should match the container's element type
                // For string "in", both should be Str (substring check)
                let is_in_op = matches!(op, hir::CmpOp::In | hir::CmpOp::NotIn);

                if let hir::ExprKind::Var(var_id) = &left_expr.kind {
                    if let Some(&idx) = var_to_index.get(var_id) {
                        if inferred_types[idx].is_none() {
                            if let Some(ty) = right_known_type.clone() {
                                // For "in" with string container, element should also be string
                                if is_in_op && matches!(ty, Type::Str) {
                                    inferred_types[idx] = Some(Type::Str);
                                } else if !is_in_op {
                                    inferred_types[idx] = Some(ty);
                                }
                            }
                        }
                    }
                }
                if let hir::ExprKind::Var(var_id) = &right_expr.kind {
                    if let Some(&idx) = var_to_index.get(var_id) {
                        if inferred_types[idx].is_none() {
                            if let Some(ty) = left_known_type.clone() {
                                // For "in" with string element, container should also be string
                                if is_in_op && matches!(ty, Type::Str) {
                                    inferred_types[idx] = Some(Type::Str);
                                } else if !is_in_op {
                                    inferred_types[idx] = Some(ty);
                                }
                            }
                        }
                    }
                }
            }
            hir::ExprKind::UnOp { operand, .. } => {
                let operand_expr = &hir_module.exprs[*operand];
                self.infer_types_from_expr(operand_expr, hir_module, var_to_index, inferred_types);
            }
            hir::ExprKind::Call { args, .. } => {
                for arg in args {
                    let arg_id = match arg {
                        hir::CallArg::Regular(id) => id,
                        hir::CallArg::Starred(id) => id,
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    self.infer_types_from_expr(arg_expr, hir_module, var_to_index, inferred_types);
                }
            }
            _ => {}
        }
    }

    /// Get the type of a literal expression
    fn get_literal_type(&self, expr: &hir::Expr) -> Option<Type> {
        match &expr.kind {
            hir::ExprKind::Int(_) => Some(Type::Int),
            hir::ExprKind::Float(_) => Some(Type::Float),
            hir::ExprKind::Bool(_) => Some(Type::Bool),
            hir::ExprKind::Str(_) => Some(Type::Str),
            hir::ExprKind::None => Some(Type::None),
            _ => None,
        }
    }

    // ==================== Lambda Return Type Inference ====================

    /// Infer the return type of a callback function (for map(), filter(), sorted(key=), etc.)
    /// This checks multiple sources in order:
    /// 1. Pre-computed return types from function definitions
    /// 2. Explicit return type annotation on the function
    /// 3. Lambda body analysis for closures
    /// 4. Fallback to Type::Any
    pub(crate) fn infer_callback_return_type(
        &self,
        func_id: pyaot_utils::FuncId,
        hir_module: &hir::Module,
    ) -> Type {
        // Check if we have a pre-computed return type
        if let Some(ret_type) = self.get_func_return_type(&func_id) {
            return ret_type.clone();
        }

        // Look up the function definition
        if let Some(func_def) = hir_module.func_defs.get(&func_id) {
            // Check for explicit return type annotation
            if let Some(ref return_type) = func_def.return_type {
                return return_type.clone();
            }

            // For lambdas (functions with simple bodies), infer from body
            // Lambda functions typically have a single return statement
            if func_def.body.len() == 1 {
                return self.infer_lambda_return_type(func_def, hir_module);
            }
        }

        // Fallback for cases where we can't determine the type
        Type::Any
    }

    /// Infer return type for a lambda function from its body
    pub(crate) fn infer_lambda_return_type(
        &self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> Type {
        // Build a map of param var_id to type from inferred param types
        let param_types = self.infer_lambda_param_types(func, hir_module);
        let mut param_type_map: IndexMap<VarId, Type> = IndexMap::new();
        for (i, param) in func.params.iter().enumerate() {
            if i < param_types.len() {
                param_type_map.insert(param.var, param_types[i].clone());
            }
        }

        // §1.17b-d — scan the CFG for any `Return(Some(expr))` terminator
        // and infer from its expr. The first matching block wins (same
        // semantics as the former tree walk). Blocks without a Return
        // terminator fall through and the function returns `None`.
        for block in func.blocks.values() {
            if let hir::HirTerminator::Return(Some(expr_id)) = &block.terminator {
                let expr = &hir_module.exprs[*expr_id];
                return self.infer_deep_expr_type(expr, hir_module, &param_type_map);
            }
        }
        Type::None
    }
}
