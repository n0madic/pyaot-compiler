//! Assignment statement lowering
//!
//! Handles: Assign, UnpackAssign, IndexAssign, FieldAssign, ClassAttrAssign

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{ClassId, InternedString, VarId};

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
                    let capture_expr = &hir_module.exprs[*capture_id];
                    let capture_type = self.get_expr_type(capture_expr, hir_module);
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
                    let mut capture_operands = Vec::new();
                    for capture_id in captures {
                        let capture_expr = &hir_module.exprs[*capture_id];
                        let capture_operand =
                            self.lower_expr(capture_expr, hir_module, mir_func)?;
                        capture_operands.push(capture_operand);
                    }

                    let void_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Create inner captures tuple
                    let captures_tuple = self.alloc_and_add_local(Type::Any, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: captures_tuple,
                        func: mir::RuntimeFunc::MakeTuple,
                        args: vec![
                            mir::Operand::Constant(mir::Constant::Int(captures.len() as i64)),
                            mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                        ],
                    });

                    // Store each capture in the inner tuple at index 0, 1, ...
                    for (i, capture_op) in capture_operands.iter().enumerate() {
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: void_local,
                            func: mir::RuntimeFunc::TupleSet,
                            args: vec![
                                mir::Operand::Local(captures_tuple),
                                mir::Operand::Constant(mir::Constant::Int(i as i64)),
                                capture_op.clone(),
                            ],
                        });
                    }

                    // Create outer tuple (func_ptr, captures_tuple) - always size 2
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dest_local,
                        func: mir::RuntimeFunc::MakeTuple,
                        args: vec![
                            mir::Operand::Constant(mir::Constant::Int(2)),
                            mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                        ],
                    });

                    // Store func_ptr at index 0
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: void_local,
                        func: mir::RuntimeFunc::TupleSet,
                        args: vec![
                            mir::Operand::Local(dest_local),
                            mir::Operand::Constant(mir::Constant::Int(0)),
                            mir::Operand::Local(func_addr_local),
                        ],
                    });

                    // Store captures_tuple at index 1
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: void_local,
                        func: mir::RuntimeFunc::TupleSet,
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

        // Priority: explicit type hint > existing variable type > inferred from RHS
        // For reassignments to existing variables (especially Union types),
        // we need to use the variable's original type, not the RHS type
        let var_type = type_hint.unwrap_or_else(|| {
            // Check if variable already has a type (reassignment case)
            self.get_var_type(&target)
                .cloned()
                .unwrap_or_else(|| self.get_expr_type(expr, hir_module))
        });
        // Track the variable type for later reference
        self.insert_var_type(target, var_type.clone());

        // Check if this is a narrowed Union variable - if so, we need to use the
        // original Union type for boxing, even though the narrowed type is not Union
        let original_union_type = self.get_narrowed_union_type(&target);

        // Lower the value expression first
        let value_operand = self.lower_expr(expr, hir_module, mir_func)?;

        // Box primitives when assigning to Union type (or narrowed Union variable)
        let final_operand = if var_type.is_union() || original_union_type.is_some() {
            let value_type = self.get_expr_type(expr, hir_module);
            self.box_value_for_union(value_operand, &value_type, mir_func)
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

    /// Lower an unpacking assignment: a, b = value or a, *rest, b = value
    ///
    /// Supports extended unpacking with starred expression:
    /// - `a, *rest = [1, 2, 3, 4]` → a=1, rest=[2, 3, 4]
    /// - `*rest, b = [1, 2, 3, 4]` → rest=[1, 2, 3], b=4
    /// - `a, *mid, b = [1, 2, 3, 4]` → a=1, mid=[2, 3], b=4
    pub(crate) fn lower_unpack_assign(
        &mut self,
        before_star: &[VarId],
        starred: Option<&VarId>,
        after_star: &[VarId],
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let expr = &hir_module.exprs[value];
        let value_type = self.get_expr_type(expr, hir_module);

        // Lower the RHS expression once
        let value_operand = self.lower_expr(expr, hir_module, mir_func)?;

        // Determine the slice function based on source type
        // For starred unpacking, we always produce a list, so tuples use TupleSliceToList
        let (is_tuple, slice_func) = match &value_type {
            Type::List(_) => (false, mir::RuntimeFunc::ListSlice),
            _ => (true, mir::RuntimeFunc::TupleSliceToList),
        };

        // Helper to determine element type
        let get_elem_type = |index: usize| -> Type {
            match &value_type {
                Type::Tuple(elem_types) => {
                    if index < elem_types.len() {
                        elem_types[index].clone()
                    } else {
                        Type::Any
                    }
                }
                Type::List(elem_ty) => (**elem_ty).clone(),
                _ => Type::Any,
            }
        };

        // Helper to determine element type for negative index
        let get_elem_type_neg = |neg_index: i64| -> Type {
            match &value_type {
                Type::Tuple(elem_types) => {
                    let len = elem_types.len() as i64;
                    let actual_idx = (len + neg_index) as usize;
                    if actual_idx < elem_types.len() {
                        elem_types[actual_idx].clone()
                    } else {
                        Type::Any
                    }
                }
                Type::List(elem_ty) => (**elem_ty).clone(),
                _ => Type::Any,
            }
        };

        // Store all extracted values in temps first (for parallel assignment safety)
        let mut temp_locals = Vec::new();

        // 1. Extract before_star elements with positive indices
        for (i, _target) in before_star.iter().enumerate() {
            let index_operand = mir::Operand::Constant(mir::Constant::Int(i as i64));
            let elem_type = get_elem_type(i);

            let temp_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

            // Choose appropriate Get function based on element type
            let get_func = if is_tuple {
                match &elem_type {
                    Type::Int => mir::RuntimeFunc::TupleGetInt,
                    Type::Float => mir::RuntimeFunc::TupleGetFloat,
                    Type::Bool => mir::RuntimeFunc::TupleGetBool,
                    _ => mir::RuntimeFunc::TupleGet,
                }
            } else {
                mir::RuntimeFunc::ListGet
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: temp_local,
                func: get_func,
                args: vec![value_operand.clone(), index_operand],
            });

            temp_locals.push((temp_local, elem_type));
        }

        // 2. Extract starred portion (if any) using slice
        if let Some(_starred_var) = starred {
            // Slice indices: start = before_star.len(), end = -after_star.len() (or None if 0)
            let start_idx = before_star.len() as i64;
            let end_idx = if after_star.is_empty() {
                // No after_star: slice to the end (use a large number, runtime will clamp)
                i64::MAX
            } else {
                -(after_star.len() as i64)
            };

            // Starred always produces a list
            let starred_elem_type = match &value_type {
                Type::List(elem_ty) => (**elem_ty).clone(),
                Type::Tuple(elem_types) => {
                    // Union of all types in the starred portion
                    let middle_start = before_star.len();
                    let middle_end = elem_types.len().saturating_sub(after_star.len());
                    if middle_start < middle_end {
                        let middle_types: Vec<_> = elem_types[middle_start..middle_end].to_vec();
                        Type::normalize_union(middle_types)
                    } else {
                        Type::Any
                    }
                }
                _ => Type::Any,
            };
            let starred_type = Type::List(Box::new(starred_elem_type));

            let temp_local = self.alloc_and_add_local(starred_type.clone(), mir_func);

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: temp_local,
                func: slice_func,
                args: vec![
                    value_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(start_idx)),
                    mir::Operand::Constant(mir::Constant::Int(end_idx)),
                ],
            });

            temp_locals.push((temp_local, starred_type));
        }

        // 3. Extract after_star elements with negative indices
        for (i, _target) in after_star.iter().enumerate() {
            // Negative index: -(after_star.len() - i)
            let neg_index = -((after_star.len() - i) as i64);
            let index_operand = mir::Operand::Constant(mir::Constant::Int(neg_index));
            let elem_type = get_elem_type_neg(neg_index);

            let temp_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

            // Choose appropriate Get function based on element type
            let get_func = if is_tuple {
                match &elem_type {
                    Type::Int => mir::RuntimeFunc::TupleGetInt,
                    Type::Float => mir::RuntimeFunc::TupleGetFloat,
                    Type::Bool => mir::RuntimeFunc::TupleGetBool,
                    _ => mir::RuntimeFunc::TupleGet,
                }
            } else {
                mir::RuntimeFunc::ListGet
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: temp_local,
                func: get_func,
                args: vec![value_operand.clone(), index_operand],
            });

            temp_locals.push((temp_local, elem_type));
        }

        // 4. Copy from temps to actual target variables
        // Verify that we have the expected number of temp locals before iterating
        debug_assert_eq!(
            temp_locals.len(),
            before_star.len() + starred.is_some() as usize + after_star.len(),
            "temp_locals count mismatch in unpacking assignment"
        );

        let mut temp_iter = temp_locals.into_iter();

        // Copy to before_star targets
        for target in before_star {
            let (temp_local, elem_type) = temp_iter
                .next()
                .expect("unpacking requires at least one temporary for before_star targets");
            self.insert_var_type(*target, elem_type.clone());
            let dest_local = self.get_or_create_local(*target, elem_type, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: mir::Operand::Local(temp_local),
            });
        }

        // Copy to starred target (if any)
        if let Some(starred_var) = starred {
            let (temp_local, starred_type) = temp_iter
                .next()
                .expect("unpacking requires at least one temporary for starred target");
            self.insert_var_type(*starred_var, starred_type.clone());
            let dest_local = self.get_or_create_local(*starred_var, starred_type, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: mir::Operand::Local(temp_local),
            });
        }

        // Copy to after_star targets
        for target in after_star {
            let (temp_local, elem_type) = temp_iter
                .next()
                .expect("unpacking requires at least one temporary for after_star targets");
            self.insert_var_type(*target, elem_type.clone());
            let dest_local = self.get_or_create_local(*target, elem_type, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: dest_local,
                src: mir::Operand::Local(temp_local),
            });
        }

        Ok(())
    }

    /// Lower an index assignment: obj[index] = value
    pub(crate) fn lower_index_assign(
        &mut self,
        obj: hir::ExprId,
        index: hir::ExprId,
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_expr_type(obj_expr, hir_module);

        let index_expr = &hir_module.exprs[index];
        let index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;
        let index_type = self.get_expr_type(index_expr, hir_module);

        let value_expr = &hir_module.exprs[value];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;
        let value_type = self.get_expr_type(value_expr, hir_module);

        // Create a dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        match obj_type {
            Type::Dict(ref key_ty, ref val_ty) => {
                // Refine Dict(Any, Any) type based on actual key/value types
                // This happens with dict comprehensions where the initial empty dict has unknown types
                if **key_ty == Type::Any || **val_ty == Type::Any {
                    if let hir::ExprKind::Var(var_id) = &obj_expr.kind {
                        let refined_key = if **key_ty == Type::Any && index_type != Type::Any {
                            Box::new(index_type.clone())
                        } else {
                            key_ty.clone()
                        };
                        let refined_val = if **val_ty == Type::Any && value_type != Type::Any {
                            Box::new(value_type.clone())
                        } else {
                            val_ty.clone()
                        };
                        self.insert_var_type(*var_id, Type::Dict(refined_key, refined_val));
                    }
                }

                // dict[key] = value - box key and value if needed (primitives must be boxed for GC)
                let boxed_key = self.box_dict_key_if_needed(index_operand, &index_type, mir_func);
                let boxed_value =
                    self.box_dict_value_if_needed(value_operand, &value_type, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::DictSet,
                    args: vec![obj_operand, boxed_key, boxed_value],
                });
            }
            Type::List(_) => {
                // list[index] = value
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::ListSet,
                    args: vec![obj_operand, index_operand, value_operand],
                });
            }
            Type::Class { class_id, .. } => {
                // Class with __setitem__ dunder
                let setitem_func = self
                    .get_class_info(&class_id)
                    .and_then(|info| info.setitem_func);

                if let Some(func_id) = setitem_func {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: func_id,
                        args: vec![obj_operand, index_operand, value_operand],
                    });
                }
            }
            _ => {
                // Unsupported type for indexed assignment
            }
        }

        Ok(())
    }

    /// Lower a delete indexed item: del obj[key]
    /// Uses DictPop for dicts and ListPop for lists (discarding the result).
    pub(crate) fn lower_index_delete(
        &mut self,
        obj: hir::ExprId,
        index: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_expr_type(obj_expr, hir_module);

        let index_expr = &hir_module.exprs[index];
        let index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;
        let index_type = self.get_expr_type(index_expr, hir_module);

        // Create a dummy local for the discarded return value
        // Use Type::Any (i64) since DictPop/ListPop return heap pointers
        let dummy_local = self.alloc_and_add_local(Type::Any, mir_func);

        match obj_type {
            Type::Dict(_, _) => {
                // del dict[key] → rt_dict_pop(dict, key) and discard result
                let boxed_key = self.box_dict_key_if_needed(index_operand, &index_type, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::DictPop,
                    args: vec![obj_operand, boxed_key],
                });
            }
            Type::List(_) => {
                // del list[index] → rt_list_pop(list, index) and discard result
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::ListPop,
                    args: vec![obj_operand, index_operand],
                });
            }
            Type::Class { class_id, .. } => {
                // Class with __delitem__ dunder
                let delitem_func = self
                    .get_class_info(&class_id)
                    .and_then(|info| info.delitem_func);

                if let Some(func_id) = delitem_func {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: func_id,
                        args: vec![obj_operand, index_operand],
                    });
                }
            }
            _ => {
                // Unsupported type for indexed delete
            }
        }

        Ok(())
    }

    /// Lower a field assignment: obj.field = value
    /// Also handles @property setters.
    pub(crate) fn lower_field_assign(
        &mut self,
        obj: hir::ExprId,
        field: InternedString,
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_expr_type(obj_expr, hir_module);

        let value_expr = &hir_module.exprs[value];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;

        // Look up field offset from class info
        if let Type::Class { class_id, .. } = &obj_type {
            if let Some(class_info) = self.get_class_info(class_id).cloned() {
                // 1. Check for @property setter first
                if let Some((_getter, Some(setter_id))) = class_info.properties.get(&field) {
                    let setter_id = *setter_id;
                    // Create a dummy local for void return
                    let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Call the setter with (self, value)
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: setter_id,
                        args: vec![obj_operand, value_operand],
                    });

                    return Ok(());
                }

                // 2. Regular field assignment
                if let Some(&offset) = class_info.field_offsets.get(&field) {
                    // Create a dummy local for void return
                    let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Set the field value
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::InstanceSetField,
                        args: vec![
                            obj_operand,
                            mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                            value_operand,
                        ],
                    });
                }
            }
        }

        Ok(())
    }

    /// Lower a class attribute assignment: ClassName.attr = value
    pub(crate) fn lower_class_attr_assign(
        &mut self,
        class_id: ClassId,
        attr: InternedString,
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let value_expr = &hir_module.exprs[value];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;

        // Look up class attribute (owning_class_id, offset) and type from class info
        // The owning_class_id is the class where the attribute was actually defined
        if let Some(class_info) = self.get_class_info(&class_id) {
            if let (Some(&(owning_class_id, attr_offset)), Some(attr_type)) = (
                class_info.class_attr_offsets.get(&attr),
                class_info.class_attr_types.get(&attr).cloned(),
            ) {
                // Create a dummy local for void return
                let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

                // Get the appropriate runtime function based on type
                let set_func = self.get_class_attr_set_func(&attr_type);

                // Emit runtime call: rt_class_attr_set_*(owning_class_id, attr_idx, value)
                // Use the owning_class_id, not the accessed class_id, to handle inheritance
                let effective_class_id = self.get_effective_class_id(owning_class_id);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: set_func,
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                        value_operand,
                    ],
                });
            }
        }

        Ok(())
    }

    /// Lower nested unpacking assignment: (a, (b, c)) = value
    /// Supports arbitrary depth nesting through recursive extraction
    pub(crate) fn lower_nested_unpack_assign(
        &mut self,
        targets: &[hir::UnpackTarget],
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let expr = &hir_module.exprs[value];
        let value_type = self.get_expr_type(expr, hir_module);

        // Lower the RHS expression once
        let value_operand = self.lower_expr(expr, hir_module, mir_func)?;

        // Recursively extract nested targets
        self.lower_nested_recursive(targets, value_operand, &value_type, mir_func)?;

        Ok(())
    }

    /// Recursively lower nested unpacking pattern - handles arbitrary nesting depth
    fn lower_nested_recursive(
        &mut self,
        targets: &[hir::UnpackTarget],
        source_operand: mir::Operand,
        source_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let is_tuple = matches!(source_type, Type::Tuple(_));
        let elem_types: Vec<Type> = match source_type {
            Type::Tuple(types) => types.clone(),
            Type::List(inner) => vec![(**inner).clone(); targets.len()],
            _ => vec![Type::Any; targets.len()],
        };

        for (i, target) in targets.iter().enumerate() {
            let elem_type = elem_types.get(i).cloned().unwrap_or(Type::Any);

            match target {
                hir::UnpackTarget::Var(var_id) => {
                    // Simple variable - extract and assign directly
                    let get_func = if is_tuple {
                        match &elem_type {
                            Type::Int => mir::RuntimeFunc::TupleGetInt,
                            Type::Float => mir::RuntimeFunc::TupleGetFloat,
                            Type::Bool => mir::RuntimeFunc::TupleGetBool,
                            _ => mir::RuntimeFunc::TupleGet,
                        }
                    } else {
                        mir::RuntimeFunc::ListGet
                    };

                    self.insert_var_type(*var_id, elem_type.clone());
                    let dest_local = if let Some(existing) = self.get_var_local(var_id) {
                        existing
                    } else {
                        let new_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
                        self.insert_var_local(*var_id, new_local);
                        new_local
                    };

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dest_local,
                        func: get_func,
                        args: vec![
                            source_operand.clone(),
                            mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        ],
                    });
                }
                hir::UnpackTarget::Nested(nested_targets) => {
                    // Extract nested tuple first into a temp
                    let nested_temp = self.alloc_and_add_local(elem_type.clone(), mir_func);
                    let get_func = if is_tuple {
                        mir::RuntimeFunc::TupleGet
                    } else {
                        mir::RuntimeFunc::ListGet
                    };

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: nested_temp,
                        func: get_func,
                        args: vec![
                            source_operand.clone(),
                            mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        ],
                    });

                    // Recursively unpack nested targets
                    self.lower_nested_recursive(
                        nested_targets,
                        mir::Operand::Local(nested_temp),
                        &elem_type,
                        mir_func,
                    )?;
                }
            }
        }

        Ok(())
    }

    // Removed recursive extraction functions - using simplified flat version above
}
