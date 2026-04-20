//! Empty container type refinement
//!
//! When `li = []` has no type annotation, the type planner infers `List(Any)`.
//! This causes elem_tag=ELEM_HEAP_OBJ at runtime, but the lowering passes raw
//! i64 values for int appends, causing a mismatch that leads to segfaults.
//!
//! This pass scans statement blocks for empty container assignments and refines
//! their element type from subsequent method calls (append, insert, add, etc.).

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::Lowering;

impl<'a> Lowering<'a> {
    /// Refine types of empty containers by scanning for subsequent method calls.
    /// Must run before lowering so that `get_var_type` returns the refined type.
    ///
    /// §1.17b-d/f — all HIR functions, including the synthetic module-init
    /// function, are scanned via their CFG blocks in allocation order. Each
    /// block is treated as a flat stmt list; "subsequent uses" are read from
    /// `block.stmts[i+1..]`.
    pub(crate) fn refine_empty_container_types(&mut self, hir_module: &hir::Module) {
        for func_id in hir_module.functions.iter() {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                let overlay = self
                    .hir_types
                    .per_function_prescan_var_types
                    .get(func_id)
                    .cloned()
                    .unwrap_or_default();
                // Collect block stmt-lists first (avoid borrow conflict with
                // self.refine_empty_containers_in_block's &mut self).
                let block_stmt_lists: Vec<Vec<hir::StmtId>> =
                    func.blocks.values().map(|b| b.stmts.clone()).collect();
                for stmts in &block_stmt_lists {
                    self.refine_empty_containers_in_block(stmts, hir_module, &overlay);
                }
            }
        }
    }

    /// Scan a flat statement block for `var = []` followed by `var.append(expr)`
    /// and refine the variable's type.
    fn refine_empty_containers_in_block(
        &mut self,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) {
        for (i, stmt_id) in stmts.iter().enumerate() {
            let stmt = &hir_module.stmts[*stmt_id];

            // Look for: var = [] / {} / set() (no type hint, empty container)
            let empty_container = match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    value,
                    type_hint: None,
                } => Some((*target_var, *value)),
                _ => None,
            };
            if let Some((target, value)) = empty_container {
                let expr = &hir_module.exprs[value];
                let is_empty_list =
                    matches!(&expr.kind, hir::ExprKind::List(elems) if elems.is_empty());
                let is_empty_set =
                    matches!(&expr.kind, hir::ExprKind::Set(elems) if elems.is_empty());
                let is_empty_dict =
                    matches!(&expr.kind, hir::ExprKind::Dict(pairs) if pairs.is_empty());

                if is_empty_dict {
                    // Scan for d[key] = value assignments to infer key/value types
                    let (key_ty, val_ty) = self.find_dict_types_from_usage(
                        target,
                        &stmts[i + 1..],
                        hir_module,
                        overlay,
                    );
                    if key_ty != Type::Any || val_ty != Type::Any {
                        let refined = Type::Dict(Box::new(key_ty), Box::new(val_ty));
                        self.hir_types.refined_var_types.insert(target, refined);
                    }
                } else if is_empty_list || is_empty_set {
                    // Scan subsequent statements for method calls on this variable
                    let elem_ty = self.find_elem_type_from_usage(
                        target,
                        &stmts[i + 1..],
                        hir_module,
                        overlay,
                    );

                    if elem_ty != Type::Any {
                        let refined = if is_empty_list {
                            Type::List(Box::new(elem_ty))
                        } else {
                            Type::Set(Box::new(elem_ty))
                        };
                        // Store in refined_var_types which persists across function lowerings
                        self.hir_types.refined_var_types.insert(target, refined);
                    }
                }
            }
        }
    }

    /// Look through subsequent statements for method calls that reveal the element type.
    fn find_elem_type_from_usage(
        &self,
        var: VarId,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) -> Type {
        for stmt_id in stmts {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                hir::StmtKind::Expr(expr_id) => {
                    if let Some(ty) =
                        self.extract_elem_type_from_method_call(var, *expr_id, hir_module, overlay)
                    {
                        return ty;
                    }
                }
                // Stop at reassignment to the same variable — any
                // subsequent `.append` targets a different list.
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    ..
                } if *target_var == var => {
                    return Type::Any;
                }
                // Nested closure that captures our variable — recurse into
                // the closure function's body, replacing the captured-var
                // references with the corresponding `__capture_*` param.
                // Catches the idiomatic Python pattern
                //     topo = []
                //     def build_topo(v):
                //         topo.append(v)   # captures topo from the outer scope
                //     build_topo(self)
                // where the `.append()` that reveals the element type lives
                // inside a nested function, not as a sibling of the empty-list
                // bind.
                hir::StmtKind::Bind { value, .. } => {
                    let value_expr = &hir_module.exprs[*value];
                    if let hir::ExprKind::Closure {
                        func: closure_func_id,
                        captures,
                    } = &value_expr.kind
                    {
                        for (cap_idx, cap_id) in captures.iter().enumerate() {
                            let cap_expr = &hir_module.exprs[*cap_id];
                            if !matches!(&cap_expr.kind, hir::ExprKind::Var(v) if *v == var) {
                                continue;
                            }
                            // Captured vars become the first `cap_idx`
                            // leading params (`__capture_*`) on the closure
                            // function — translate `var` to the matching
                            // capture-param VarId so the recursion keys on
                            // the right identifier inside the callee body.
                            let Some(closure_func) = hir_module.func_defs.get(closure_func_id)
                            else {
                                continue;
                            };
                            let Some(capture_param) = closure_func.params.get(cap_idx) else {
                                continue;
                            };
                            // Use the closure's own prescan overlay so
                            // `append(v)` where `v` is a closure param
                            // resolves to the nested-function-inferred
                            // type (via `infer_nested_function_param_types`)
                            // rather than `Any`.
                            let closure_overlay = self
                                .hir_types
                                .per_function_prescan_var_types
                                .get(closure_func_id)
                                .cloned()
                                .unwrap_or_default();
                            for block in closure_func.blocks.values() {
                                let result = self.find_elem_type_from_usage(
                                    capture_param.var,
                                    &block.stmts,
                                    hir_module,
                                    &closure_overlay,
                                );
                                if result != Type::Any {
                                    return result;
                                }
                            }
                        }
                    }
                }
                hir::StmtKind::Return(_)
                | hir::StmtKind::Break
                | hir::StmtKind::Continue
                | hir::StmtKind::Raise { .. }
                | hir::StmtKind::Pass
                | hir::StmtKind::Assert { .. }
                | hir::StmtKind::IndexDelete { .. }
                | hir::StmtKind::IterAdvance { .. }
                | hir::StmtKind::IterSetup { .. } => {}
            }
        }
        Type::Any
    }

    /// Check if an expression is `var.append(expr)` / `var.insert(_, expr)` / `var.add(expr)`
    /// and return the element type from the argument.
    fn extract_elem_type_from_method_call(
        &self,
        var: VarId,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) -> Option<Type> {
        let expr = &hir_module.exprs[expr_id];
        if let hir::ExprKind::MethodCall {
            obj, method, args, ..
        } = &expr.kind
        {
            // Check that the object is our variable
            let obj_expr = &hir_module.exprs[*obj];
            if !matches!(&obj_expr.kind, hir::ExprKind::Var(v) if *v == var) {
                return None;
            }

            let method_name = self.interner.resolve(*method);
            let value_arg_idx = match method_name {
                "append" | "add" | "remove" => Some(0),
                "insert" => Some(1), // insert(index, value)
                _ => None,
            };

            if let Some(idx) = value_arg_idx {
                if let Some(arg_id) = args.get(idx) {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    let ty = self.infer_deep_expr_type(arg_expr, hir_module, overlay);
                    if ty != Type::Any {
                        return Some(ty);
                    }
                }
            }
        }
        None
    }

    /// Look through subsequent statements for dict index assignments (`d[key] = value`)
    /// that reveal the key and value types.
    fn find_dict_types_from_usage(
        &self,
        var: VarId,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) -> (Type, Type) {
        for stmt_id in stmts {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Index { obj, index, .. },
                    value,
                    ..
                } => {
                    let obj_expr = &hir_module.exprs[*obj];
                    if matches!(&obj_expr.kind, hir::ExprKind::Var(v) if *v == var) {
                        let key_ty = self.infer_deep_expr_type(
                            &hir_module.exprs[*index],
                            hir_module,
                            overlay,
                        );
                        let val_ty = self.infer_deep_expr_type(
                            &hir_module.exprs[*value],
                            hir_module,
                            overlay,
                        );
                        if key_ty != Type::Any && val_ty != Type::Any {
                            return (key_ty, val_ty);
                        }
                    }
                }
                // Stop at reassignment to the same variable
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    ..
                } if *target_var == var => {
                    return (Type::Any, Type::Any);
                }
                hir::StmtKind::Bind { .. }
                | hir::StmtKind::Expr(_)
                | hir::StmtKind::Return(_)
                | hir::StmtKind::Break
                | hir::StmtKind::Continue
                | hir::StmtKind::Raise { .. }
                | hir::StmtKind::Pass
                | hir::StmtKind::Assert { .. }
                | hir::StmtKind::IndexDelete { .. }
                | hir::StmtKind::IterAdvance { .. }
                | hir::StmtKind::IterSetup { .. } => {}
            }
        }
        (Type::Any, Type::Any)
    }
}
