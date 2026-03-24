//! Pre-scan: closure/lambda/decorator discovery + lambda type inference
//! + empty container type refinement
//!
//! Moved from lambda_inference.rs. Handles pre-scan for closures, decorator
//! patterns, lambda parameter inference, and lambda return type inference.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use super::infer::extract_iterable_element_type;
use crate::Lowering;

// =============================================================================
// Empty container type refinement
// =============================================================================
//
// When `li = []` has no type annotation, the type planner infers `List(Any)`.
// This causes elem_tag=ELEM_HEAP_OBJ at runtime, but the lowering passes raw
// i64 values for int appends, causing a mismatch that leads to segfaults.
//
// This pass scans statement blocks for empty container assignments and refines
// their element type from subsequent method calls (append, insert, add, etc.).

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

            // Look for: var = [] (no type hint, empty list)
            if let hir::StmtKind::Assign {
                target,
                value,
                type_hint: None,
            } = &stmt.kind
            {
                let expr = &hir_module.exprs[*value];
                let is_empty_list =
                    matches!(&expr.kind, hir::ExprKind::List(elems) if elems.is_empty());
                let is_empty_set =
                    matches!(&expr.kind, hir::ExprKind::Set(elems) if elems.is_empty());

                if !is_empty_list && !is_empty_set {
                    continue;
                }

                // Scan subsequent statements for method calls on this variable
                let elem_ty = self.find_elem_type_from_usage(*target, &stmts[i + 1..], hir_module);

                if elem_ty != Type::Any {
                    let refined = if is_empty_list {
                        Type::List(Box::new(elem_ty))
                    } else {
                        Type::Set(Box::new(elem_ty))
                    };
                    // Store in refined_var_types which persists across function lowerings
                    self.refined_var_types.insert(*target, refined);
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
                hir::StmtKind::For { body, .. }
                | hir::StmtKind::ForUnpack { body, .. }
                | hir::StmtKind::ForUnpackStarred { body, .. }
                | hir::StmtKind::While { body, .. } => {
                    self.refine_empty_containers_in_block(body, hir_module);
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
                hir::StmtKind::Assign { target, .. } if *target == var => {
                    return Type::Any;
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
                    let ty = self.infer_static_expr_type(arg_expr, hir_module);
                    if ty != Type::Any {
                        return Some(ty);
                    }
                }
            }
        }
        None
    }

    /// Lightweight static type inference for expressions (no mutable state needed).
    /// Delegates to infer_deep_expr_type with empty param_types.
    fn infer_static_expr_type(&self, expr: &hir::Expr, hir_module: &hir::Module) -> Type {
        self.infer_deep_expr_type(expr, hir_module, &IndexMap::new())
    }
}

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
            hir::StmtKind::Assign {
                target,
                value,
                type_hint,
            } => {
                let expr = &hir_module.exprs[*value];

                // Determine the variable type
                let var_type = type_hint
                    .clone()
                    .unwrap_or_else(|| self.get_expr_type_static(expr, hir_module, var_types));
                var_types.insert(*target, var_type);

                // Scan the value expression for inline closures
                // This catches cases like: result = list(map(lambda x: ..., ...))
                self.scan_expr_for_closures(expr, hir_module, var_types);

                // Check for decorated function pattern: var = decorator(FuncRef(func))
                // If the decorator returns a closure, mark that closure as a wrapper
                if let hir::ExprKind::Call {
                    func: call_func, ..
                } = &expr.kind
                {
                    // TODO: innermost_func_id (the decorated function) is found but currently
                    // unused — future work should use it to link the decorated function to its
                    // wrapper so call sites can be rewritten directly.
                    if self.find_innermost_func_ref(expr, hir_module).is_some() {
                        let call_func_expr = &hir_module.exprs[*call_func];
                        if let hir::ExprKind::FuncRef(decorator_func_id) = &call_func_expr.kind {
                            if let Some(decorator_def) = hir_module.func_defs.get(decorator_func_id)
                            {
                                if let Some(wrapper_func_id) =
                                    self.find_returned_closure(decorator_def, hir_module)
                                {
                                    // Mark this function as a wrapper
                                    self.insert_wrapper_func_id(wrapper_func_id);
                                }
                            }
                        }
                    }
                }
            }
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
            hir::StmtKind::While { body, .. } => {
                for stmt_id in body {
                    self.scan_stmt_for_closures(*stmt_id, hir_module, var_types);
                }
            }
            hir::StmtKind::For { body, .. }
            | hir::StmtKind::ForUnpack { body, .. }
            | hir::StmtKind::ForUnpackStarred { body, .. } => {
                for stmt_id in body {
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
                            self.get_expr_type_static(capture_expr, hir_module, var_types);
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
                // For map(func, iterable) and filter(func, iterable), register
                // parameter type hints so the lambda gets the correct element type
                // instead of defaulting to Any.
                if matches!(builtin, hir::Builtin::Map | hir::Builtin::Filter) && args.len() >= 2 {
                    let func_arg = &hir_module.exprs[args[0]];
                    let iterable_arg = &hir_module.exprs[args[1]];
                    let iterable_type =
                        self.get_expr_type_static(iterable_arg, hir_module, var_types);
                    let elem_type = extract_iterable_element_type(&iterable_type);
                    if !matches!(elem_type, Type::Any) {
                        let func_info = match &func_arg.kind {
                            hir::ExprKind::FuncRef(func_id) => Some((*func_id, vec![])),
                            hir::ExprKind::Closure { func, captures } => {
                                Some((*func, captures.clone()))
                            }
                            _ => None,
                        };
                        if let Some((func_id, captures)) = func_info {
                            if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                                let num_captures = captures.len();
                                let num_non_capture =
                                    func_def.params.len().saturating_sub(num_captures);
                                // map/filter callback takes exactly 1 element parameter
                                if num_non_capture == 1 {
                                    let mut param_hints = Vec::new();
                                    for cap_id in &captures {
                                        let cap_expr = &hir_module.exprs[*cap_id];
                                        let cap_type = self
                                            .get_expr_type_static(cap_expr, hir_module, var_types);
                                        param_hints.push(cap_type);
                                    }
                                    param_hints.push(elem_type);
                                    self.insert_lambda_param_type_hints(func_id, param_hints);
                                }
                            }
                        }
                    }
                }

                // For reduce(), register parameter type hints for the callback lambda
                // reduce(func, iterable[, initial]) — func takes (acc, elem) both of element type
                if matches!(builtin, hir::Builtin::Reduce) && args.len() >= 2 {
                    let func_arg = &hir_module.exprs[args[0]];
                    let iterable_arg = &hir_module.exprs[args[1]];
                    let iterable_type =
                        self.get_expr_type_static(iterable_arg, hir_module, var_types);
                    let elem_type = extract_iterable_element_type(&iterable_type);
                    if !matches!(elem_type, Type::Any) {
                        // Extract func_id and captures from FuncRef or Closure
                        let func_info = match &func_arg.kind {
                            hir::ExprKind::FuncRef(func_id) => Some((*func_id, vec![])),
                            hir::ExprKind::Closure { func, captures } => {
                                Some((*func, captures.clone()))
                            }
                            _ => None,
                        };
                        if let Some((func_id, captures)) = func_info {
                            if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                                let num_captures = captures.len();
                                let num_non_capture =
                                    func_def.params.len().saturating_sub(num_captures);
                                if num_non_capture == 2 {
                                    let mut param_hints = Vec::new();
                                    for cap_id in &captures {
                                        let cap_expr = &hir_module.exprs[*cap_id];
                                        let cap_type = self
                                            .get_expr_type_static(cap_expr, hir_module, var_types);
                                        param_hints.push(cap_type);
                                    }
                                    param_hints.push(elem_type.clone());
                                    param_hints.push(elem_type);
                                    self.insert_lambda_param_type_hints(func_id, param_hints);
                                }
                            }
                        }
                    }
                }

                // For sorted(iterable, key=lambda), min(iterable, key=lambda), max(iterable, key=lambda):
                // The key function receives element type of the iterable
                if matches!(
                    builtin,
                    hir::Builtin::Sorted | hir::Builtin::Min | hir::Builtin::Max
                ) && !args.is_empty()
                {
                    // Find key= kwarg
                    let key_func = kwargs.iter().find_map(|kw| {
                        let kw_name = self.interner.resolve(kw.name);
                        if kw_name == "key" {
                            Some(&hir_module.exprs[kw.value])
                        } else {
                            None
                        }
                    });
                    if let Some(key_expr) = key_func {
                        let iterable_arg = &hir_module.exprs[args[0]];
                        let iterable_type =
                            self.get_expr_type_static(iterable_arg, hir_module, var_types);
                        let elem_type = extract_iterable_element_type(&iterable_type);
                        if !matches!(elem_type, Type::Any) {
                            let func_info = match &key_expr.kind {
                                hir::ExprKind::FuncRef(func_id) => Some((*func_id, vec![])),
                                hir::ExprKind::Closure { func, captures } => {
                                    Some((*func, captures.clone()))
                                }
                                _ => None,
                            };
                            if let Some((func_id, captures)) = func_info {
                                if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                                    let num_captures = captures.len();
                                    let num_non_capture =
                                        func_def.params.len().saturating_sub(num_captures);
                                    if num_non_capture == 1 {
                                        let mut param_hints = Vec::new();
                                        for cap_id in &captures {
                                            let cap_expr = &hir_module.exprs[*cap_id];
                                            let cap_type = self.get_expr_type_static(
                                                cap_expr, hir_module, var_types,
                                            );
                                            param_hints.push(cap_type);
                                        }
                                        param_hints.push(elem_type);
                                        self.insert_lambda_param_type_hints(func_id, param_hints);
                                    }
                                }
                            }
                        }
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

    /// Get expression type using only static information (for pre-processing).
    /// Delegates to infer_deep_expr_type with var_types as param_types override.
    fn get_expr_type_static(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        var_types: &IndexMap<VarId, Type>,
    ) -> Type {
        self.infer_deep_expr_type(expr, hir_module, var_types)
    }

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

        // Lambda body should have a single return statement
        if let Some(stmt_id) = func.body.first() {
            let stmt = &hir_module.stmts[*stmt_id];
            if let hir::StmtKind::Return(Some(expr_id)) = &stmt.kind {
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

        // Scan all statements for a return statement
        // (functions may have multiple statements before the return)
        for stmt_id in &func.body {
            let stmt = &hir_module.stmts[*stmt_id];
            if let hir::StmtKind::Return(Some(expr_id)) = &stmt.kind {
                let expr = &hir_module.exprs[*expr_id];
                return self.infer_expr_return_type_with_params(expr, hir_module, &param_type_map);
            }
        }
        Type::None
    }

    /// Infer the type of an expression for return type inference, using known param types.
    /// Delegates to infer_deep_expr_type for comprehensive inference.
    fn infer_expr_return_type_with_params(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        self.infer_deep_expr_type(expr, hir_module, param_types)
    }
}
