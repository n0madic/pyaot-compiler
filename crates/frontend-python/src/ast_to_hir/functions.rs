use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_types::Type;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_function_def(&mut self, func_def: py::StmtFunctionDef) -> Result<()> {
        let func_span = Self::span_from(&func_def);
        let func_id = self.alloc_func_id();
        let func_name = self.interner.intern(&func_def.name);

        // Register function in func_map early to allow recursive calls
        self.func_map.insert(func_name, func_id);

        // Save outer var_map and create new scope
        let outer_var_map = std::mem::take(&mut self.var_map);
        let outer_global_vars = std::mem::take(&mut self.global_vars);
        let outer_nonlocal_vars = std::mem::take(&mut self.nonlocal_vars);
        let outer_cell_vars = std::mem::take(&mut self.current_cell_vars);
        let outer_initialized_vars = std::mem::take(&mut self.initialized_vars);
        let outer_is_generator = self.current_func_is_generator;
        self.current_func_is_generator = false;

        // Push outer scope onto stack for nonlocal lookup
        self.scope_stack.push(outer_var_map.clone());

        // Calculate default values mapping
        // defaults apply to the last N parameters
        let num_params = func_def.args.args.len();
        let defaults: Vec<_> = func_def.args.defaults().collect();
        let num_defaults = defaults.len();
        let first_default_idx = num_params.saturating_sub(num_defaults);

        // Convert parameters
        let mut params = Vec::new();
        for (i, arg) in func_def.args.args.iter().enumerate() {
            let param_name = self.interner.intern(&arg.def.arg);
            let param_id = self.alloc_var_id();
            self.var_map.insert(param_name, param_id);
            // Mark parameter as initialized (parameters are always initialized when function is called)
            self.initialized_vars.insert(param_name);

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
                span: func_span,
            });
        }

        // Process *args parameter (vararg)
        if let Some(vararg_param) = &func_def.args.vararg {
            let vararg_name = self.interner.intern(&vararg_param.arg);
            let vararg_id = self.alloc_var_id();
            self.var_map.insert(vararg_name, vararg_id);
            self.initialized_vars.insert(vararg_name);

            // Type annotation: *args: int → tuple[int, ...]
            let vararg_type = if let Some(annotation) = &vararg_param.annotation {
                let element_type = self.convert_type_annotation(annotation)?;
                Some(Type::Tuple(vec![element_type]))
            } else {
                Some(Type::Tuple(vec![Type::Any])) // Default: tuple[Any, ...]
            };

            params.push(Param {
                name: vararg_name,
                var: vararg_id,
                ty: vararg_type,
                default: None,
                kind: ParamKind::VarPositional,
                span: func_span,
            });
        }

        // Process keyword-only parameters (kwonlyargs - parameters after *args)
        for kwonly_arg in func_def.args.kwonlyargs.iter() {
            let param_name = self.interner.intern(&kwonly_arg.def.arg);
            let param_id = self.alloc_var_id();
            self.var_map.insert(param_name, param_id);
            self.initialized_vars.insert(param_name);

            let param_type = if let Some(annotation) = &kwonly_arg.def.annotation {
                Some(self.convert_type_annotation(annotation)?)
            } else {
                None
            };

            // Extract default value from AST (if present)
            let default = if let Some(default_expr) = &kwonly_arg.default {
                Some(self.convert_expr((**default_expr).clone())?)
            } else {
                None
            };

            params.push(Param {
                name: param_name,
                var: param_id,
                ty: param_type,
                default,
                kind: ParamKind::KeywordOnly,
                span: func_span,
            });
        }

        // Process **kwargs parameter (kwarg)
        if let Some(kwarg_param) = &func_def.args.kwarg {
            let kwarg_name = self.interner.intern(&kwarg_param.arg);
            let kwarg_id = self.alloc_var_id();
            self.var_map.insert(kwarg_name, kwarg_id);
            self.initialized_vars.insert(kwarg_name);

            // Type annotation: **kwargs: int → dict[str, int]
            let kwarg_type = if let Some(annotation) = &kwarg_param.annotation {
                let value_type = self.convert_type_annotation(annotation)?;
                Some(Type::Dict(Box::new(Type::Str), Box::new(value_type)))
            } else {
                Some(Type::Dict(Box::new(Type::Str), Box::new(Type::Any)))
            };

            params.push(Param {
                name: kwarg_name,
                var: kwarg_id,
                ty: kwarg_type,
                default: None,
                kind: ParamKind::VarKeyword,
                span: func_span,
            });
        }

        // Convert return type (None means no annotation, not "returns None")
        // In Python, unannotated functions can return any type, so we represent this
        // as Option::None to distinguish from explicit "-> None" annotation.
        let return_type = if let Some(ret_ann) = &func_def.returns {
            Some(self.convert_type_annotation(ret_ann)?)
        } else {
            None // No annotation = unknown type (Any), not implicitly None
        };

        // Convert function body
        let mut body_stmts = Vec::new();
        for stmt in func_def.body {
            let stmt_id = self.convert_stmt(stmt)?;
            // Inject any pending statements from comprehensions before this statement
            let pending = self.take_pending_stmts();
            body_stmts.extend(pending);
            body_stmts.push(stmt_id);
        }

        // Take the cell_vars collected during function body processing
        let func_cell_vars = std::mem::take(&mut self.current_cell_vars);
        let func_is_generator = self.current_func_is_generator;

        let function = Function {
            id: func_id,
            name: func_name,
            params,
            return_type,
            body: body_stmts,
            span: func_span,
            cell_vars: func_cell_vars,
            nonlocal_vars: std::collections::HashSet::new(), // Top-level functions don't have nonlocal
            is_generator: func_is_generator,
            method_kind: MethodKind::default(), // Top-level functions are not methods
            is_abstract: false,                 // Top-level functions cannot be abstract
        };

        self.module.functions.push(func_id);
        self.module.func_defs.insert(func_id, function);

        // Handle decorators: @decorator def foo(): ... becomes foo = decorator(foo)
        // Decorators are applied bottom-up
        if !func_def.decorator_list.is_empty() {
            // Create variable for decorated function
            let func_var_id = self.alloc_var_id();
            self.module_var_map.insert(func_name, func_var_id);

            // Start with FuncRef to the original function
            let mut current_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::FuncRef(func_id),
                ty: None,
                span: func_span,
            });

            // Apply decorators bottom-up (last decorator applied first)
            for decorator in func_def.decorator_list.iter().rev() {
                current_expr = self.apply_decorator(decorator, current_expr, func_span)?;
            }

            // Create assignment: func_name = decorated_result
            let assign_stmt = self.module.stmts.alloc(Stmt {
                kind: StmtKind::Assign {
                    target: func_var_id,
                    value: current_expr,
                    type_hint: None,
                },
                span: func_span,
            });
            self.module.module_init_stmts.push(assign_stmt);

            // Remove from func_map so calls go through var_map
            self.func_map.remove(&func_name);
        }

        // Pop scope from stack
        self.scope_stack.pop();

        // Restore outer scope
        self.global_vars = outer_global_vars;
        self.nonlocal_vars = outer_nonlocal_vars;
        self.var_map = outer_var_map;
        self.current_cell_vars = outer_cell_vars;
        self.initialized_vars = outer_initialized_vars;
        self.current_func_is_generator = outer_is_generator;

        Ok(())
    }

    /// Apply a decorator to a target expression
    /// Returns a Call expression: decorator(target)
    pub(crate) fn apply_decorator(
        &mut self,
        decorator: &py::Expr,
        target: ExprId,
        span: Span,
    ) -> Result<ExprId> {
        match decorator {
            py::Expr::Name(name) => {
                // @decorator - resolve and call
                let dec_expr = self.resolve_decorator_name(&name.id, span)?;
                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::Call {
                        func: dec_expr,
                        args: vec![CallArg::Regular(target)],
                        kwargs: vec![],
                        kwargs_unpack: None,
                    },
                    ty: None,
                    span,
                }))
            }
            py::Expr::Call(call) => {
                // @decorator(args) - call factory, then apply result
                let factory_expr = self.convert_expr((*call.func).clone())?;

                let args = self.convert_call_args(call.args.clone())?;

                let (kwargs, kwargs_unpack) = self.convert_keywords(call.keywords.clone())?;

                let factory_call = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Call {
                        func: factory_expr,
                        args,
                        kwargs,
                        kwargs_unpack,
                    },
                    ty: None,
                    span,
                });

                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::Call {
                        func: factory_call,
                        args: vec![CallArg::Regular(target)],
                        kwargs: vec![],
                        kwargs_unpack: None,
                    },
                    ty: None,
                    span,
                }))
            }
            py::Expr::Attribute(_) => {
                // @module.decorator
                let dec_expr = self.convert_expr(decorator.clone())?;
                Ok(self.module.exprs.alloc(Expr {
                    kind: ExprKind::Call {
                        func: dec_expr,
                        args: vec![CallArg::Regular(target)],
                        kwargs: vec![],
                        kwargs_unpack: None,
                    },
                    ty: None,
                    span,
                }))
            }
            _ => Err(CompilerError::parse_error(
                "Unsupported decorator syntax",
                span,
            )),
        }
    }

    /// Resolve a decorator name to an expression
    fn resolve_decorator_name(&mut self, name: &str, span: Span) -> Result<ExprId> {
        let interned = self.interner.intern(name);

        // Check if it's a function reference
        if let Some(&func_id) = self.func_map.get(&interned) {
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::FuncRef(func_id),
                ty: None,
                span,
            }));
        }

        // Check local variables
        if let Some(&var_id) = self.var_map.get(&interned) {
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(var_id),
                ty: None,
                span,
            }));
        }

        // Check module-level variables
        if let Some(&var_id) = self.module_var_map.get(&interned) {
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(var_id),
                ty: None,
                span,
            }));
        }

        Err(CompilerError::name_error(
            format!("Decorator '{}' is not defined", name),
            span,
        ))
    }
}
