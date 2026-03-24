//! Call expression lowering: function calls and class instantiation

use pyaot_core_defs::TypeTagKind;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId};

use crate::context::Lowering;

/// Maximum number of captures supported in closure dispatch.
/// This limit exists because we generate static branches for each case.
const MAX_CLOSURE_CAPTURES: usize = 8;

/// Represents an expanded call argument
/// Used to track whether an argument needs runtime unpacking
#[derive(Debug, Clone, Copy)]
pub(crate) enum ExpandedArg {
    /// Regular argument - lower normally
    Regular(hir::ExprId),
    /// Runtime tuple unpacking - extract elements at runtime
    RuntimeUnpackTuple(hir::ExprId),
    /// Runtime list unpacking - extract elements at runtime
    RuntimeUnpackList(hir::ExprId),
}

impl<'a> Lowering<'a> {
    /// Lower expanded call arguments to MIR operands, handling runtime tuple unpacking.
    /// If `param_types` is provided, parameter types are propagated into argument
    /// expressions via `expected_type` (bidirectional type inference).
    pub(crate) fn lower_expanded_args(
        &mut self,
        expanded_args: &[ExpandedArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        self.lower_expanded_args_with_params(expanded_args, None, hir_module, mir_func)
    }

    /// Lower expanded call arguments with optional parameter type propagation.
    fn lower_expanded_args_with_params(
        &mut self,
        expanded_args: &[ExpandedArg],
        param_types: Option<&[hir::Param]>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Vec<mir::Operand>> {
        let mut operands = Vec::new();
        let mut positional_index = 0usize;

        for arg in expanded_args {
            match arg {
                ExpandedArg::Regular(expr_id) => {
                    let arg_expr = &hir_module.exprs[*expr_id];

                    // Bidirectional: propagate parameter type into argument expression
                    let expected = param_types
                        .and_then(|p| p.get(positional_index))
                        .and_then(|p| p.ty.clone());
                    let operand =
                        self.lower_expr_expecting(arg_expr, expected, hir_module, mir_func)?;

                    operands.push(operand);
                    positional_index += 1;
                }
                ExpandedArg::RuntimeUnpackTuple(expr_id) => {
                    // Runtime tuple unpacking - extract each element
                    let tuple_expr = &hir_module.exprs[*expr_id];
                    let tuple_type = self.get_expr_type(tuple_expr, hir_module);

                    // Lower the tuple expression to get the operand
                    let tuple_operand = self.lower_expr(tuple_expr, hir_module, mir_func)?;

                    // Extract element types
                    if let Type::Tuple(elem_types) = tuple_type {
                        // Extract each element from the tuple
                        for (i, elem_type) in elem_types.iter().enumerate() {
                            let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

                            // Choose appropriate Get function based on element type
                            let get_func = match elem_type {
                                Type::Int => mir::RuntimeFunc::TupleGetInt,
                                Type::Float => mir::RuntimeFunc::TupleGetFloat,
                                Type::Bool => mir::RuntimeFunc::TupleGetBool,
                                _ => mir::RuntimeFunc::TupleGet,
                            };

                            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                                dest: elem_local,
                                func: get_func,
                                args: vec![
                                    tuple_operand.clone(),
                                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                                ],
                            });

                            operands.push(mir::Operand::Local(elem_local));
                        }
                    } else {
                        // Should not happen - type checker should catch this
                        // But handle gracefully by passing the tuple as-is
                        operands.push(tuple_operand);
                    }
                }
                ExpandedArg::RuntimeUnpackList(_expr_id) => {
                    // TODO: Implement full list unpacking for all call paths
                    // Runtime list unpacking is handled in resolve_call_args
                    // where we have access to the function signature.
                    // This case should not be reached when using lower_expanded_args
                    // directly (without resolve_call_args).
                    return Err(pyaot_diagnostics::CompilerError::semantic_error(
                        "Star unpacking of non-literal lists is not yet supported in this call context",
                        pyaot_utils::Span::dummy(),
                    ));
                }
            }
        }

        Ok(operands)
    }

