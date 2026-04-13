//! Closure/lambda pre-scanning
//!
//! Pre-compute closure capture types from module-level statements and function
//! bodies. Handles decorator patterns, lambda parameter type hints for HOFs
//! (map/filter/reduce/sorted/min/max key=), and inline closure discovery.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use super::infer::extract_iterable_element_type;
use crate::Lowering;

impl<'a> Lowering<'a> {
    // ==================== Pre-computation Phase ====================

    /// Pre-compute closure capture types from module-level statements and function bodies.
    /// This must run before lowering functions so that lambda/closure type inference
    /// can use the captured variable types.
    pub(crate) fn precompute_closure_capture_types(&mut self, hir_module: &hir::Module) {
        // Track module-level variable types as we scan statements
        let mut module_var_types: IndexMap<VarId, Type> = IndexMap::new();

        // First, scan module-level statements
        for stmt_id in &hir_module.module_init_stmts {
            self.scan_stmt_for_closures(*stmt_id, hir_module, &mut module_var_types);
        }

        // Then, scan all function bodies
        for func_id in &hir_module.functions {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                // Build variable types from function parameters
                let mut func_var_types: IndexMap<VarId, Type> = IndexMap::new();
                for param in &func.params {
                    if let Some(ref ty) = param.ty {
                        func_var_types.insert(param.var, ty.clone());
                    }
                }
                // Scan function body for closures
                for stmt_id in &func.body {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, &mut func_var_types);
                }
            }
        }
    }

    /// Recursively scan a statement for closure assignments and record capture types
    fn scan_stmt_for_closures(
        &mut self,
        stmt_id: hir::StmtId,
        hir_module: &hir::Module,
        var_types: &mut IndexMap<VarId, Type>,
    ) {
        let stmt = &hir_module.stmts[stmt_id];
        match &stmt.kind {
            hir::StmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                for stmt_id in then_block {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
                for stmt_id in else_block {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
            }
            hir::StmtKind::ForBind {
                body, else_block, ..
            }
            | hir::StmtKind::While {
                body, else_block, ..
            } => {
                for stmt_id in body {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
                for stmt_id in else_block {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
            }
            hir::StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                for stmt_id in body {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
                for handler in handlers {
                    for stmt_id in &handler.body {
                        self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                    }
                }
                for stmt_id in else_block {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
                for stmt_id in finally_block {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
            }
            hir::StmtKind::Match { subject, cases } => {
                let subj_expr = &hir_module.exprs[*subject];
                self.scan_expr_for_closures(subj_expr, hir_module, var_types);
                for case in cases {
                    if let Some(guard) = &case.guard {
                        let guard_expr = &hir_module.exprs[*guard];
                        self.scan_expr_for_closures(guard_expr, hir_module, var_types);
                    }
                    for stmt_id in &case.body {
                        self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                    }
                }
            }
            hir::StmtKind::Bind {
                target,
                value,
                type_hint,
            } => {
                let expr = &hir_module.exprs[*value];

                if let hir::BindingTarget::Var(target_var) = target {
                    // Determine the variable type (mirrors the Assign branch above)
                    let var_type = type_hint
                        .clone()
                        .unwrap_or_else(|| self.infer_deep_expr_type(expr, hir_module, var_types));
                    var_types.insert(*target_var, var_type);

                    // Scan the value expression for inline closures
                    self.scan_expr_for_closures(expr, hir_module, var_types);

                    // Check for decorated function pattern: var = decorator(FuncRef(func))
                    if let hir::ExprKind::Call {
                        func: call_func, ..
                    } = &expr.kind
                    {
                        if let Some(innermost_func_id) =
                            self.find_innermost_func_ref(expr, hir_module)
                        {
                            let call_func_expr = &hir_module.exprs[*call_func];
                            if let hir::ExprKind::FuncRef(decorator_func_id) = &call_func_expr.kind
                            {
                                if let Some(decorator_def) =
                                    hir_module.func_defs.get(decorator_func_id)
                                {
                                    if let Some(wrapper_func_id) =
                                        self.find_returned_closure(decorator_def, hir_module)
                                    {
                                        self.insert_wrapper_func_id(wrapper_func_id);
                                        self.closures
                                            .decorated_to_wrapper
                                            .insert(innermost_func_id, wrapper_func_id);
                                        if let Some(func_param) = decorator_def.params.first() {
                                            self.closures
                                                .wrapper_func_param_name
                                                .insert(wrapper_func_id, func_param.name);
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // For Tuple/Attr/Index/ClassAttr targets, record each bound Var
                    // with Any type and scan the value for closures.
                    target.for_each_var(&mut |var_id| {
                        var_types.insert(var_id, Type::Any);
                    });
                    self.scan_expr_for_closures(expr, hir_module, var_types);
                }
            }
            hir::StmtKind::Expr(expr_id) => {
                // Scan expression statements (like function calls) for inline closures
                let expr = &hir_module.exprs[*expr_id];
                self.scan_expr_for_closures(expr, hir_module, var_types);
            }
            hir::StmtKind::Return(Some(expr_id)) => {
                // Scan return expressions for inline closures
                let expr = &hir_module.exprs[*expr_id];
                self.scan_expr_for_closures(expr, hir_module, var_types);
            }
            // Other statement types don't contain nested closures
            _ => {}
        }
    }

    /// Recursively scan an expression for inline closures and record their capture types
    fn scan_expr_for_closures(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        var_types: &IndexMap<VarId, Type>,
    ) {
        match &expr.kind {
            hir::ExprKind::Closure { func, captures } => {
                // Found an inline closure - record its capture types
                if !self.has_closure_capture_types(func) {
                    let mut capture_types = Vec::new();
                    for capture_id in captures {
                        let capture_expr = &hir_module.exprs[*capture_id];
                        let capture_type =
                            self.infer_deep_expr_type(capture_expr, hir_module, var_types);
                        capture_types.push(capture_type);
                    }
                    self.insert_closure_capture_types(*func, capture_types);
                }
            }
            hir::ExprKind::Call {
                func, args, kwargs, ..
            } => {
                // Scan function and all arguments
                let func_expr = &hir_module.exprs[*func];
                self.scan_expr_for_closures(func_expr, hir_module, var_types);
                for call_arg in args {
                    let arg_id = match call_arg {
                        hir::CallArg::Regular(expr_id) | hir::CallArg::Starred(expr_id) => expr_id,
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    self.scan_expr_for_closures(arg_expr, hir_module, var_types);
                }
                for kw in kwargs {
                    let kw_expr = &hir_module.exprs[kw.value];
                    self.scan_expr_for_closures(kw_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::BuiltinCall {
                builtin,
                args,
                kwargs,
                ..
            } => {
                // Register lambda parameter type hints for builtin HOFs
                // map/filter: callback takes 1 element parameter
                if matches!(builtin, hir::Builtin::Map | hir::Builtin::Filter) && args.len() >= 2 {
                    self.register_lambda_hints_from_iterable(
                        &hir_module.exprs[args[0]],
                        &hir_module.exprs[args[1]],
                        hir_module,
                        var_types,
                        1,
                        |elem| vec![elem],
                    );
                }
                // reduce: callback takes 2 element parameters (acc, elem)
                if matches!(builtin, hir::Builtin::Reduce) && args.len() >= 2 {
                    self.register_lambda_hints_from_iterable(
                        &hir_module.exprs[args[0]],
                        &hir_module.exprs[args[1]],
                        hir_module,
                        var_types,
                        2,
                        |elem| vec![elem.clone(), elem],
                    );
                }
                // sorted/min/max key=: key callback takes 1 element parameter
                if matches!(
                    builtin,
                    hir::Builtin::Sorted | hir::Builtin::Min | hir::Builtin::Max
                ) && !args.is_empty()
                {
                    let key_func = kwargs.iter().find_map(|kw| {
                        let kw_name = self.interner.resolve(kw.name);
                        if kw_name == "key" {
                            Some(&hir_module.exprs[kw.value])
                        } else {
                            None
                        }
                    });
                    if let Some(key_expr) = key_func {
                        self.register_lambda_hints_from_iterable(
                            key_expr,
                            &hir_module.exprs[args[0]],
                            hir_module,
                            var_types,
                            1,
                            |elem| vec![elem],
                        );
                    }
                }

                // Scan all arguments (this catches map(lambda ..., ...), filter(lambda ..., ...), etc.)
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    self.scan_expr_for_closures(arg_expr, hir_module, var_types);
                }
                for kw in kwargs {
                    let kw_expr = &hir_module.exprs[kw.value];
                    self.scan_expr_for_closures(kw_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::MethodCall {
                obj, args, kwargs, ..
            } => {
                let obj_expr = &hir_module.exprs[*obj];
                self.scan_expr_for_closures(obj_expr, hir_module, var_types);
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    self.scan_expr_for_closures(arg_expr, hir_module, var_types);
                }
                for kw in kwargs {
                    let kw_expr = &hir_module.exprs[kw.value];
                    self.scan_expr_for_closures(kw_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::BinOp { left, right, .. }
            | hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];
                self.scan_expr_for_closures(left_expr, hir_module, var_types);
                self.scan_expr_for_closures(right_expr, hir_module, var_types);
            }
            hir::ExprKind::UnOp { operand, .. } => {
                let operand_expr = &hir_module.exprs[*operand];
                self.scan_expr_for_closures(operand_expr, hir_module, var_types);
            }
            hir::ExprKind::Compare { left, right, .. } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];
                self.scan_expr_for_closures(left_expr, hir_module, var_types);
                self.scan_expr_for_closures(right_expr, hir_module, var_types);
            }
            hir::ExprKind::List(elements)
            | hir::ExprKind::Tuple(elements)
            | hir::ExprKind::Set(elements) => {
                for elem_id in elements {
                    let elem_expr = &hir_module.exprs[*elem_id];
                    self.scan_expr_for_closures(elem_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::Dict(pairs) => {
                for (key_id, val_id) in pairs {
                    let key_expr = &hir_module.exprs[*key_id];
                    let val_expr = &hir_module.exprs[*val_id];
                    self.scan_expr_for_closures(key_expr, hir_module, var_types);
                    self.scan_expr_for_closures(val_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::Index { obj, index } => {
                let obj_expr = &hir_module.exprs[*obj];
                let index_expr = &hir_module.exprs[*index];
                self.scan_expr_for_closures(obj_expr, hir_module, var_types);
                self.scan_expr_for_closures(index_expr, hir_module, var_types);
            }
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                let cond_expr = &hir_module.exprs[*cond];
                let then_expr = &hir_module.exprs[*then_val];
                let else_expr = &hir_module.exprs[*else_val];
                self.scan_expr_for_closures(cond_expr, hir_module, var_types);
                self.scan_expr_for_closures(then_expr, hir_module, var_types);
                self.scan_expr_for_closures(else_expr, hir_module, var_types);
            }
            // Primitives and other simple expressions don't contain closures
            _ => {}
        }
    }

    // ==================== Lambda Hint Registration ====================

    /// Register lambda parameter type hints for a callback that takes elements from an iterable.
    /// Shared by map/filter (1 param), reduce (2 params), and sorted/min/max key= (1 param).
    pub(crate) fn register_lambda_hints_from_iterable(
        &mut self,
        func_expr: &hir::Expr,
        iterable_expr: &hir::Expr,
        hir_module: &hir::Module,
        var_types: &IndexMap<VarId, Type>,
        expected_non_capture: usize,
        make_hints: impl FnOnce(Type) -> Vec<Type>,
    ) {
        let iterable_type = self.infer_deep_expr_type(iterable_expr, hir_module, var_types);
        let elem_type = extract_iterable_element_type(&iterable_type);
        if matches!(elem_type, Type::Any) {
            return;
        }
        let func_info = match &func_expr.kind {
            hir::ExprKind::FuncRef(func_id) => Some((*func_id, vec![])),
            hir::ExprKind::Closure { func, captures } => Some((*func, captures.clone())),
            _ => None,
        };
        let Some((func_id, captures)) = func_info else {
            return;
        };
        let Some(func_def) = hir_module.func_defs.get(&func_id) else {
            return;
        };
        let num_non_capture = func_def.params.len().saturating_sub(captures.len());
        if num_non_capture != expected_non_capture {
            return;
        }
        let mut param_hints = Vec::new();
        for cap_id in &captures {
            let cap_expr = &hir_module.exprs[*cap_id];
            let cap_type = self.infer_deep_expr_type(cap_expr, hir_module, var_types);
            param_hints.push(cap_type);
        }
        param_hints.extend(make_hints(elem_type));
        self.insert_lambda_param_type_hints(func_id, param_hints);
    }
}
