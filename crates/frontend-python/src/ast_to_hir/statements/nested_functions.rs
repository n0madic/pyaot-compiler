//! Nested function definitions with closure capture

use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{cfg_build::CfgBuilder, *};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_nested_function_def(
        &mut self,
        func_def: py::StmtFunctionDef,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Nested function definition - treat like a closure
        // Similar to lambda, but with statement body and explicit name

        // 1. Collect parameter names for free variable detection
        let local_params: std::collections::HashSet<String> = func_def
            .args
            .args
            .iter()
            .map(|arg| arg.def.arg.to_string())
            .collect();

        // 1.5 Pre-scan for nonlocal declarations in the nested function
        // Variables declared nonlocal need to be wrapped in cells in the outer scope
        let nonlocal_in_nested = self.scan_for_nonlocal_declarations(&func_def.body);

        // 1.6 Check that all nonlocal variables are initialized before the nested function
        // This prevents UnboundLocalError-equivalent issues at runtime
        // A variable is considered "available" if:
        // - It was initialized in this scope, OR
        // - It is declared as nonlocal in this scope (chained nonlocal)
        for name in &nonlocal_in_nested {
            let is_initialized = self.scope.initialized_vars.contains(name);
            let is_nonlocal_here = self.scope.nonlocal_vars.contains(name);
            if !is_initialized && !is_nonlocal_here {
                let name_str = self.interner.resolve(*name);
                return Err(CompilerError::parse_error(
                    format!(
                        "variable '{}' referenced in nonlocal declaration is not initialized before nested function definition",
                        name_str
                    ),
                    stmt_span,
                ));
            }
        }

        // Mark these variables as cell_vars in the current (outer) function
        for name in &nonlocal_in_nested {
            if let Some(&var_id) = self.symbols.var_map.get(name) {
                self.scope.current_cell_vars.insert(var_id);
            }
        }

        // 2. Find captured variables BEFORE changing scope
        let mut all_free_vars = self.find_free_variables_in_body(&func_def.body, &local_params);

        // Also add nonlocal variables to free vars (they need to be captured as cells)
        // The free variable detection doesn't find them because assignments add them to local scope
        for name in &nonlocal_in_nested {
            if !all_free_vars.contains(name) {
                all_free_vars.push(*name);
            }
        }

        // Filter out global variables - they should use global storage, not captures
        let (global_propagation, captured_vars): (Vec<_>, Vec<_>) =
            all_free_vars.into_iter().partition(|name| {
                // Check if this variable is in the module's globals set
                if let Some(&var_id) = self.symbols.var_map.get(name) {
                    self.module.globals.contains(&var_id)
                } else {
                    false
                }
            });

        // 3. Allocate func_id and register early (for recursive calls)
        let func_id = self.ids.alloc_func();
        let nested_func_name = format!("__nested_{}_{}", func_def.name, self.ids.next_lambda_id);
        self.ids.next_lambda_id += 1;
        let internal_func_name = self.interner.intern(&nested_func_name);

        // Register the nested function under its original name in the outer scope's func_map
        let user_func_name = self.interner.intern(&func_def.name);
        self.symbols.func_map.insert(user_func_name, func_id);

        // 4. Save outer scope (including cell_vars tracking)
        let outer_var_map = std::mem::take(&mut self.symbols.var_map);
        let outer_global_vars = std::mem::take(&mut self.scope.global_vars);
        let outer_cell_vars = std::mem::take(&mut self.scope.current_cell_vars);
        let outer_initialized_vars = std::mem::take(&mut self.scope.initialized_vars);
        let outer_is_generator = self.scope.current_func_is_generator;
        self.scope.current_func_is_generator = false;

        // 4.25 Push outer scope onto scope_stack for nonlocal lookup
        self.scope.scope_stack.push(outer_var_map.clone());

        // 4.5 Auto-propagate global variables to nested scope
        // These variables use global storage instead of being captured
        for name in &global_propagation {
            if let Some(&var_id) = outer_var_map.get(name) {
                // Map the variable to the same module-level VarId
                self.symbols.var_map.insert(*name, var_id);
                // Mark as global in this scope
                self.scope.global_vars.insert(*name);
            }
        }

        // 5. Create parameters: captured vars first, then function params
        let mut params = Vec::new();

        // Add captured variables as implicit leading parameters
        for captured_name in &captured_vars {
            let capture_param_name = self.interner.intern(&format!(
                "__capture_{}",
                self.interner.resolve(*captured_name)
            ));
            let param_id = self.ids.alloc_var();
            // Map original name to capture param so body references work
            self.symbols.var_map.insert(*captured_name, param_id);

            params.push(Param {
                name: capture_param_name,
                var: param_id,
                ty: None, // Type inferred during lowering
                default: None,
                kind: ParamKind::Regular,
                span: stmt_span,
            });
        }

        // Add regular function parameters
        // Calculate default values mapping
        let num_params = func_def.args.args.len();
        let defaults: Vec<_> = func_def.args.defaults().collect();
        let num_defaults = defaults.len();
        let first_default_idx = num_params.saturating_sub(num_defaults);

        for (i, arg) in func_def.args.args.iter().enumerate() {
            let param_name = self.interner.intern(&arg.def.arg);
            let param_id = self.ids.alloc_var();
            self.symbols.var_map.insert(param_name, param_id);

            let param_type = if let Some(annotation) = &arg.def.annotation {
                Some(self.convert_type_annotation(annotation)?)
            } else {
                None
            };

            // Get default value if this parameter has one
            let default = if i >= first_default_idx {
                let default_idx = i - first_default_idx;
                Some(self.convert_expr((*defaults[default_idx]).clone())?)
            } else {
                None
            };

            params.push(Param {
                name: param_name,
                var: param_id,
                ty: param_type,
                default,
                kind: ParamKind::Regular,
                span: stmt_span,
            });
        }

        // Process *args, keyword-only, and **kwargs parameters
        params.extend(self.convert_extra_params(&func_def.args, stmt_span)?);

        // 6. Convert return type (None means no annotation, not "returns None")
        // In Python, unannotated functions can return any type, so we represent this
        // as Option::None to distinguish from explicit "-> None" annotation.
        let return_type = if let Some(ret_ann) = &func_def.returns {
            Some(self.convert_type_annotation(ret_ann)?)
        } else {
            None // No annotation = unknown type (Any), not implicitly None
        };

        // 7. Convert function body
        let mut body_stmts = Vec::new();
        for stmt in func_def.body {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            body_stmts.extend(pending);
            body_stmts.push(stmt_id);
        }

        // 8. Create and register function
        // Collect nonlocal_vars for the nested function (VarIds of captured params
        // that correspond to nonlocal declarations)
        let mut nested_nonlocal_vars = std::collections::HashSet::new();
        for name in &nonlocal_in_nested {
            if let Some(&param_var_id) = self.symbols.var_map.get(name) {
                nested_nonlocal_vars.insert(param_var_id);
            }
        }

        // Take the cell_vars collected during nested function body processing
        let nested_cell_vars = std::mem::take(&mut self.scope.current_cell_vars);
        let nested_is_generator = self.scope.current_func_is_generator;

        let mut cfg = CfgBuilder::new();
        let entry_block = cfg.new_block();
        cfg.enter(entry_block);
        cfg.lower_stmts(&body_stmts, &mut self.module);
        cfg.terminate_if_open(HirTerminator::Return(None));
        let (blocks, entry_block, try_scopes) = cfg.finish(entry_block);
        let function = Function {
            id: func_id,
            name: internal_func_name,
            params,
            return_type,
            body: body_stmts,
            span: stmt_span,
            cell_vars: nested_cell_vars,
            nonlocal_vars: nested_nonlocal_vars,
            is_generator: nested_is_generator,
            method_kind: MethodKind::default(), // Nested functions are not class methods
            is_abstract: false,                 // Nested functions cannot be abstract
            blocks,
            entry_block,
            try_scopes,
        };
        self.module.functions.push(func_id);
        self.module.func_defs.insert(func_id, function);

        // 9. Pop scope_stack and restore outer scope
        self.scope.scope_stack.pop();
        self.scope.global_vars = outer_global_vars;
        self.symbols.var_map = outer_var_map;
        self.scope.current_cell_vars = outer_cell_vars;
        self.scope.initialized_vars = outer_initialized_vars;
        self.scope.current_func_is_generator = outer_is_generator;

        // Also mark nonlocal variables as cell_vars in the outer scope
        for name in &nonlocal_in_nested {
            if let Some(&var_id) = self.symbols.var_map.get(name) {
                self.scope.current_cell_vars.insert(var_id);
            }
        }

        // 10. For closures with captures, remove from func_map so external calls
        // go through var_map (which holds the closure with captures).
        // Keep in func_map only for non-capturing functions.
        if !captured_vars.is_empty() {
            self.symbols.func_map.remove(&user_func_name);
        }

        // 11. Create variable for the nested function in outer scope
        let nested_var_id = self.ids.alloc_var();
        self.symbols.var_map.insert(user_func_name, nested_var_id);

        // 12. Create capture expressions (references to outer variables)
        let captures: Vec<ExprId> = captured_vars
            .iter()
            .map(|name| {
                let var_id = self
                    .symbols
                    .var_map
                    .get(name)
                    .expect("internal error: captured variable not in var_map");
                self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(*var_id),
                    ty: None,
                    span: stmt_span,
                })
            })
            .collect();

        // 13. Create Closure or FuncRef expression
        let expr_kind = if captures.is_empty() {
            ExprKind::FuncRef(func_id)
        } else {
            ExprKind::Closure {
                func: func_id,
                captures,
            }
        };

        let closure_expr = self.module.exprs.alloc(Expr {
            kind: expr_kind,
            ty: None,
            span: stmt_span,
        });

        // 14. Return an assignment statement: nested_func_name = closure
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Bind {
                target: BindingTarget::Var(nested_var_id),
                value: closure_expr,
                type_hint: None,
            },
            span: stmt_span,
        }))
    }
}