    /// Lower a function call expression.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_call(
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
                    .and_then(|info| info.call_func)
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
                    func: mir::RuntimeFunc::GlobalGet(mir::ValueKind::Ptr),
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
        }

        // We support direct function calls where func is a FuncRef
        // But first check if the func is actually an imported reference disguised as FuncRef
        // This can happen when the import is resolved during frontend processing
        if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
            // Check if this FuncId maps to a function in the current module
            // If not, it might be an unresolved import
            if !hir_module.func_defs.contains_key(func_id) {
                // TODO: cross-module FuncRef — this FuncId is not defined in the current
                // module, so it refers to an imported function that was not yet resolved
                // into a CallNamed/CallDirect. Proper handling requires linking the FuncId
                // back to its source module and emitting a CallNamed instruction.
                // For now, lower the args (for side-effects) and return None as a fallback.
                let _ = kwargs; // Ignore kwargs for now
                let _ = expr;
                let _arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        }

        if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
            // Type check: validate arg count and types against function signature
            let regular_arg_ids: Vec<hir::ExprId> = args
                .iter()
                .filter_map(|a| {
                    if let crate::expressions::calls::ExpandedArg::Regular(id) = a {
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
            self.check_call_args(func_id, &regular_arg_ids, call_span, hir_module);

            // Get function definition to access parameter names and defaults
            let func_def = hir_module.func_defs.get(func_id);

            // Use first arg span for call site location (Call expr span may be wrong)
            let call_site_span = args
                .iter()
                .find_map(|a| {
                    if let crate::expressions::calls::ExpandedArg::Regular(id) = a {
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
                func: mir::RuntimeFunc::GetTypeTag,
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
            // TODO: unhandled callee expression kind — this branch is reached for callee
            // shapes that are not yet supported (e.g., subscript expressions, arbitrary
            // callable objects). Proper handling would require emitting a runtime dispatch.
            // Returning None here silently produces incorrect results.
            Ok(mir::Operand::Constant(mir::Constant::None))
        }
    }

    /// Lower a closure call.
    /// Closures have captured variables that need to be prepended to the argument list.
    #[allow(clippy::too_many_arguments)]
    fn lower_closure_call(
        &mut self,
        func_id: pyaot_utils::FuncId,
        captures: &[hir::ExprId],
        args: &[ExpandedArg],
        _kwargs: &[hir::KeywordArg],
        _expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Lower captured expressions first (these are prepended to args)
        // For cell variables (used by nonlocal), pass the cell pointer directly
        let mut all_args = Vec::new();
        for capture_id in captures {
            let capture_expr = &hir_module.exprs[*capture_id];
            // Check if this capture is a cell variable - if so, pass the cell pointer directly
            let capture_op = if let hir::ExprKind::Var(var_id) = &capture_expr.kind {
                if let Some(cell_local) = self.get_nonlocal_cell(var_id) {
                    // This is a cell variable - pass the cell pointer, not the value
                    mir::Operand::Local(cell_local)
                } else {
                    self.lower_expr(capture_expr, hir_module, mir_func)?
                }
            } else {
                self.lower_expr(capture_expr, hir_module, mir_func)?
            };
            all_args.push(capture_op);
        }

        // Then lower regular call arguments with runtime unpacking support
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
        all_args.extend(arg_operands);

        // Get return type: check inferred types first, then HIR definition
        let func_def = hir_module.func_defs.get(&func_id);
        let result_ty = self
            .get_func_return_type(&func_id)
            .cloned()
            .or_else(|| func_def.and_then(|f| f.return_type.clone()))
            .unwrap_or(Type::Any);

        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Emit CallDirect instruction with combined args (captures + user args)
        self.emit_instruction(mir::InstructionKind::CallDirect {
            dest: result_local,
            func: func_id,
            args: all_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a wrapper decorator call.
    /// Wrapper decorators return a closure that wraps the original function.
    /// The wrapper receives the original function address as its first capture argument.
    #[allow(clippy::too_many_arguments)]
    fn lower_wrapper_call(
        &mut self,
        wrapper_func_id: pyaot_utils::FuncId,
        original_func_id: pyaot_utils::FuncId,
        args: &[ExpandedArg],
        _kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // 1. Get function address of original function
        // The wrapper expects this as its first argument (the captured 'func' parameter)
        let func_ptr_local = self.alloc_and_add_local(Type::Any, mir_func);
        self.emit_instruction(mir::InstructionKind::FuncAddr {
            dest: func_ptr_local,
            func: original_func_id,
        });

        // 2. Build arguments: func_ptr + user args
        let mut all_args = vec![mir::Operand::Local(func_ptr_local)];

        // Lower user arguments with runtime unpacking support
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
        all_args.extend(arg_operands);

        // 3. Get return type from wrapper function
        let func_def = hir_module.func_defs.get(&wrapper_func_id);
        let result_ty = self
            .get_func_return_type(&wrapper_func_id)
            .cloned()
            .or_else(|| func_def.and_then(|f| f.return_type.clone()))
            .unwrap_or(Type::Any);

        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // 4. Call wrapper with combined args (func_ptr + user args)
        self.emit_instruction(mir::InstructionKind::CallDirect {
            dest: result_local,
            func: wrapper_func_id,
            args: all_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower an indirect call through a function pointer parameter.
    /// This is used inside wrapper functions when calling the captured `func` parameter.
    fn lower_indirect_call(
        &mut self,
        func_var_id: pyaot_utils::VarId,
        args: &[ExpandedArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Get the function pointer from the parameter variable
        let func_local = self
            .get_var_local(&func_var_id)
            .expect("Function pointer parameter not found");

        // Lower the arguments
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;

        // Use the wrapper function's return type for the indirect call result
        // This ensures type consistency when the wrapper returns the call result
        let result_ty = mir_func.return_type.clone();
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // For chained decorators, the `func` parameter might be:
        // 1. A raw function pointer (when the decorator receives a FuncRef directly)
        // 2. A closure tuple with nested format: (func_ptr, (cap0, cap1, ...))
        //
        // We check the type tag and dispatch accordingly.

        // Get the type tag
        let type_tag_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: type_tag_local,
            func: mir::RuntimeFunc::GetTypeTag,
            args: vec![mir::Operand::Local(func_local)],
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

        let tuple_result = self.emit_closure_call(
            func_local,
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
            func: mir::Operand::Local(func_local),
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
    }

    /// Lower a class instantiation: ClassName(args)
    /// Creates instance, initializes fields to null, then calls __init__ if present
    pub(crate) fn lower_class_instantiation(
        &mut self,
        class_id: pyaot_utils::ClassId,
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let class_info = self.get_class_info(&class_id).cloned();

        if let Some(info) = class_info {
            // Create the class type - get class name from class definition
            let class_name = match hir_module.class_defs.get(&class_id) {
                Some(class_def) => class_def.name,
                None => {
                    // If we can't find the class, return None
                    return Ok(mir::Operand::Constant(mir::Constant::None));
                }
            };

            let class_type = Type::Class {
                class_id,
                name: class_name,
            };

            // Allocate result local for the instance
            let result_local = self.alloc_and_add_local(class_type, mir_func);

            // Create instance: rt_make_instance(class_id, total_field_count)
            // Use total_field_count to include inherited fields
            // Use offset-adjusted class_id for local classes
            let effective_class_id = self.get_effective_class_id(class_id);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::MakeInstance,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                    mir::Operand::Constant(mir::Constant::Int(info.total_field_count as i64)),
                ],
            });

            // Call __init__ if present
            if let Some(init_func_id) = info.init_func {
                // Get the __init__ function definition
                if let Some(init_func) = hir_module.func_defs.get(&init_func_id) {
                    // Resolve arguments: __init__ takes self as first argument
                    // Note: __init__ params include 'self', so we skip it when matching user args
                    let init_params: Vec<_> = init_func.params.iter().skip(1).cloned().collect();

                    // Lower the user-provided arguments (skip self)
                    // We use param_index_offset=1 because:
                    // - default_value_slots uses (FuncId, param_index) where param_index is
                    //   relative to the original function parameters (including self)
                    // - init_params skips self, so param at index 0 in init_params is actually
                    //   at index 1 in the original function
                    let user_args = self.resolve_call_args(
                        args,
                        kwargs,
                        &init_params,
                        Some(init_func_id),
                        1,                          // Offset by 1 because self is skipped
                        pyaot_utils::Span::dummy(), // class instantiation has no call expr
                        hir_module,
                        mir_func,
                    )?;

                    // Build full args: self + user args
                    let mut all_args = vec![mir::Operand::Local(result_local)];
                    all_args.extend(user_args);

                    // Create dummy local for __init__ return (always None)
                    let init_result_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Call __init__
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: init_result_local,
                        func: init_func_id,
                        args: all_args,
                    });
                }
            }

            Ok(mir::Operand::Local(result_local))
        } else {
            // Unknown class
            Ok(mir::Operand::Constant(mir::Constant::None))
        }
    }

    /// Lower a cross-module class instantiation: module.ClassName(args)
    /// The class_id is already remapped (offset-adjusted) from module_class_exports.
    #[allow(clippy::too_many_arguments)]
    fn lower_cross_module_class_instantiation(
        &mut self,
        source_module: &str,
        class_id: pyaot_utils::ClassId,
        class_name: &str,
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // For cross-module classes, we don't have class_info available.
        // We create the instance and call __init__ via CallNamed.

        // Lower arguments with runtime unpacking support
        let _ = kwargs; // Ignore kwargs for now
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;

        // Use Type::Any for cross-module classes
        // We can't create a proper Type::Class because the class_name InternedString
        // is from a different module's interner. Type::Any maps to pointer type
        // which is correct for class instances.
        let _ = class_name; // Used for __init__ call only
        let class_type = Type::Any;

        // Allocate result local for the instance
        let result_local = self.alloc_and_add_local(class_type, mir_func);

        // Try to get actual field count from class info.
        // Fall back to 32 as a conservative upper bound for cross-module classes whose
        // metadata is not yet available in this compilation unit.
        // TODO: export total_field_count in cross-module class metadata so this fallback
        //       can be eliminated and the correct value used unconditionally.
        let default_field_count = self
            .get_class_info(&class_id)
            .map(|info| info.total_field_count as i64)
            .unwrap_or(32);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeInstance,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(class_id.0 as i64)),
                mir::Operand::Constant(mir::Constant::Int(default_field_count)),
            ],
        });

        // Call __init__ via CallNamed
        // The __init__ function name is mangled as __module_{module}_{class}$__init__
        // (uses $ as separator between class name and method name)
        // Replace dots with underscores for package paths
        let safe_module = source_module.replace('.', "_");
        let init_func_name = format!("__module_{}_{}$__init__", safe_module, class_name);

        // Create dummy local for __init__ return (always None)
        let init_result_local = self.alloc_and_add_local(Type::None, mir_func);

        // Build args: self + user args
        let mut all_args = vec![mir::Operand::Local(result_local)];
        all_args.extend(arg_operands);

        // Call __init__ via CallNamed
        self.emit_instruction(mir::InstructionKind::CallNamed {
            dest: init_result_local,
            name: init_func_name,
            args: all_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a call to an imported function.
    /// Generates a CallNamed instruction that will be resolved at codegen time.
    #[allow(clippy::too_many_arguments)]
    fn lower_imported_call(
        &mut self,
        module: &str,
        name: &str,
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Generate the mangled function name (replace dots with underscores for packages)
        let safe_module = module.replace('.', "_");
        let mangled_name = format!("__module_{}_{}", safe_module, name);

        // Lower arguments with runtime unpacking support (ignore kwargs for imported functions for now)
        let _ = kwargs;
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;

        // Determine result type:
        // 1. Check module_func_exports for cross-module function return types
        // 2. Fall back to expression type hint
        // 3. Default to Any
        let key = (module.to_string(), name.to_string());
        let result_ty = self
            .get_module_func_export(&key)
            .cloned()
            .or_else(|| expr.ty.clone())
            .unwrap_or(Type::Any);
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Emit CallNamed instruction - will be resolved at codegen time
        self.emit_instruction(mir::InstructionKind::CallNamed {
            dest: result_local,
            name: mangled_name,
            args: arg_operands,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Emit code to call a closure stored in a tuple with the nested format:
    /// `(func_ptr, (cap0, cap1, ...))`.
    ///
    /// This helper extracts the function pointer and captures from the closure tuple,
    /// then generates branching code to handle different numbers of captures (0 to MAX_CLOSURE_CAPTURES).
    ///
    /// # Arguments
    /// * `closure_local` - The local containing the closure tuple
    /// * `user_args` - The user-provided arguments to pass after captures
    /// * `result_ty` - The expected return type
    /// * `mir_func` - The MIR function being built
    ///
    /// # Returns
    /// The local containing the call result
    fn emit_closure_call(
        &mut self,
        closure_local: LocalId,
        user_args: Vec<mir::Operand>,
        result_ty: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // Result local shared across all branches
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Extract func_ptr from index 0
        let func_ptr_local = self.alloc_and_add_local(Type::Any, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: func_ptr_local,
            func: mir::RuntimeFunc::TupleGet,
            args: vec![
                mir::Operand::Local(closure_local),
                mir::Operand::Constant(mir::Constant::Int(0)),
            ],
        });

        // Extract captures tuple from index 1
        let captures_tuple = self.alloc_and_add_local(Type::Any, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: captures_tuple,
            func: mir::RuntimeFunc::TupleGet,
            args: vec![
                mir::Operand::Local(closure_local),
                mir::Operand::Constant(mir::Constant::Int(1)),
            ],
        });

        // Get the number of captures
        let n_captures_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: n_captures_local,
            func: mir::RuntimeFunc::TupleLen,
            args: vec![mir::Operand::Local(captures_tuple)],
        });

        // Create merge block for all branches
        let merge_bb = self.new_block();
        let merge_id = merge_bb.id;

        // Generate cascading branches for 0, 1, 2, ... MAX_CLOSURE_CAPTURES captures
        // Each branch checks if n_captures == i, and if so, extracts i captures and calls
        self.emit_capture_dispatch(
            0,
            func_ptr_local,
            captures_tuple,
            n_captures_local,
            &user_args,
            result_local,
            result_ty,
            merge_id,
            mir_func,
        );

        // Push the merge block
        self.push_block(merge_bb);

        result_local
    }

    /// Recursively emit capture dispatch branches.
    /// For each capture count from `current` to MAX_CLOSURE_CAPTURES, generate:
    /// - Check if n_captures == current
    /// - If yes: extract captures and call
    /// - If no: continue to next case
    #[allow(clippy::too_many_arguments)]
    fn emit_capture_dispatch(
        &mut self,
        current: usize,
        func_ptr_local: LocalId,
        captures_tuple: LocalId,
        n_captures_local: LocalId,
        user_args: &[mir::Operand],
        result_local: LocalId,
        result_ty: Type,
        merge_id: BlockId,
        mir_func: &mut mir::Function,
    ) {
        if current > MAX_CLOSURE_CAPTURES {
            // Fallback: call with just user args (shouldn't normally reach here)
            let fallback_result = self.alloc_and_add_local(result_ty, mir_func);
            self.emit_instruction(mir::InstructionKind::Call {
                dest: fallback_result,
                func: mir::Operand::Local(func_ptr_local),
                args: user_args.to_vec(),
            });
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(fallback_result),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);
            return;
        }

        // Check if n_captures == current
        let is_current = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: is_current,
            op: mir::BinOp::Eq,
            left: mir::Operand::Local(n_captures_local),
            right: mir::Operand::Constant(mir::Constant::Int(current as i64)),
        });

        // Create blocks
        let match_bb = self.new_block();
        let next_bb = self.new_block();
        let match_id = match_bb.id;
        let next_id = next_bb.id;

        // Branch
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(is_current),
            then_block: match_id,
            else_block: next_id,
        };

        // Match case: extract `current` captures and call
        self.push_block(match_bb);

        // Extract all captures
        let mut call_args = Vec::with_capacity(current + user_args.len());
        for i in 0..current {
            let cap_local = self.alloc_and_add_local(Type::Any, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: cap_local,
                func: mir::RuntimeFunc::TupleGet,
                args: vec![
                    mir::Operand::Local(captures_tuple),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                ],
            });
            call_args.push(mir::Operand::Local(cap_local));
        }
        call_args.extend(user_args.iter().cloned());

        // Make the call
        let branch_result = self.alloc_and_add_local(result_ty.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Call {
            dest: branch_result,
            func: mir::Operand::Local(func_ptr_local),
            args: call_args,
        });

        // Copy to shared result
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: mir::Operand::Local(branch_result),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // Continue with next case
        self.push_block(next_bb);
        self.emit_capture_dispatch(
            current + 1,
            func_ptr_local,
            captures_tuple,
            n_captures_local,
            user_args,
            result_local,
            result_ty,
            merge_id,
            mir_func,
        );
    }
}
