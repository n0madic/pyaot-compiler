//! Assignment statement lowering
//!
//! Handles: Bind (unified), IndexDelete
//!
//! Split into focused submodules:
//! - `bind`: Unified binding targets (Bind, ForBind)
//! - `augmented`: Index delete

mod augmented;
mod bind;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a simple assignment: target = value
    pub(crate) fn lower_assign(
        &mut self,
        target: VarId,
        value: hir::ExprId,
        type_hint: Option<Type>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let expr = &hir_module.exprs[value];

        // Detect in-place dict update: d |= other → d = d | other (desugared)
        // Instead of creating a new dict via DictMerge, call DictUpdate in-place
        // to preserve alias semantics (matching CPython behavior).
        if let hir::ExprKind::BinOp {
            op: hir::BinOp::BitOr,
            left,
            right,
        } = &expr.kind
        {
            let left_expr = &hir_module.exprs[*left];
            if let hir::ExprKind::Var(var_id) = &left_expr.kind {
                if *var_id == target {
                    let left_ty = self.seed_expr_type(*left, hir_module);
                    if matches!(left_ty, Type::Dict(_, _)) {
                        let dict_operand = self.lower_expr(left_expr, hir_module, mir_func)?;
                        let right_expr = &hir_module.exprs[*right];
                        let right_operand = self.lower_expr(right_expr, hir_module, mir_func)?;

                        let _dummy = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_DICT_UPDATE,
                            ),
                            vec![dict_operand, right_operand],
                            Type::None,
                            mir_func,
                        );
                        self.remove_block_narrowed_local(&target);
                        return Ok(());
                    }
                }
            }
        }

        // Track if RHS is a function reference or closure for later call resolution
        // For these cases, we don't need to emit any code - we resolve calls through tracking maps
        match &expr.kind {
            hir::ExprKind::FuncRef(func_id) => {
                self.insert_var_func(target, *func_id);
                self.remove_block_narrowed_local(&target);
                // Skip lowering - calls are resolved through var_to_func
                return Ok(());
            }
            hir::ExprKind::Closure { func, captures } => {
                self.insert_var_closure(target, *func, captures.clone());

                // Track capture types for the lambda function
                let mut capture_types = Vec::new();
                for capture_id in captures {
                    let capture_type = self.seed_expr_type(*capture_id, hir_module);
                    capture_types.push(capture_type);
                }
                self.insert_closure_capture_types(*func, capture_types.clone());

                // §P.2.2: re-infer the lambda's return type now that capture
                // types are populated. Type-planning Pass 2 ran before this
                // statement was lowered and saw Any-typed captures, widening
                // the inferred return to Any. Without this re-inference,
                // `map(fn_closure, ...)` later resolves to `Iterator[Any]`
                // and the for-loop's `IterAdvance` Protocol arm passes the
                // lambda's raw scalar return through unchanged, leaving raw
                // int bits in a HeapAny shadow-stack slot that trips the GC
                // alignment guard.
                if let Some(func_def) = hir_module.func_defs.get(func) {
                    if func_def.return_type.is_none() && func_def.blocks.len() == 1 {
                        let cached = self.get_func_return_type(func).cloned();
                        let inferred = self.infer_lambda_return_type(func_def, hir_module);
                        let needs_update = match cached.as_ref() {
                            None => true,
                            Some(prev) => {
                                matches!(prev, Type::Any) && !matches!(inferred, Type::Any)
                            }
                        };
                        if needs_update && !matches!(inferred, Type::Any) {
                            self.insert_func_return_type(*func, inferred);
                        }
                    }
                }

                // For closures that may be returned from decorators (used in chained decorators),
                // we need to emit code to store the function pointer in the local.
                // This allows the closure to be returned and used as a first-class value.
                //
                // For closures with captures (decorator wrappers), we create a tuple
                // (func_ptr, capture) so that when called indirectly, we can extract
                // the capture and pass it as the first argument.
                let dest_local = self.get_or_create_local(target, Type::Any, mir_func);

                if captures.is_empty() {
                    // No captures - just store the function address
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: dest_local,
                        func: *func,
                    });
                } else {
                    // Create nested closure tuple: (func_ptr, (cap0, cap1, ...))
                    // Outer tuple always has exactly 2 elements for uniform dispatch
                    let func_addr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: func_addr_local,
                        func: *func,
                    });

                    // §F.5: wrap the raw i64 function pointer as a tagged
                    // `Value::from_int` so the closure tuple slot 0 reads
                    // as `is_ptr() == false`. Without this, the GC's mark
                    // walk would treat the function-code address as a heap
                    // pointer and either follow it (SEGV) or rely on the
                    // address-heuristic filter, which §F.8 removes.
                    let func_addr_value = self.alloc_stack_local(Type::HeapAny, mir_func);
                    self.emit_instruction(mir::InstructionKind::ValueFromInt {
                        dest: func_addr_value,
                        src: mir::Operand::Local(func_addr_local),
                    });

                    // Lower all capture expressions
                    // For cell variables (used by nonlocal), pass the cell pointer directly
                    let mut capture_operands = Vec::new();
                    for capture_id in captures {
                        let capture_expr = &hir_module.exprs[*capture_id];
                        let capture_operand = if let hir::ExprKind::Var(var_id) = &capture_expr.kind
                        {
                            if let Some(cell_local) = self.get_nonlocal_cell(var_id) {
                                // Cell variable — pass the cell pointer, not the value
                                mir::Operand::Local(cell_local)
                            } else {
                                self.lower_expr(capture_expr, hir_module, mir_func)?
                            }
                        } else {
                            self.lower_expr(capture_expr, hir_module, mir_func)?
                        };
                        capture_operands.push(capture_operand);
                    }

                    // After §F.7c: captures tuple stores uniform tagged Values;
                    // primitives are boxed below.
                    let captures_tuple = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
                        vec![mir::Operand::Constant(mir::Constant::Int(
                            captures.len() as i64
                        ))],
                        Type::Any,
                        mir_func,
                    );

                    // §P.2.2: wrap fn-ptr captures (per wrapper-driven mask)
                    // so GC sees Value::from_int-tagged bits, not raw text.
                    let fn_ptr_idx = self.wrapper_fn_ptr_capture_index(*func, hir_module);
                    for (i, capture_op) in capture_operands.iter().enumerate() {
                        let stored_op = if Some(i) == fn_ptr_idx {
                            let wrapped = self.alloc_stack_local(Type::HeapAny, mir_func);
                            self.emit_instruction(mir::InstructionKind::ValueFromInt {
                                dest: wrapped,
                                src: capture_op.clone(),
                            });
                            mir::Operand::Local(wrapped)
                        } else {
                            let op_type = self.operand_type(capture_op, mir_func);
                            self.box_primitive_if_needed(capture_op.clone(), &op_type, mir_func)
                        };
                        self.emit_runtime_call_void(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_TUPLE_SET,
                            ),
                            vec![
                                mir::Operand::Local(captures_tuple),
                                mir::Operand::Constant(mir::Constant::Int(i as i64)),
                                stored_op,
                            ],
                            mir_func,
                        );
                    }

                    // Create outer tuple (func_ptr, captures_tuple) - size 2.
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dest_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE,
                        ),
                        args: vec![mir::Operand::Constant(mir::Constant::Int(2))],
                    });

                    // Store func_ptr at index 0 — tagged as `Value::from_int`
                    // so the slot is_ptr() == false (see §F.5 above).
                    self.emit_runtime_call_void(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
                        vec![
                            mir::Operand::Local(dest_local),
                            mir::Operand::Constant(mir::Constant::Int(0)),
                            mir::Operand::Local(func_addr_value),
                        ],
                        mir_func,
                    );

                    // Store captures_tuple at index 1
                    self.emit_runtime_call_void(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
                        vec![
                            mir::Operand::Local(dest_local),
                            mir::Operand::Constant(mir::Constant::Int(1)),
                            mir::Operand::Local(captures_tuple),
                        ],
                        mir_func,
                    );
                }
                self.remove_block_narrowed_local(&target);
                return Ok(());
            }
            // Handle decorated function pattern: var = decorator(FuncRef(func))
            // This pattern arises from @decorator syntax desugaring
            hir::ExprKind::Call { func, args, .. } => {
                let func_expr = &hir_module.exprs[*func];

                // Check for decorator factory pattern: func is itself a Call
                // e.g., @multiply(3) def f -> multiply(3) returns the actual decorator
                let is_factory = matches!(&func_expr.kind, hir::ExprKind::Call { .. });

                // Check if this is a chained decorator (nested calls in args).
                let is_chained = args.iter().any(|arg| {
                    if let hir::CallArg::Regular(arg_id) = arg {
                        matches!(hir_module.exprs[*arg_id].kind, hir::ExprKind::Call { .. })
                    } else {
                        false
                    }
                });

                // Decorator factories always need runtime evaluation because:
                // 1. The factory must be called first with its arguments
                // 2. The result (a closure/decorator) must then be applied to the function
                // 3. The decorator typically captures the factory arguments in a closure
                let needs_runtime_eval = is_factory
                    || (is_chained && self.chain_contains_wrapper_decorator(expr, hir_module));

                // For chained wrapper decorators or decorator factories, fall through to
                // evaluate the expression. The full chain needs runtime evaluation to
                // properly capture closures.
                if !needs_runtime_eval {
                    // Try to find the innermost FuncRef in a decorator chain
                    if let Some(innermost_func_id) = self.find_innermost_func_ref(expr, hir_module)
                    {
                        // Check what the outermost decorator function is
                        if let hir::ExprKind::FuncRef(decorator_func_id) = &func_expr.kind {
                            // Check if the decorator returns a closure (wrapper pattern)
                            if let Some(decorator_def) = hir_module.func_defs.get(decorator_func_id)
                            {
                                if let Some(wrapper_func_id) =
                                    self.find_returned_closure(decorator_def, hir_module)
                                {
                                    // Wrapper decorator: track wrapper with original function
                                    // The wrapper will receive the original function as its first capture
                                    self.insert_var_wrapper(
                                        target,
                                        wrapper_func_id,
                                        innermost_func_id,
                                    );
                                    // Mark this function as a wrapper so we know to handle
                                    // indirect calls to its first parameter
                                    self.insert_wrapper_func_id(wrapper_func_id);
                                    self.remove_block_narrowed_local(&target);
                                    return Ok(());
                                }
                            }
                        }
                        // Identity-like decorator: track the original function directly
                        self.insert_var_func(target, innermost_func_id);
                        self.remove_block_narrowed_local(&target);
                        return Ok(());
                    }
                }
                // For chained wrapper decorators or decorator factories, fall through to
                // evaluate the expression
            }
            _ => {}
        }

        // Check if RHS is a call to a function that returns a closure.
        // If so, mark the target variable as a dynamic closure so f() uses emit_closure_call.
        if let hir::ExprKind::Call { func, .. } = &expr.kind {
            let func_expr = &hir_module.exprs[*func];
            if let Some((called_func_id, _)) =
                self.extract_func_with_captures(func_expr, hir_module)
            {
                if let Some(func_def) = hir_module.func_defs.get(&called_func_id) {
                    if self.find_returned_closure(func_def, hir_module).is_some() {
                        self.closures.dynamic_closure_vars.insert(target);
                        // §P.2.2: record the outermost wrapper's return type
                        // so the indirect call site can type its result
                        // local precisely.
                        if let Some(ret_ty) = self.outermost_wrapper_return_type(expr, hir_module) {
                            self.insert_dynamic_closure_return_type(target, ret_ty);
                        }
                    }
                }
            }
        }

        // Type check: validate RHS type against type hint (if present)
        if let Some(ref hint) = type_hint {
            self.check_expr_type(value, hint, hir_module);
        }

        // Priority: explicit type hint > pre-scanned unified type (Area E §E.6)
        // > existing variable type > inferred from RHS.
        //
        // The pre-scan pass walks the full function body ahead of time and
        // merges every binding observation through the numeric tower, so a
        // local that is written `Int` once and `Float` once is typed
        // `Float` here — and the Int write gets coerced below.
        let has_explicit_type_hint = type_hint.is_some();
        let initial_var_type = type_hint.unwrap_or_else(|| {
            // Priority: refined container type > prescan (when useful)
            // > active block-narrowing storage type > stable/base var type
            // > live narrowed var_type > RHS inference.
            //
            // Prescan is skipped if it's "uselessly wide" — an Any-
            // parameterised container (Dict(Any, Any), List(Any),
            // Set(Any)) — because the RHS-driven inference or later
            // refinement will produce a tighter type.
            let prescan = self
                .lowering_seed_info
                .current_local_seed_types
                .get(&target)
                .cloned()
                .filter(|ty| !crate::is_useless_container_ty(ty));
            let base = self.get_base_var_type(&target).cloned().filter(|ty| {
                !matches!(ty, Type::Any | Type::HeapAny) && !crate::is_useless_container_ty(ty)
            });
            self.lowering_seed_info
                .refined_container_types
                .get(&target)
                .cloned()
                .or(prescan)
                .or_else(|| self.get_block_narrowed_storage_type(&target).cloned())
                .or(base)
                .or_else(|| self.get_var_type(&target).cloned())
                .unwrap_or_else(|| self.seed_expr_type(value, hir_module))
        });

        // Lower the value expression with expected type for bidirectional propagation
        let value_operand =
            self.lower_expr_expecting(expr, Some(initial_var_type.clone()), hir_module, mir_func)?;
        let value_type = self.resolved_value_type_hint(value, &value_operand, hir_module, mir_func);
        let seed_value_type = self.seed_expr_type(value, hir_module);
        let semantic_value_type =
            if matches!(value_type, Type::Any | Type::HeapAny | Type::Union(_))
                && seed_value_type != value_type
                && !matches!(seed_value_type, Type::Any | Type::HeapAny | Type::Union(_))
            {
                seed_value_type
            } else {
                value_type.clone()
            };

        let mut var_type = initial_var_type;
        if !has_explicit_type_hint
            && (matches!(var_type, Type::Any | Type::HeapAny)
                || crate::is_useless_container_ty(&var_type))
            && !matches!(value_type, Type::Any | Type::HeapAny)
            && !crate::is_useless_container_ty(&value_type)
        {
            var_type = value_type.clone();
        }
        // Box primitives when assigning to Union type (or narrowed Union variable)
        // or coerce through the numeric tower when the target local is
        // wider than the RHS (Area E §E.6: `x = 0; x += 0.5` widens
        // `x: Float`; the literal `0` must be `IntToFloat`'d).
        let final_operand = if var_type.is_union() {
            self.box_primitive_if_needed(value_operand, &value_type, mir_func)
        } else {
            self.coerce_to_field_type(value_operand, &value_type, &var_type, mir_func)
        };

        self.remove_block_narrowed_local(&target);

        // Check if this is a global variable
        if self.is_global(&target) {
            // Global variable: emit runtime call to set the value
            // We need a local for the intermediate value
            let dest_local = self.get_or_create_local(target, var_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: final_operand.clone(),
            });

            // Determine the type-specific runtime function for global set
            let runtime_func = self.get_global_set_func(&var_type);

            // Emit type-specific GlobalSet runtime call with offset-adjusted VarId
            let effective_var_id = self.get_effective_var_id(target);
            self.emit_runtime_call_void(
                runtime_func,
                vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_var_id)),
                    final_operand,
                ],
                mir_func,
            );

            self.insert_var_type(target, var_type);
        } else if let Some(cell_local) = self.get_nonlocal_cell(&target) {
            // Cell-wrapped variable (either cell_var or nonlocal_var): write through cell
            // Don't create a local for the variable - it lives in the cell

            // Determine the type-specific runtime function for cell set
            let set_func = self.get_cell_set_func(&var_type);

            // Emit cell set operation
            self.emit_runtime_call_void(
                set_func,
                vec![mir::Operand::Local(cell_local), final_operand],
                mir_func,
            );

            self.insert_var_type(target, var_type);
        } else {
            // Local variable: standard copy
            let dest_local = self.get_or_create_local(target, var_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: final_operand,
            });

            let storage_ty = mir_func.locals[&dest_local].ty.clone();
            let needs_post_assign_narrowing = !has_explicit_type_hint
                && storage_ty != semantic_value_type
                && !matches!(semantic_value_type, Type::Any | Type::HeapAny)
                && matches!(storage_ty, Type::Any | Type::HeapAny | Type::Union(_));

            if needs_post_assign_narrowing {
                let narrowed_local = self.materialize_narrowed_local_from_operand(
                    mir::Operand::Local(dest_local),
                    &storage_ty,
                    &semantic_value_type,
                    mir_func,
                );
                self.insert_block_narrowed_local(
                    target,
                    narrowed_local,
                    storage_ty,
                    semantic_value_type.clone(),
                );
                self.insert_var_type(target, semantic_value_type);
            } else {
                self.insert_var_type(target, var_type);
            }
        }

        Ok(())
    }
}
