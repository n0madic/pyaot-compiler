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
                            let arg_type = match &arg_expr.kind {
                                hir::ExprKind::Var(var_id) => self
                                    .get_var_local(var_id)
                                    .and_then(|local_id| mir_func.locals.get(&local_id))
                                    .map(|local| local.ty.clone())
                                    .unwrap_or_else(|| self.seed_expr_type(*expr_id, hir_module)),
                                _ => self.seed_expr_type(*expr_id, hir_module),
                            };
                            if arg_type.is_tuple_like() {
                                // Mark for runtime tuple unpacking
                                expanded_args.push(ExpandedArg::RuntimeUnpackTuple(*expr_id));
                            } else if arg_type.is_list_like() {
                                // Mark for runtime list unpacking
                                expanded_args.push(ExpandedArg::RuntimeUnpackList(*expr_id));
                            } else {
                                // Not a tuple or list - pass as is (will cause type error)
                                expanded_args.push(ExpandedArg::Regular(*expr_id));
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
                    let dict_operand = self.lower_expr(kwargs_expr, hir_module, mir_func)?;
                    let dict_type = match &dict_operand {
                        mir::Operand::Local(local_id) => mir_func
                            .locals
                            .get(local_id)
                            .map(|local| local.ty.clone())
                            .unwrap_or_else(|| self.seed_expr_type(*kwargs_expr_id, hir_module)),
                        _ => self.seed_expr_type(*kwargs_expr_id, hir_module),
                    };

                    // Get the value type from dict type
                    let value_type = dict_type
                        .dict_kv()
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Type::Any);

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
            // Class constructor: `from mymod import Foo; Foo(...)` reaches
            // here with `ImportedRef`. Route to the class-instantiation path
            // so we emit `mymod.Foo$__init__` (not a bogus `mymod.Foo`
            // function symbol, which doesn't exist in the generated MIR).
            let key = (module.clone(), name.clone());
            if let Some((class_id, class_name)) = self.get_module_class_export(&key).cloned() {
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
                let effective_var_id = self.get_effective_var_id(*var_id);
                let closure_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GLOBAL_GET_PTR),
                    vec![mir::Operand::Constant(mir::Constant::Int(effective_var_id))],
                    Type::Any,
                    mir_func,
                );

                // Lower the user arguments
                let mut user_arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
                // §P.2.2: wrap fn-ptr args via `ValueFromInt` before storing
                // in the args-tuple, so the GC walker sees a tagged-int slot
                // (low bit 1) instead of a raw text-segment address.
                self.wrap_func_ptr_args_for_tuple(
                    &mut user_arg_operands,
                    args,
                    hir_module,
                    mir_func,
                );

                // §P.2.2: prefer the recorded outermost-wrapper return type
                // (set during chained-decorator pre-scan) over `expr.ty`/Any.
                // This types the result local precisely so chained-wrapper
                // calls don't leave raw scalars in HeapAny shadow-stack slots.
                let result_ty = self
                    .get_dynamic_closure_return_type(var_id)
                    .cloned()
                    .or_else(|| expr.ty.clone())
                    .unwrap_or(Type::Any);
                let arg_types: Vec<Type> = user_arg_operands
                    .iter()
                    .map(|op| self.operand_type(op, mir_func))
                    .collect();
                let args_tuple = self.create_tuple_from_operands_typed(
                    &user_arg_operands,
                    &Type::Any,
                    Some(&arg_types),
                    mir_func,
                );
                return self.lower_indirect_call_with_varargs(
                    closure_local,
                    args_tuple,
                    result_ty,
                    mir_func,
                );
            }

            // Check if variable holds a dynamically returned closure (e.g., f = factory())
            // These need emit_closure_call to extract func_ptr and captures from the tuple
            if self.closures.dynamic_closure_vars.contains(var_id) {
                if let Some(local_id) = self.get_var_local(var_id) {
                    let mut arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
                    self.wrap_func_ptr_args_for_tuple(
                        &mut arg_operands,
                        args,
                        hir_module,
                        mir_func,
                    );
                    // §P.2.2: prefer the recorded outermost-wrapper return
                    // type over `expr.ty`/Any. Without this, the result
                    // local lands in a `HeapAny is_gc_root=true` slot
                    // holding the wrapper's raw scalar return — tripping
                    // the GC alignment guard for chained-decorator chains.
                    let result_ty = self
                        .get_dynamic_closure_return_type(var_id)
                        .cloned()
                        .or_else(|| expr.ty.clone())
                        .unwrap_or(Type::Any);
                    let arg_types: Vec<Type> = arg_operands
                        .iter()
                        .map(|op| self.operand_type(op, mir_func))
                        .collect();
                    let args_tuple = self.create_tuple_from_operands_typed(
                        &arg_operands,
                        &Type::Any,
                        Some(&arg_types),
                        mir_func,
                    );
                    return self.lower_indirect_call_with_varargs(
                        local_id, args_tuple, result_ty, mir_func,
                    );
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
            // Closure-recursion detection: when a nested function with
            // captured variables is referenced by `FuncRef` inside its
            // own body (recursive call), the frontend emits
            // `FuncRef(func_id)` rather than a fresh `Closure { … }`
            // to avoid a circular capture reference. The callee's
            // parameter list then has leading `__capture_*` entries
            // that the AST-level call (`build_topo(v)`) never supplies.
            // Forward them from the enclosing function's own
            // like-named `__capture_*` params, which are already in
            // scope as MIR locals.
            let forwarded_captures: Vec<mir::Operand> =
                if let Some(func_def) = hir_module.func_defs.get(func_id) {
                    let capture_count = func_def
                        .params
                        .iter()
                        .take_while(|p| self.resolve(p.name).starts_with("__capture_"))
                        .count();
                    func_def
                        .params
                        .iter()
                        .take(capture_count)
                        .map(|p| {
                            let local = self.get_var_local(&p.var);
                            match local {
                                Some(l) => mir::Operand::Local(l),
                                None => mir::Operand::Constant(mir::Constant::None),
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            let has_forwarded_captures = !forwarded_captures.is_empty();

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
            let has_runtime_unpack = args
                .iter()
                .any(|arg| !matches!(arg, ExpandedArg::Regular(_)));
            let has_runtime_kwargs_unpack = kwargs_unpack
                .as_ref()
                .map(|expr_id| &hir_module.exprs[*expr_id])
                .is_some_and(|kwargs_expr| !matches!(kwargs_expr.kind, hir::ExprKind::Dict(_)));

            // Skip the legacy HIR-side validator when we're forwarding captures
            // or when the call site uses runtime *args/**kwargs expansion.
            // `check_call_args` only reasons about explicit HIR arg expr ids,
            // while `resolve_call_args` is the canonical path that understands
            // runtime unpacking and default/kwarg filling.
            if !has_forwarded_captures && !has_runtime_unpack && !has_runtime_kwargs_unpack {
                self.check_call_args(func_id, &regular_arg_ids, kwargs, call_span, hir_module);
            }

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

            let mut arg_operands = if let Some(func_def) = func_def {
                if has_forwarded_captures {
                    // Pass only the non-capture suffix to `resolve_call_args`,
                    // with `capture_count` as the offset so defaults /
                    // mutable-default-slot bookkeeping keep indexing into
                    // the original param list correctly.
                    let capture_count = forwarded_captures.len();
                    let non_capture_params: Vec<hir::Param> = func_def
                        .params
                        .iter()
                        .skip(capture_count)
                        .cloned()
                        .collect();
                    self.resolve_call_args(
                        args,
                        kwargs,
                        &non_capture_params,
                        Some(*func_id),
                        capture_count,
                        call_site_span,
                        hir_module,
                        mir_func,
                    )?
                } else {
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
                }
            } else {
                // Fallback: lower args with runtime unpacking support
                self.lower_expanded_args(args, hir_module, mir_func)?
            };

            // Prepend the synthesized captures so the CallDirect's arg
            // list matches the callee's full param arity.
            if has_forwarded_captures {
                let mut combined = forwarded_captures;
                combined.append(&mut arg_operands);
                arg_operands = combined;
            }

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
            let type_tag_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GET_TYPE_TAG),
                vec![inner_result.clone()],
                Type::Int,
                mir_func,
            );

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
