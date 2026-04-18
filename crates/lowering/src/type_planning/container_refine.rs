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
    pub(crate) fn refine_empty_container_types(&mut self, hir_module: &hir::Module) {
        // Scan module-level statements
        self.refine_empty_containers_in_block(&hir_module.module_init_stmts, hir_module);
        // Scan function bodies
        for func_id in hir_module.functions.iter() {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                self.refine_empty_containers_in_block(&func.body, hir_module);
            }
        }
    }

    /// Scan a flat statement block for `var = []` followed by `var.append(expr)`
    /// and refine the variable's type.
    fn refine_empty_containers_in_block(
        &mut self,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
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
                    let (key_ty, val_ty) =
                        self.find_dict_types_from_usage(target, &stmts[i + 1..], hir_module);
                    if key_ty != Type::Any || val_ty != Type::Any {
                        let refined = Type::Dict(Box::new(key_ty), Box::new(val_ty));
                        self.hir_types.refined_var_types.insert(target, refined);
                    }
                } else if is_empty_list || is_empty_set {
                    // Scan subsequent statements for method calls on this variable
                    let elem_ty =
                        self.find_elem_type_from_usage(target, &stmts[i + 1..], hir_module);

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

            // Recurse into nested blocks
            match &hir_module.stmts[*stmt_id].kind {
                hir::StmtKind::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.refine_empty_containers_in_block(then_block, hir_module);
                    self.refine_empty_containers_in_block(else_block, hir_module);
                }
                hir::StmtKind::ForBind {
                    body, else_block, ..
                }
                | hir::StmtKind::While {
                    body, else_block, ..
                } => {
                    self.refine_empty_containers_in_block(body, hir_module);
                    self.refine_empty_containers_in_block(else_block, hir_module);
                }
                hir::StmtKind::Try {
                    body,
                    handlers,
                    else_block,
                    finally_block,
                } => {
                    self.refine_empty_containers_in_block(body, hir_module);
                    for handler in handlers {
                        self.refine_empty_containers_in_block(&handler.body, hir_module);
                    }
                    self.refine_empty_containers_in_block(else_block, hir_module);
                    self.refine_empty_containers_in_block(finally_block, hir_module);
                }
                hir::StmtKind::Match { cases, .. } => {
                    for case in cases {
                        self.refine_empty_containers_in_block(&case.body, hir_module);
                    }
                }
                _ => {}
            }
        }
    }

    /// Look through subsequent statements for method calls that reveal the element type.
    fn find_elem_type_from_usage(
        &self,
        var: VarId,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
    ) -> Type {
        for stmt_id in stmts {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                hir::StmtKind::Expr(expr_id) => {
                    if let Some(ty) =
                        self.extract_elem_type_from_method_call(var, *expr_id, hir_module)
                    {
                        return ty;
                    }
                }
                // Also check inside assert statements: assert expr, msg
                // The assert condition is an expression statement
                hir::StmtKind::If { cond, .. } => {
                    if let Some(ty) =
                        self.extract_elem_type_from_method_call(var, *cond, hir_module)
                    {
                        return ty;
                    }
                }
                // Stop at reassignment to the same variable
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    ..
                } if *target_var == var => {
                    return Type::Any;
                }
                // Recurse into nested blocks
                hir::StmtKind::ForBind {
                    body, else_block, ..
                }
                | hir::StmtKind::While {
                    body, else_block, ..
                } => {
                    let result = self.find_elem_type_from_usage(var, body, hir_module);
                    if result != Type::Any {
                        return result;
                    }
                    let result = self.find_elem_type_from_usage(var, else_block, hir_module);
                    if result != Type::Any {
                        return result;
                    }
                }
                hir::StmtKind::Try {
                    body,
                    handlers,
                    else_block,
                    finally_block,
                } => {
                    let result = self.find_elem_type_from_usage(var, body, hir_module);
                    if result != Type::Any {
                        return result;
                    }
                    for handler in handlers {
                        let result = self.find_elem_type_from_usage(var, &handler.body, hir_module);
                        if result != Type::Any {
                            return result;
                        }
                    }
                    let result = self.find_elem_type_from_usage(var, else_block, hir_module);
                    if result != Type::Any {
                        return result;
                    }
                    let result = self.find_elem_type_from_usage(var, finally_block, hir_module);
                    if result != Type::Any {
                        return result;
                    }
                }
                hir::StmtKind::Match { cases, .. } => {
                    for case in cases {
                        let result = self.find_elem_type_from_usage(var, &case.body, hir_module);
                        if result != Type::Any {
                            return result;
                        }
                    }
                }
                _ => {}
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
                    let ty = self.infer_deep_expr_type(arg_expr, hir_module, &IndexMap::new());
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
                            &IndexMap::new(),
                        );
                        let val_ty = self.infer_deep_expr_type(
                            &hir_module.exprs[*value],
                            hir_module,
                            &IndexMap::new(),
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
                // Recurse into nested blocks
                hir::StmtKind::ForBind {
                    body, else_block, ..
                }
                | hir::StmtKind::While {
                    body, else_block, ..
                } => {
                    let result = self.find_dict_types_from_usage(var, body, hir_module);
                    if result != (Type::Any, Type::Any) {
                        return result;
                    }
                    let result = self.find_dict_types_from_usage(var, else_block, hir_module);
                    if result != (Type::Any, Type::Any) {
                        return result;
                    }
                }
                hir::StmtKind::Try {
                    body,
                    handlers,
                    else_block,
                    finally_block,
                } => {
                    let result = self.find_dict_types_from_usage(var, body, hir_module);
                    if result != (Type::Any, Type::Any) {
                        return result;
                    }
                    for handler in handlers {
                        let result =
                            self.find_dict_types_from_usage(var, &handler.body, hir_module);
                        if result != (Type::Any, Type::Any) {
                            return result;
                        }
                    }
                    let result = self.find_dict_types_from_usage(var, else_block, hir_module);
                    if result != (Type::Any, Type::Any) {
                        return result;
                    }
                    let result = self.find_dict_types_from_usage(var, finally_block, hir_module);
                    if result != (Type::Any, Type::Any) {
                        return result;
                    }
                }
                hir::StmtKind::Match { cases, .. } => {
                    for case in cases {
                        let result = self.find_dict_types_from_usage(var, &case.body, hir_module);
                        if result != (Type::Any, Type::Any) {
                            return result;
                        }
                    }
                }
                _ => {}
            }
        }
        (Type::Any, Type::Any)
    }
}
