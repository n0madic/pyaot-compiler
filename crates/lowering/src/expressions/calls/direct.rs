//! Direct function call lowering: lower_call dispatcher

use pyaot_core_defs::TypeTagKind;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

use super::ExpandedArg;

impl<'a> Lowering<'a> {
    /// Lower a function call expression.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::expressions) fn lower_call(
        &mut self,
        func: hir::ExprId,
        args: &[hir::CallArg],
        kwargs: &[hir::KeywordArg],
        kwargs_unpack: &Option<hir::ExprId>,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Expand starred arguments (*args)
        let mut expanded_args: Vec<ExpandedArg> = Vec::new();
        for arg in args {
            match arg {
                hir::CallArg::Regular(expr_id) => {
                    expanded_args.push(ExpandedArg::Regular(*expr_id));
                }
                hir::CallArg::Starred(expr_id) => {
                    // Check if this is a compile-time literal (list or tuple)
                    let arg_expr = &hir_module.exprs[*expr_id];
                    match &arg_expr.kind {
                        hir::ExprKind::List(elements) => {
                            // Compile-time unpack: *[1, 2, 3] → 1, 2, 3
                            expanded_args.extend(elements.iter().map(|e| ExpandedArg::Regular(*e)));
                        }
                        hir::ExprKind::Tuple(elements) => {
                            // Compile-time unpack: *(1, 2, 3) → 1, 2, 3
                            expanded_args.extend(elements.iter().map(|e| ExpandedArg::Regular(*e)));
                        }
                        _ => {
                            // Runtime unpacking: check if it's a tuple or list variable
                            let arg_type = self.get_expr_type(arg_expr, hir_module);
                            match &arg_type {
                                Type::Tuple(_) => {
                                    // Mark for runtime tuple unpacking
                                    expanded_args.push(ExpandedArg::RuntimeUnpackTuple(*expr_id));
                                }
                                Type::List(_) => {
                                    // Mark for runtime list unpacking
                                    expanded_args.push(ExpandedArg::RuntimeUnpackList(*expr_id));
                                }
                                _ => {
                                    // Not a tuple or list - pass as is (will cause type error)
                                    expanded_args.push(ExpandedArg::Regular(*expr_id));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Handle **kwargs unpacking
        let mut expanded_kwargs = kwargs.to_vec();
        if let Some(kwargs_expr_id) = kwargs_unpack {
            let kwargs_expr = &hir_module.exprs[*kwargs_expr_id];
            match &kwargs_expr.kind {
                hir::ExprKind::Dict(pairs) => {
                    // Compile-time unpack: **{"a": 1, "b": 2}
                    for (key_id, value_id) in pairs {
                        let key_expr = &hir_module.exprs[*key_id];
                        if let hir::ExprKind::Str(key_str) = &key_expr.kind {
                            // CPython: check for duplicate kwargs (explicit + unpacked)
                            // f(a=1, **{"a": 2}) should raise TypeError
                            let key_name = self.resolve(*key_str);
                            if expanded_kwargs
                                .iter()
                                .any(|kw| self.resolve(kw.name) == key_name)
                            {
                                return Err(pyaot_diagnostics::CompilerError::type_error(
                                    format!(
                                        "got multiple values for keyword argument '{}'",
                                        key_name
                                    ),
                                    key_expr.span,
                                ));
                            }
                            expanded_kwargs.push(hir::KeywordArg {
                                name: *key_str,
                                value: *value_id,
                                span: key_expr.span,
                            });
                        } else {
                            // Non-string dict keys not supported
                            // Will be caught during type checking
                        }
                    }
                }
                _ => {
                    // Runtime **kwargs unpacking: lower the dict and pass to resolve_call_args
                    let dict_type = self.get_expr_type(kwargs_expr, hir_module);
                    let dict_operand = self.lower_expr(kwargs_expr, hir_module, mir_func)?;

                    // Get the value type from dict type
                    let value_type = match &dict_type {
                        Type::Dict(_, v) => (**v).clone(),
                        _ => Type::Any,
                    };

                    // Store the dict operand in a local for later use
                    if let mir::Operand::Local(dict_local) = dict_operand {
                        self.set_pending_kwargs(dict_local, value_type);
                    } else {
                        // If it's a constant (unlikely), copy to a local
                        let dict_local = self.alloc_and_add_local(dict_type.clone(), mir_func);
                        self.emit_instruction(mir::InstructionKind::Copy {
                            dest: dict_local,
                            src: dict_operand,
                        });
                        self.set_pending_kwargs(dict_local, value_type);
                    }
                }
            }
        }

        let args = &expanded_args;
        let kwargs = &expanded_kwargs;
        // Get the function expression
        let func_expr = &hir_module.exprs[func];

        // Handle imported function calls: ImportedRef or ModuleAttr
        if let hir::ExprKind::ImportedRef { module, name } = &func_expr.kind {
            return self
                .lower_imported_call(module, name, args, kwargs, expr, hir_module, mir_func);
        }

        if let hir::ExprKind::ModuleAttr { module, attr } = &func_expr.kind {
            let attr_name = self.resolve(*attr).to_string();

            // First, check if this is a class instantiation
            let key = (module.clone(), attr_name.clone());
            if let Some((class_id, class_name)) = self.get_module_class_export(&key).cloned() {
                // This is a cross-module class instantiation: module.ClassName(args)
                // The class_id is already remapped (offset-adjusted)
                return self.lower_cross_module_class_instantiation(
                    module,
                    class_id,
                    &class_name,
                    args,
                    kwargs,
                    hir_module,
                    mir_func,
                );
            }

            // Otherwise, treat as a function call
            return self
                .lower_imported_call(module, &attr_name, args, kwargs, expr, hir_module, mir_func);
        }

        // Check if this is a class instantiation: ClassName(args)
        if let hir::ExprKind::ClassRef(class_id) = &func_expr.kind {
            return self.lower_class_instantiation(*class_id, args, kwargs, hir_module, mir_func);
        }

        // Handle closures: prepend captured values to arguments
        if let hir::ExprKind::Closure {
            func: func_id,
            captures,
        } = &func_expr.kind
        {
            return self
                .lower_closure_call(*func_id, captures, args, kwargs, expr, hir_module, mir_func);
        }

        // Handle calling through a variable that holds a function reference
        if let hir::ExprKind::Var(var_id) = &func_expr.kind {
            // Check if this is a function pointer parameter (inside a wrapper function)
            // This happens when calling the captured `func` parameter in a decorator wrapper
            if self.is_func_ptr_param(var_id) {
                return self.lower_indirect_call(*var_id, args, hir_module, mir_func);
            }
            // Check if this variable holds a wrapper decorator closure (function-local)
            if let Some((wrapper_func_id, original_func_id)) = self.get_var_wrapper(var_id) {
                return self.lower_wrapper_call(
                    wrapper_func_id,
                    original_func_id,
                    args,
                    kwargs,
                    hir_module,
                    mir_func,
                );
            }
            // Check if this is a module-level variable that holds a wrapper decorator closure
            if let Some((wrapper_func_id, original_func_id)) = self.get_module_var_wrapper(var_id) {
                return self.lower_wrapper_call(
                    wrapper_func_id,
                    original_func_id,
                    args,
                    kwargs,
                    hir_module,
                    mir_func,
                );
            }
            // Check if this variable holds a closure
            if let Some((func_id, captures)) = self.get_var_closure(var_id).cloned() {
                return self.lower_closure_call(
                    func_id, &captures, args, kwargs, expr, hir_module, mir_func,
                );
            }
            // Check if this variable holds a function reference (function-local)
            if let Some(func_id) = self.get_var_func(var_id) {
                // Call the function directly
                let func_def = hir_module.func_defs.get(&func_id);
                let arg_operands = if let Some(func_def) = func_def {
                    self.resolve_call_args(
                        args,
                        kwargs,
                        &func_def.params,
                        Some(func_id),
                        0, // No offset for regular function calls
                        expr.span,
                        hir_module,
                        mir_func,
                    )?
                } else {
                    // Fallback: lower args with runtime unpacking support
                    self.lower_expanded_args(args, hir_module, mir_func)?
                };

                // Use inferred return type if available, then HIR return type, then Any
                let result_ty = self
                    .get_func_return_type(&func_id)
                    .cloned()
                    .or_else(|| func_def.and_then(|f| f.return_type.clone()))
                    .unwrap_or(Type::Any);
                let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: func_id,
                    args: arg_operands,
                });

                return Ok(mir::Operand::Local(result_local));
            }
            // Check if this is a module-level variable that holds a function reference
            if let Some(func_id) = self.get_module_var_func(var_id) {
                // Call the function directly
                let func_def = hir_module.func_defs.get(&func_id);
                let arg_operands = if let Some(func_def) = func_def {
                    self.resolve_call_args(
                        args,
                        kwargs,
                        &func_def.params,
                        Some(func_id),
                        0, // No offset for regular function calls
                        expr.span,
                        hir_module,
                        mir_func,
                    )?
                } else {
                    // Fallback: lower args with runtime unpacking support
                    self.lower_expanded_args(args, hir_module, mir_func)?
                };

                // Use inferred return type if available, then HIR return type, then Any
                let result_ty = self
                    .get_func_return_type(&func_id)
                    .cloned()
                    .or_else(|| func_def.and_then(|f| f.return_type.clone()))
                    .unwrap_or(Type::Any);
                let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: func_id,
                    args: arg_operands,
                });

                return Ok(mir::Operand::Local(result_local));
            }

            // Check if this variable holds a class instance with __call__
            if let Some(Type::Class { class_id, .. }) = self.get_var_type(var_id).cloned().as_ref()
            {
                if let Some(call_func_id) = self
                    .get_class_info(class_id)
                    .and_then(|info| info.get_dunder_func("__call__"))
                {
                    let obj_op = self.lower_expr(func_expr, hir_module, mir_func)?;
                    let mut call_args = vec![obj_op];
                    let user_args = self.lower_expanded_args(args, hir_module, mir_func)?;
                    call_args.extend(user_args);
                    let result_ty = self
                        .get_func_return_type(&call_func_id)
                        .cloned()
                        .unwrap_or(Type::Any);
                    let result_local = self.alloc_and_add_local(result_ty, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: call_func_id,
                        args: call_args,
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
            }

            // Final fallback for Var: if it's a global variable holding a function pointer
            // (e.g., from chained decorators or decorator factories), load it and do an indirect call
            if self.is_global(var_id) {
                // Load the closure tuple/function pointer from the global
                let closure_local = self.alloc_and_add_local(Type::Any, mir_func);
                let effective_var_id = self.get_effective_var_id(*var_id);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: closure_local,
                    func: mir::RuntimeFunc::Call(mir::ValueKind::Ptr.global_get_def()),
                    args: vec![mir::Operand::Constant(mir::Constant::Int(effective_var_id))],
                });

                // Lower the user arguments
                let user_arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;

                // Use the expression's type hint if available, otherwise Any
                let result_ty = expr.ty.clone().unwrap_or(Type::Any);

                // Use the helper to emit closure call with nested format (func_ptr, (captures...))
                let result_local =
                    self.emit_closure_call(closure_local, user_arg_operands, result_ty, mir_func);

                return Ok(mir::Operand::Local(result_local));
            }

            // Check if variable holds a dynamically returned closure (e.g., f = factory())
            // These need emit_closure_call to extract func_ptr and captures from the tuple
            if self.closures.dynamic_closure_vars.contains(var_id) {
                if let Some(local_id) = self.get_var_local(var_id) {
                    let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
                    let result_ty = expr.ty.clone().unwrap_or(Type::Any);
                    let result_local =
                        self.emit_closure_call(local_id, arg_operands, result_ty, mir_func);
                    return Ok(mir::Operand::Local(result_local));
                }
            }

            // General fallback: call through variable as an indirect function pointer.
            // This handles cases like calling a parameter that holds a function reference
            // (e.g., `func` parameter in decorator wrappers not detected by is_func_ptr_param).
            if let Some(local_id) = self.get_var_local(var_id) {
                let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
                let result_ty = self
                    .symbols
                    .current_func_return_type
                    .clone()
                    .or_else(|| expr.ty.clone())
                    .unwrap_or(Type::Any);
                let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Call {
                    dest: result_local,
                    func: mir::Operand::Local(local_id),
                    args: arg_operands,
                });
                return Ok(mir::Operand::Local(result_local));
            }
        }

        // We support direct function calls where func is a FuncRef
        // But first check if the func is actually an imported reference disguised as FuncRef
        // This can happen when the import is resolved during frontend processing
        if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
            // Check if this FuncId maps to a function in the current module
            // If not, it might be an unresolved import
            if !hir_module.func_defs.contains_key(func_id) {
                return Err(pyaot_diagnostics::CompilerError::semantic_error(
                    format!(
                        "unresolved cross-module function reference FuncId({})",
                        func_id.0
                    ),
                    func_expr.span,
                ));
            }
        }

        if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
            // Type check: validate arg count and types against function signature
            let regular_arg_ids: Vec<hir::ExprId> = args
                .iter()
                .filter_map(|a| {
                    if let ExpandedArg::Regular(id) = a {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect();
            // For error span: use the first arg's span (points to call site),
            // or construct from the func_expr span end (f(←here))
            let call_span = if let Some(first_arg) = regular_arg_ids.first() {
                hir_module.exprs[*first_arg].span
            } else {
                // No args at all — use func expression span
                func_expr.span
            };
            self.check_call_args(func_id, &regular_arg_ids, kwargs, call_span, hir_module);

            // Get function definition to access parameter names and defaults
            let func_def = hir_module.func_defs.get(func_id);

            // Use first arg span for call site location (Call expr span may be wrong)
            let call_site_span = args
                .iter()
                .find_map(|a| {
                    if let ExpandedArg::Regular(id) = a {
                        Some(hir_module.exprs[*id].span)
                    } else {
                        None
                    }
                })
                .unwrap_or(expr.span);

            let arg_operands = if let Some(func_def) = func_def {
                // Resolve arguments using helper (handles kwargs and defaults)
                self.resolve_call_args(
                    args,
                    kwargs,
                    &func_def.params,
                    Some(*func_id),
                    0, // No offset for regular function calls
                    call_site_span,
                    hir_module,
                    mir_func,
                )?
            } else {
                // Fallback: lower args with runtime unpacking support
                self.lower_expanded_args(args, hir_module, mir_func)?
            };

            // Create a destination local for the result
            // Check inferred return types first (for generators), then HIR definition
            let result_ty = self
                .get_func_return_type(func_id)
                .cloned()
                .or_else(|| func_def.and_then(|f| f.return_type.clone()))
                .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any));
            let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

            // Emit CallDirect instruction with known FuncId
            self.emit_instruction(mir::InstructionKind::CallDirect {
                dest: result_local,
                func: *func_id,
                args: arg_operands,
            });

            Ok(mir::Operand::Local(result_local))
        } else if let hir::ExprKind::Call { .. } = &func_expr.kind {
            // Handle case where func is itself a call expression (e.g., chained decorators,
            // decorator factories). The inner call returns a function/closure that we then
            // call with our args.
            //
            // The inner result may be:
            // 1. A raw function pointer (direct callable)
            // 2. A closure tuple with nested format: (func_ptr, (cap0, cap1, ...))
            //
            // We check the type tag and dispatch accordingly.
            let inner_result = self.lower_expr(func_expr, hir_module, mir_func)?;

            // Lower our arguments
            let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;

            // Use the expression's type hint if available, otherwise Any
            let result_ty = expr.ty.clone().unwrap_or(Type::Any);
            let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

            // Get the type tag of the inner result
            let type_tag_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: type_tag_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GET_TYPE_TAG),
                args: vec![inner_result.clone()],
            });

            // Compare with tuple tag
            let is_tuple_local = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: is_tuple_local,
                op: mir::BinOp::Eq,
                left: mir::Operand::Local(type_tag_local),
                right: mir::Operand::Constant(mir::Constant::Int(TypeTagKind::Tuple.tag() as i64)),
            });

            // Create blocks for the two cases
            let tuple_case_bb = self.new_block();
            let direct_case_bb = self.new_block();
            let merge_bb = self.new_block();
            let tuple_case_id = tuple_case_bb.id;
            let direct_case_id = direct_case_bb.id;
            let merge_id = merge_bb.id;

            // Branch based on whether it's a tuple
            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(is_tuple_local),
                then_block: tuple_case_id,
                else_block: direct_case_id,
            };

            // === Tuple case: use helper to call closure with nested format ===
            self.push_block(tuple_case_bb);

            // Store the inner result in a local for the helper
            let closure_local = if let mir::Operand::Local(l) = inner_result.clone() {
                l
            } else {
                let tmp = self.alloc_and_add_local(Type::Any, mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: tmp,
                    src: inner_result.clone(),
                });
                tmp
            };

            let tuple_result = self.emit_closure_call(
                closure_local,
                arg_operands.clone(),
                result_ty.clone(),
                mir_func,
            );

            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(tuple_result),
            });

            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

            // === Direct case: call directly ===
            self.push_block(direct_case_bb);

            let direct_result = self.alloc_and_add_local(result_ty, mir_func);
            self.emit_instruction(mir::InstructionKind::Call {
                dest: direct_result,
                func: inner_result,
                args: arg_operands,
            });

            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(direct_result),
            });

            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

            // === Merge block ===
            self.push_block(merge_bb);

            Ok(mir::Operand::Local(result_local))
        } else {
            Err(pyaot_diagnostics::CompilerError::semantic_error(
                "unsupported callable expression",
                expr.span,
            ))
        }
    }
}
