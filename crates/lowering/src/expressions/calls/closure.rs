//! Closure, wrapper, and indirect call lowering

use pyaot_core_defs::TypeTagKind;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId};

use crate::context::Lowering;

use super::{ExpandedArg, MAX_CLOSURE_CAPTURES};

impl<'a> Lowering<'a> {
    /// Lower a closure call.
    /// Closures have captured variables that need to be prepended to the argument list.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_closure_call(
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
    pub(super) fn lower_wrapper_call(
        &mut self,
        wrapper_func_id: pyaot_utils::FuncId,
        original_func_id: pyaot_utils::FuncId,
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
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

        // Check if the wrapper has *args — if so, use resolve_call_args to properly
        // pack user arguments into a varargs tuple matching the wrapper's signature
        let has_varargs = hir_module
            .func_defs
            .get(&wrapper_func_id)
            .map(|f| {
                f.params
                    .iter()
                    .any(|p| p.kind == hir::ParamKind::VarPositional)
            })
            .unwrap_or(false);

        if has_varargs {
            // Get the wrapper's user-facing params (skip the capture 'func' param)
            let user_params: Vec<hir::Param> = hir_module
                .func_defs
                .get(&wrapper_func_id)
                .map(|f| f.params.iter().skip(1).cloned().collect())
                .unwrap_or_default();

            let user_arg_operands = self.resolve_call_args(
                args,
                kwargs,
                &user_params,
                Some(wrapper_func_id),
                1, // offset for capture param
                self.call_span(),
                hir_module,
                mir_func,
            )?;
            all_args.extend(user_arg_operands);
        } else {
            // No *args — lower directly (existing behavior)
            let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
            all_args.extend(arg_operands);
        }

        // 3. Get return type: prefer original function's return type (more precise),
        // fall back to wrapper's return type
        let result_ty = self
            .get_func_return_type(&original_func_id)
            .cloned()
            .or_else(|| {
                hir_module
                    .func_defs
                    .get(&original_func_id)
                    .and_then(|f| f.return_type.clone())
            })
            .or_else(|| self.get_func_return_type(&wrapper_func_id).cloned())
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
    pub(super) fn lower_indirect_call(
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

        // Check if this is a *args forwarding pattern: func(*args) where args is VarPositional.
        // In this case we can't statically unpack the tuple (unknown arity at compile time),
        // so we use a runtime trampoline that dispatches based on tuple length.
        let varargs_tuple_local = self.detect_varargs_forward(args, hir_module, mir_func)?;

        if let Some(args_tuple_local) = varargs_tuple_local {
            // *args forwarding: use runtime trampoline
            return self.lower_indirect_call_with_varargs(func_local, args_tuple_local, mir_func);
        }

        // Non-varargs case: lower arguments normally
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
        let type_tag_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GET_TYPE_TAG),
            vec![mir::Operand::Local(func_local)],
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

    /// Detect if args contain a *args forwarding pattern (func(*varargs_param)).
    /// Returns the tuple local if detected, None otherwise.
    fn detect_varargs_forward(
        &mut self,
        args: &[ExpandedArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Option<pyaot_utils::LocalId>> {
        // Look for exactly one RuntimeUnpackTuple arg that references a VarPositional param
        if args.len() != 1 {
            return Ok(None);
        }
        if let ExpandedArg::RuntimeUnpackTuple(expr_id) = &args[0] {
            let expr = &hir_module.exprs[*expr_id];
            if let hir::ExprKind::Var(var_id) = &expr.kind {
                if self.closures.varargs_params.contains(var_id) {
                    // This is func(*args) where args is a VarPositional param
                    let tuple_operand = self.lower_expr(expr, hir_module, mir_func)?;
                    let tuple_local = match tuple_operand {
                        mir::Operand::Local(local) => local,
                        _ => {
                            let local = self.alloc_and_add_local(Type::Any, mir_func);
                            self.emit_instruction(mir::InstructionKind::Copy {
                                dest: local,
                                src: tuple_operand,
                            });
                            local
                        }
                    };
                    return Ok(Some(tuple_local));
                }
            }
        }
        Ok(None)
    }

    /// Lower an indirect call with *args forwarding via runtime trampoline.
    /// Handles both raw function pointers and closure tuples.
    fn lower_indirect_call_with_varargs(
        &mut self,
        func_local: pyaot_utils::LocalId,
        args_tuple_local: pyaot_utils::LocalId,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let result_ty = mir_func.return_type.clone();
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Check if func is a closure (tuple) or raw function pointer
        let type_tag_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GET_TYPE_TAG),
            vec![mir::Operand::Local(func_local)],
            Type::Int,
            mir_func,
        );

        let is_tuple_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: is_tuple_local,
            op: mir::BinOp::Eq,
            left: mir::Operand::Local(type_tag_local),
            right: mir::Operand::Constant(mir::Constant::Int(TypeTagKind::Tuple.tag() as i64)),
        });

