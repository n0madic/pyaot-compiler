//! Assignment statement lowering
//!
//! Handles: Assign, UnpackAssign, IndexAssign, FieldAssign, ClassAttrAssign
//!
//! Split into focused submodules:
//! - `unpack`: Tuple/list unpacking assignments (UnpackAssign, NestedUnpackAssign)
//! - `augmented`: Index/field/class_attr assignments + index delete

mod augmented;
mod unpack;

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
                    let left_ty = self.get_type_of_expr_id(*left, hir_module);
                    if matches!(left_ty, Type::Dict(_, _)) {
                        let dict_operand = self.lower_expr(left_expr, hir_module, mir_func)?;
                        let right_expr = &hir_module.exprs[*right];
                        let right_operand = self.lower_expr(right_expr, hir_module, mir_func)?;

                        let dummy = self.alloc_and_add_local(Type::None, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: dummy,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_DICT_UPDATE,
                            ),
                            args: vec![dict_operand, right_operand],
                        });
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
                // Skip lowering - calls are resolved through var_to_func
                return Ok(());
            }
            hir::ExprKind::Closure { func, captures } => {
                self.insert_var_closure(target, *func, captures.clone());

                // Track capture types for the lambda function
                let mut capture_types = Vec::new();
                for capture_id in captures {
                    let capture_type = self.get_type_of_expr_id(*capture_id, hir_module);
                    capture_types.push(capture_type);
                }
                self.insert_closure_capture_types(*func, capture_types.clone());

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
                    let func_addr_local = self.alloc_and_add_local(Type::Any, mir_func);
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: func_addr_local,
                        func: *func,
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

                    let void_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Determine elem_tag for captures tuple based on actual types.
                    // Use ELEM_RAW_INT when no capture needs GC tracing.
                    // Cell variables are always heap pointers that need GC tracing.
                    let capture_elem_tag: i64 = {
                        let any_needs_gc = captures.iter().enumerate().any(|(i, capture_id)| {
                            let capture_expr = &hir_module.exprs[*capture_id];
                            if let hir::ExprKind::Var(var_id) = &capture_expr.kind {
                                if self.get_nonlocal_cell(var_id).is_some() {
                                    return true;
                                }
                            }
                            let op_type = self.operand_type(&capture_operands[i], mir_func);
                            op_type.is_heap()
                        });
                        if any_needs_gc {
                            0
                        } else {
                            1
                        }
                    };

                    // Collect per-operand types for heap_field_mask
                    let capture_types: Vec<Type> = capture_operands
                        .iter()
                        .map(|op| self.operand_type(op, mir_func))
                        .collect();

                    // Create inner captures tuple
                    let captures_tuple = self.alloc_and_add_local(Type::Any, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: captures_tuple,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE,
                        ),
                        args: vec![
                            mir::Operand::Constant(mir::Constant::Int(captures.len() as i64)),
                            mir::Operand::Constant(mir::Constant::Int(capture_elem_tag)),
                        ],
                    });

                    // Set per-field heap_field_mask when tuple has mixed types (ELEM_HEAP_OBJ)
                    if capture_elem_tag == 0 {
                        self.emit_heap_field_mask(captures_tuple, &capture_types, mir_func);
                    }

                    // Store each capture in the inner tuple at index 0, 1, ...
                    for (i, capture_op) in capture_operands.iter().enumerate() {
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: void_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_TUPLE_SET,
                            ),
                            args: vec![
                                mir::Operand::Local(captures_tuple),
                                mir::Operand::Constant(mir::Constant::Int(i as i64)),
                                capture_op.clone(),
                            ],
                        });
                    }

                    // Create outer tuple (func_ptr, captures_tuple) - always size 2
                    // heap_field_mask: bit 0 = 0 (func_ptr is raw), bit 1 = 1 (captures_tuple is heap)
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dest_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE,
                        ),
                        args: vec![
                            mir::Operand::Constant(mir::Constant::Int(2)),
                            mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                        ],
                    });
                    // Set mask: only index 1 (captures_tuple) is a heap pointer
                    self.emit_heap_field_mask(
                        dest_local,
                        &[Type::Int, Type::Any], // func_ptr=raw, captures=heap
                        mir_func,
                    );

                    // Store func_ptr at index 0
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: void_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_TUPLE_SET,
                        ),
                        args: vec![
                            mir::Operand::Local(dest_local),
                            mir::Operand::Constant(mir::Constant::Int(0)),
                            mir::Operand::Local(func_addr_local),
                        ],
                    });

                    // Store captures_tuple at index 1
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: void_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_TUPLE_SET,
                        ),
                        args: vec![
                            mir::Operand::Local(dest_local),
                            mir::Operand::Constant(mir::Constant::Int(1)),
                            mir::Operand::Local(captures_tuple),
                        ],
                    });
                }
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
                                    return Ok(());
                                }
                            }
                        }
                        // Identity-like decorator: track the original function directly
                        self.insert_var_func(target, innermost_func_id);
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
                    }
                }
            }
        }

        // Type check: validate RHS type against type hint (if present)
        if let Some(ref hint) = type_hint {
            self.check_expr_type(value, hint, hir_module);
        }

        // Priority: explicit type hint > existing variable type > inferred from RHS
        // For reassignments to existing variables (especially Union types),
        // we need to use the variable's original type, not the RHS type
        let var_type = type_hint.unwrap_or_else(|| {
            // Check if variable already has a type (reassignment case)
            self.get_var_type(&target)
                .cloned()
                .unwrap_or_else(|| self.get_type_of_expr_id(value, hir_module))
        });
        // Track the variable type for later reference
        self.insert_var_type(target, var_type.clone());

        // Check if this is a narrowed Union variable - if so, we need to use the
        // original Union type for boxing, even though the narrowed type is not Union
        let original_union_type = self.get_narrowed_union_type(&target);

        // Lower the value expression with expected type for bidirectional propagation
        let value_operand =
            self.lower_expr_expecting(expr, Some(var_type.clone()), hir_module, mir_func)?;

        // Box primitives when assigning to Union type (or narrowed Union variable)
        let final_operand = if var_type.is_union() || original_union_type.is_some() {
            let value_type = self.get_type_of_expr_id(value, hir_module);
            self.box_primitive_if_needed(value_operand, &value_type, mir_func)
        } else {
            value_operand
        };

        // Check if this is a global variable
        if self.is_global(&target) {
            // Global variable: emit runtime call to set the value
            // We need a local for the intermediate value
            let dest_local = self.get_or_create_local(target, var_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: final_operand.clone(),
            });

            // Create a dummy local for the void return
            let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

            // Determine the type-specific runtime function for global set
            let runtime_func = self.get_global_set_func(&var_type);

            // Emit type-specific GlobalSet runtime call with offset-adjusted VarId
            let effective_var_id = self.get_effective_var_id(target);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: runtime_func,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_var_id)),
                    final_operand,
                ],
            });
        } else if let Some(cell_local) = self.get_nonlocal_cell(&target) {
            // Cell-wrapped variable (either cell_var or nonlocal_var): write through cell
            // Don't create a local for the variable - it lives in the cell
            // Create a dummy local for the void return
            let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

            // Determine the type-specific runtime function for cell set
            let set_func = self.get_cell_set_func(&var_type);

            // Emit cell set operation
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: set_func,
                args: vec![mir::Operand::Local(cell_local), final_operand],
            });
        } else {
            // Local variable: standard copy
            let dest_local = self.get_or_create_local(target, var_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: final_operand,
            });
        }

        Ok(())
    }
}
