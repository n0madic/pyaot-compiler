use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_utils::InternedString;
use rustpython_parser::ast as py;
use std::collections::HashSet;

impl AstToHir {
    /// Convert a lambda expression into a closure.
    /// Lambdas are desugared into regular functions with captured variables as implicit leading parameters.
    pub(crate) fn convert_lambda(&mut self, lambda: py::ExprLambda) -> Result<ExprId> {
        let lambda_span = Self::span_from(&lambda);
        // 1. Collect parameter names for free variable detection
        let local_params: HashSet<String> = lambda
            .args
            .args
            .iter()
            .map(|arg| arg.def.arg.to_string())
            .collect();

        // 2. Find captured variables BEFORE changing scope
        let all_free_vars = self.find_free_variables(&lambda.body, &local_params);

        // Filter out global variables - they should use global storage, not captures
        let (global_propagation, captured_vars): (Vec<_>, Vec<_>) =
            all_free_vars.into_iter().partition(|name| {
                // Check if this variable is in the module's globals set
                if let Some(&var_id) = self.var_map.get(name) {
                    self.module.globals.contains(&var_id)
                } else {
                    false
                }
            });

        // 3. Convert default values in the OUTER scope (Python semantics: defaults
        // are evaluated at definition time, not call time)
        let defaults: Vec<_> = lambda.args.defaults().collect();
        let num_defaults = defaults.len();
        let num_lambda_params = lambda.args.args.len();
        let first_default_idx = num_lambda_params.saturating_sub(num_defaults);
        let mut converted_defaults: Vec<Option<ExprId>> = Vec::new();
        for i in 0..num_lambda_params {
            if i >= first_default_idx {
                let default_idx = i - first_default_idx;
                converted_defaults.push(Some(self.convert_expr((*defaults[default_idx]).clone())?));
            } else {
                converted_defaults.push(None);
            }
        }

        // 4. Generate unique function name
        let lambda_name = format!("__lambda_{}", self.next_lambda_id);
        self.next_lambda_id += 1;
        let func_id = self.alloc_func_id();
        let func_name = self.interner.intern(&lambda_name);

        // 5. Save outer scope
        let outer_var_map = std::mem::take(&mut self.var_map);
        let outer_global_vars = std::mem::take(&mut self.global_vars);

        // 5.5 Auto-propagate global variables to nested scope
        // These variables use global storage instead of being captured
        for name in &global_propagation {
            if let Some(&var_id) = outer_var_map.get(name) {
                // Map the variable to the same module-level VarId
                self.var_map.insert(*name, var_id);
                // Mark as global in this scope
                self.global_vars.insert(*name);
            }
        }

        // 6. Create parameters: captured vars first, then lambda params
        let mut params = Vec::new();

        // Add captured variables as implicit leading parameters
        for captured_name in &captured_vars {
            let capture_param_name = self.interner.intern(&format!(
                "__capture_{}",
                self.interner.resolve(*captured_name)
            ));
            let param_id = self.alloc_var_id();
            // Map original name to capture param so body references work
            self.var_map.insert(*captured_name, param_id);

            params.push(Param {
                name: capture_param_name,
                var: param_id,
                ty: None, // Type inferred during lowering
                default: None,
                kind: ParamKind::Regular,
                span: lambda_span,
            });
        }

        // Add regular lambda parameters (with defaults from outer scope)
        for (i, arg) in lambda.args.args.iter().enumerate() {
            let param_name = self.interner.intern(&arg.def.arg);
            let param_id = self.alloc_var_id();
            self.var_map.insert(param_name, param_id);

            params.push(Param {
                name: param_name,
                var: param_id,
                ty: None, // Type inferred during lowering
                default: converted_defaults[i],
                kind: ParamKind::Regular,
                span: lambda_span,
            });
        }

        // 7. Convert lambda body expression
        let body_expr = self.convert_expr(*lambda.body)?;

        // 8. Create return statement
        let return_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Return(Some(body_expr)),
            span: lambda_span,
        });

        // 9. Create and register function
        let function = Function {
            id: func_id,
            name: func_name,
            params,
            return_type: None, // Type inferred during lowering
            body: vec![return_stmt],
            span: lambda_span,
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: false,                // Lambdas cannot be generators
            method_kind: MethodKind::default(), // Lambdas are not methods
            is_abstract: false,                 // Lambdas cannot be abstract
        };
        self.module.functions.push(func_id);
        self.module.func_defs.insert(func_id, function);

        // 10. Restore scope
        self.global_vars = outer_global_vars;
        self.var_map = outer_var_map;

        // 11. Create capture expressions (references to outer variables)
        let captures: Vec<ExprId> = captured_vars
            .iter()
            .map(|name| {
                let var_id = self
                    .var_map
                    .get(name)
                    .expect("internal error: captured variable not in var_map");
                self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(*var_id),
                    ty: None,
                    span: lambda_span,
                })
            })
            .collect();

        // 12. Return Closure or FuncRef expression
        let expr_kind = if captures.is_empty() {
            ExprKind::FuncRef(func_id)
        } else {
            ExprKind::Closure {
                func: func_id,
                captures,
            }
        };

        let expr_id = self.module.exprs.alloc(Expr {
            kind: expr_kind,
            ty: None,
            span: lambda_span,
        });
        Ok(expr_id)
    }

    /// Find variables referenced in an expression that are not in the given local scope
    fn find_free_variables(
        &self,
        expr: &py::Expr,
        local_params: &HashSet<String>,
    ) -> Vec<InternedString> {
        let mut free_vars = Vec::new();
        self.collect_free_variables(expr, local_params, &mut free_vars);
        free_vars
    }

    /// Find free variables in a list of statements (for nested functions)
    pub(crate) fn find_free_variables_in_body(
        &self,
        stmts: &[py::Stmt],
        local_names: &HashSet<String>,
    ) -> Vec<InternedString> {
        let mut free_vars = Vec::new();
        let mut local_scope = local_names.clone();
        self.collect_free_variables_in_stmts(stmts, &mut local_scope, &mut free_vars);
        free_vars
    }

    /// Recursively collect free variables from a list of statements
    fn collect_free_variables_in_stmts(
        &self,
        stmts: &[py::Stmt],
        local_scope: &mut HashSet<String>,
        free_vars: &mut Vec<InternedString>,
    ) {
        for stmt in stmts {
            self.collect_free_variables_in_stmt(stmt, local_scope, free_vars);
        }
    }

    /// Collect free variables from a single statement
    fn collect_free_variables_in_stmt(
        &self,
        stmt: &py::Stmt,
        local_scope: &mut HashSet<String>,
        free_vars: &mut Vec<InternedString>,
    ) {
        match stmt {
            py::Stmt::Expr(expr_stmt) => {
                self.collect_free_variables(&expr_stmt.value, local_scope, free_vars);
            }
            py::Stmt::Assign(assign) => {
                // First collect from value (before target is defined)
                self.collect_free_variables(&assign.value, local_scope, free_vars);
                // Then add target to local scope
                for target in &assign.targets {
                    self.add_target_to_scope(target, local_scope);
                }
            }
            py::Stmt::AnnAssign(ann_assign) => {
                // Collect from value if present
                if let Some(ref value) = ann_assign.value {
                    self.collect_free_variables(value, local_scope, free_vars);
                }
                // Add target to local scope
                self.add_target_to_scope(&ann_assign.target, local_scope);
            }
            py::Stmt::AugAssign(aug_assign) => {
                // Augmented assignment reads then writes the target
                self.collect_free_variables(&aug_assign.target, local_scope, free_vars);
                self.collect_free_variables(&aug_assign.value, local_scope, free_vars);
            }
            py::Stmt::Return(ret) => {
                if let Some(ref value) = ret.value {
                    self.collect_free_variables(value, local_scope, free_vars);
                }
            }
            py::Stmt::If(if_stmt) => {
                self.collect_free_variables(&if_stmt.test, local_scope, free_vars);
                self.collect_free_variables_in_stmts(&if_stmt.body, local_scope, free_vars);
                self.collect_free_variables_in_stmts(&if_stmt.orelse, local_scope, free_vars);
            }
            py::Stmt::While(while_stmt) => {
                self.collect_free_variables(&while_stmt.test, local_scope, free_vars);
                self.collect_free_variables_in_stmts(&while_stmt.body, local_scope, free_vars);
            }
            py::Stmt::For(for_stmt) => {
                // Collect from iterator first
                self.collect_free_variables(&for_stmt.iter, local_scope, free_vars);
                // Add loop variable to local scope
                self.add_target_to_scope(&for_stmt.target, local_scope);
                // Collect from body
                self.collect_free_variables_in_stmts(&for_stmt.body, local_scope, free_vars);
            }
            py::Stmt::Try(try_stmt) => {
                self.collect_free_variables_in_stmts(&try_stmt.body, local_scope, free_vars);
                for handler in &try_stmt.handlers {
                    let py::ExceptHandler::ExceptHandler(h) = handler;
                    // Add exception variable to local scope if present
                    if let Some(ref name) = h.name {
                        local_scope.insert(name.to_string());
                    }
                    self.collect_free_variables_in_stmts(&h.body, local_scope, free_vars);
                }
                self.collect_free_variables_in_stmts(&try_stmt.finalbody, local_scope, free_vars);
            }
            py::Stmt::FunctionDef(func_def) => {
                // The nested function name becomes a local
                local_scope.insert(func_def.name.to_string());

                // For deeply nested functions, we need to find free variables that the
                // nested function uses but doesn't define. These need to be captured by
                // the current function so they can be passed through.
                let mut nested_local_scope = local_scope.clone();
                // Add the nested function's parameters to its local scope
                for arg in &func_def.args.args {
                    nested_local_scope.insert(arg.def.arg.to_string());
                }
                // Recursively find free variables in the nested function's body
                self.collect_free_variables_in_stmts(
                    &func_def.body,
                    &mut nested_local_scope,
                    free_vars,
                );
            }
            py::Stmt::Assert(assert_stmt) => {
                self.collect_free_variables(&assert_stmt.test, local_scope, free_vars);
                if let Some(ref msg) = assert_stmt.msg {
                    self.collect_free_variables(msg, local_scope, free_vars);
                }
            }
            py::Stmt::Raise(raise_stmt) => {
                if let Some(ref exc) = raise_stmt.exc {
                    self.collect_free_variables(exc, local_scope, free_vars);
                }
            }
            // Pass, Break, Continue don't reference variables
            py::Stmt::Pass(_) | py::Stmt::Break(_) | py::Stmt::Continue(_) => {}
            // Global variables should NOT be captured - they're module-level
            py::Stmt::Global(global_stmt) => {
                for name in &global_stmt.names {
                    local_scope.insert(name.to_string());
                }
            }
            // Other statements - skip for now
            _ => {}
        }
    }

    /// Add a target expression to the local scope (handles names, tuples, etc.)
    fn add_target_to_scope(&self, target: &py::Expr, local_scope: &mut HashSet<String>) {
        match target {
            py::Expr::Name(name) => {
                local_scope.insert(name.id.to_string());
            }
            py::Expr::Tuple(tuple) => {
                for elem in &tuple.elts {
                    self.add_target_to_scope(elem, local_scope);
                }
            }
            py::Expr::List(list) => {
                for elem in &list.elts {
                    self.add_target_to_scope(elem, local_scope);
                }
            }
            // Attribute and subscript don't add new local names
            _ => {}
        }
    }

    /// Recursively collect free variables from an expression
    fn collect_free_variables(
        &self,
        expr: &py::Expr,
        local_params: &HashSet<String>,
        free_vars: &mut Vec<InternedString>,
    ) {
        match expr {
            py::Expr::Name(name) => {
                // If not a local param and exists in outer scope, it's a capture
                if !local_params.contains(name.id.as_str()) {
                    if let Some(interned) = self.interner.lookup(&name.id) {
                        if self.var_map.contains_key(&interned) && !free_vars.contains(&interned) {
                            free_vars.push(interned);
                        }
                    }
                }
            }
            py::Expr::BinOp(binop) => {
                self.collect_free_variables(&binop.left, local_params, free_vars);
                self.collect_free_variables(&binop.right, local_params, free_vars);
            }
            py::Expr::UnaryOp(unop) => {
                self.collect_free_variables(&unop.operand, local_params, free_vars);
            }
            py::Expr::Compare(cmp) => {
                self.collect_free_variables(&cmp.left, local_params, free_vars);
                for comparator in &cmp.comparators {
                    self.collect_free_variables(comparator, local_params, free_vars);
                }
            }
            py::Expr::BoolOp(boolop) => {
                for value in &boolop.values {
                    self.collect_free_variables(value, local_params, free_vars);
                }
            }
            py::Expr::Call(call) => {
                self.collect_free_variables(&call.func, local_params, free_vars);
                for arg in &call.args {
                    self.collect_free_variables(arg, local_params, free_vars);
                }
            }
            py::Expr::IfExp(ifexp) => {
                self.collect_free_variables(&ifexp.test, local_params, free_vars);
                self.collect_free_variables(&ifexp.body, local_params, free_vars);
                self.collect_free_variables(&ifexp.orelse, local_params, free_vars);
            }
            py::Expr::Subscript(sub) => {
                self.collect_free_variables(&sub.value, local_params, free_vars);
                self.collect_free_variables(&sub.slice, local_params, free_vars);
            }
            py::Expr::Attribute(attr) => {
                self.collect_free_variables(&attr.value, local_params, free_vars);
            }
            py::Expr::List(list) => {
                for elem in &list.elts {
                    self.collect_free_variables(elem, local_params, free_vars);
                }
            }
            py::Expr::Tuple(tuple) => {
                for elem in &tuple.elts {
                    self.collect_free_variables(elem, local_params, free_vars);
                }
            }
            py::Expr::Dict(dict) => {
                for key in dict.keys.iter().flatten() {
                    self.collect_free_variables(key, local_params, free_vars);
                }
                for value in &dict.values {
                    self.collect_free_variables(value, local_params, free_vars);
                }
            }
            py::Expr::Set(set) => {
                for elem in &set.elts {
                    self.collect_free_variables(elem, local_params, free_vars);
                }
            }
            // Constants don't reference variables
            py::Expr::Constant(_) => {}
            // Other expressions - recursively handle if needed
            _ => {}
        }
    }
}