        let closure_bb = self.new_block();
        let direct_bb = self.new_block();
        let merge_bb = self.new_block();
        let closure_id = closure_bb.id;
        let direct_id = direct_bb.id;
        let merge_id = merge_bb.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(is_tuple_local),
            then_block: closure_id,
            else_block: direct_id,
        };

        // === Closure case: extract func_ptr from closure, prepend captures to args ===
        self.push_block(closure_bb);
        {
            // Extract func_ptr from closure tuple index 0
            let real_func = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
                vec![
                    mir::Operand::Local(func_local),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                ],
                Type::Any,
                mir_func,
            );

            // Extract captures tuple from closure tuple index 1
            let captures_tuple = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
                vec![
                    mir::Operand::Local(func_local),
                    mir::Operand::Constant(mir::Constant::Int(1)),
                ],
                Type::Any,
                mir_func,
            );

            // Concatenate captures + args into a single tuple
            let combined = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_CONCAT),
                vec![
                    mir::Operand::Local(captures_tuple),
                    mir::Operand::Local(args_tuple_local),
                ],
                Type::Any,
                mir_func,
            );

            // Call via trampoline with combined args
            let closure_result = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_CALL_WITH_TUPLE_ARGS),
                vec![
                    mir::Operand::Local(real_func),
                    mir::Operand::Local(combined),
                ],
                result_ty.clone(),
                mir_func,
            );

            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(closure_result),
            });
        }
        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // === Direct case: call via trampoline with args tuple ===
        self.push_block(direct_bb);
        {
            let direct_result = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_CALL_WITH_TUPLE_ARGS),
                vec![
                    mir::Operand::Local(func_local),
                    mir::Operand::Local(args_tuple_local),
                ],
                result_ty,
                mir_func,
            );

            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(direct_result),
            });
        }
        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // === Merge ===
        self.push_block(merge_bb);

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
    pub(super) fn emit_closure_call(
        &mut self,
        closure_local: LocalId,
        user_args: Vec<mir::Operand>,
        result_ty: Type,
        mir_func: &mut mir::Function,
    ) -> LocalId {
        // Result local shared across all branches
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Extract func_ptr from index 0
        let func_ptr_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
            vec![
                mir::Operand::Local(closure_local),
                mir::Operand::Constant(mir::Constant::Int(0)),
            ],
            Type::Any,
            mir_func,
        );

        // Extract captures tuple from index 1
        let captures_tuple = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
            vec![
                mir::Operand::Local(closure_local),
                mir::Operand::Constant(mir::Constant::Int(1)),
            ],
            Type::Any,
            mir_func,
        );

        // Get the number of captures
        let n_captures_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_LEN),
            vec![mir::Operand::Local(captures_tuple)],
            Type::Int,
            mir_func,
        );

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
            let cap_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
                vec![
                    mir::Operand::Local(captures_tuple),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                ],
                Type::Any,
                mir_func,
            );
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
