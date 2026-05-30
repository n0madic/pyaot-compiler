//! Closure, wrapper, and indirect call lowering

use pyaot_core_defs::TypeTagKind;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::LocalId;

use crate::context::Lowering;

use super::ExpandedArg;

impl<'a> Lowering<'a> {
    /// Lower a closure call.
    /// Closures have captured variables that need to be prepended to the argument list.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_closure_call(
        &mut self,
        func_id: pyaot_utils::FuncId,
        captures: &[hir::ExprId],
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
        _expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Lower captured expressions first (these are prepended to args).
        // Stage E (unified closure ABI): the callee's primitive capture
        // params are typed `Type::Any` and unbox once in the prologue —
        // Primitive captures are wrapped as tagged Values (ValueFromInt/ValueFromBool)
        // so the direct CallDirect path delivers the same tagged Value bits as
        // the trampoline / HOF dispatcher paths.
        // For cell variables (used by nonlocal), pass the cell pointer directly.
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
            // Box int/bool/float primitives to tagged Value bits; cells and
            // heap objects pass through `emit_value_slot` unchanged.
            let op_type = self.operand_type(&capture_op, mir_func);
            let stored_op = self.emit_value_slot(capture_op, &op_type, mir_func);
            all_args.push(stored_op);
        }

        // Lower regular call arguments. When the function definition is available, use
        // resolve_call_args so that *list unpacking and default parameters are handled
        // correctly. Fall back to lower_expanded_args when the definition is absent.
        let func_def = hir_module.func_defs.get(&func_id);
        let arg_operands = if let Some(func_def) = func_def {
            let n_captures = captures.len();
            let user_params: Vec<hir::Param> =
                func_def.params.iter().skip(n_captures).cloned().collect();
            // Pass `kwargs` through (not `&[]`): calling a capturing closure
            // with keyword arguments (`g = lambda a: a + c; g(a=5)`) must
            // bind them to the user params. `resolve_call_args` maps keyword
            // names against `user_params` (capture params already stripped),
            // so `n_captures` keeps the positional offset correct.
            self.resolve_call_args(
                args,
                kwargs,
                &user_params,
                Some(func_id),
                n_captures,
                self.call_span(),
                hir_module,
                mir_func,
            )?
        } else {
            self.lower_expanded_args(args, hir_module, mir_func)?
        };
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
        // The wrapper expects this as its first argument (the captured 'func' parameter).
        // §P.2.2: wrap as `Value::from_int` so the wrapper's prologue
        // `UnwrapValueInt` recovers the raw text-segment address — same ABI
        // as the closure-tuple trampoline path. Stays off the shadow stack
        // (alloc_stack_local) since the wrapped Value(low bit 1) is_ptr=false.
        // Raw text-segment address: register translation Int → Raw(I64),
        // computed_is_gc_root = false (no GC tracking for code pointers).
        let func_ptr_raw = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::FuncAddr {
            dest: func_ptr_raw,
            func: original_func_id,
        });
        let func_ptr_local =
            self.alloc_and_add_local_with_mir_ty(Type::Any, mir::MirType::Tagged, mir_func);
        self.emit_instruction(mir::InstructionKind::BoxValue {
            dest: func_ptr_local,
            src: mir::Operand::Local(func_ptr_raw),
            src_type: Type::Int,
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
            // No *args — use resolve_call_args so that *list unpacking and default
            // parameters work correctly even in the non-varargs wrapper path.
            let user_arg_operands =
                if let Some(wrapper_def) = hir_module.func_defs.get(&wrapper_func_id) {
                    let user_params: Vec<hir::Param> =
                        wrapper_def.params.iter().skip(1).cloned().collect();
                    self.resolve_call_args(
                        args,
                        kwargs,
                        &user_params,
                        Some(wrapper_func_id),
                        1, // offset for the func-ptr capture param
                        self.call_span(),
                        hir_module,
                        mir_func,
                    )?
                } else {
                    self.lower_expanded_args(args, hir_module, mir_func)?
                };
            all_args.extend(user_arg_operands);
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
            // Stage E.1 (Source-1 fix): prefer the decorated function's return
            // type over the wrapper's own (unannotated) return type. If the
            // wrapper has `return_type = Type::Any` (no annotation) but the
            // original function is typed (e.g. `-> int`), use the original's
            // return type so the trampoline result lands in a Raw (not GC-
            // tracked) slot rather than a `mir_ty: Tagged` slot that would
            // SIGSEGV if GC runs while holding raw primitive bits.
            let result_ty = self
                .get_original_func_return_type_for_wrapper(mir_func.id, hir_module)
                .unwrap_or_else(|| mir_func.return_type.clone());
            return self.lower_indirect_call_with_varargs(
                func_local,
                args_tuple_local,
                result_ty,
                mir_func,
            );
        }

        // Non-varargs case: lower arguments normally
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;
        // Stage E.1 (Source-1 fix): same as the *args path above — prefer
        // the original decorated function's precise return type.
        let result_ty = self
            .get_original_func_return_type_for_wrapper(mir_func.id, hir_module)
            .unwrap_or_else(|| mir_func.return_type.clone());
        let arg_types: Vec<Type> = arg_operands
            .iter()
            .map(|op| self.operand_type(op, mir_func))
            .collect();
        let args_tuple_local = self.create_tuple_from_operands_typed(
            &arg_operands,
            &Type::Any,
            Some(&arg_types),
            mir_func,
        );
        self.lower_indirect_call_with_varargs(func_local, args_tuple_local, result_ty, mir_func)
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
                            // VarPositional *args is a TupleObj heap pointer — tagged.
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
    pub(super) fn lower_indirect_call_with_varargs(
        &mut self,
        func_local: pyaot_utils::LocalId,
        args_tuple_local: pyaot_utils::LocalId,
        result_ty: Type,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
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

        // === Closure case: extract func_ptr from closure, deliver captures
        // and args to the callee ===
        //
        // Stage E: replaces the legacy `rt_tuple_concat` +
        // `rt_call_with_tuple_args` combo. `rt_call_with_captures_and_args`
        // walks captures (tagged Values) and args (raw scalars) separately
        // so each side is unwrapped correctly.
        self.push_block(closure_bb);
        // Extract func_ptr from closure tuple index 0 — stored as a
        // `Value::from_int` tagged Value (§F.5); unwrap to recover the
        // raw i64 function pointer the trampoline expects.
        let tagged_func = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
            vec![
                mir::Operand::Local(func_local),
                mir::Operand::Constant(mir::Constant::Int(0)),
            ],
            Type::Any,
            mir_func,
        );
        let real_func = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::UnboxValue {
            dest: real_func,
            src: mir::Operand::Local(tagged_func),
            dest_type: Type::Int,
        });

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

        // Marker-bit dispatch for return ABI: the trampoline propagates
        // the callee's return verbatim — tagged Value for phase4-safe
        // (return-flipped) callees, raw bits for legacy callees. For
        // primitive `result_ty`, branch at runtime on the marker bit
        // and unbox in the tagged path.
        let primitive_dest = matches!(result_ty, Type::Int | Type::Bool | Type::Float);
        if primitive_dest {
            let marker_val = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: marker_val,
                op: mir::BinOp::BitAnd,
                left: mir::Operand::Local(real_func),
                right: mir::Operand::Constant(mir::Constant::Int(
                    crate::PHASE4_TAGGED_USER_ARGS_MARKER,
                )),
            });
            let marker_bool = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: marker_bool,
                op: mir::BinOp::NotEq,
                left: mir::Operand::Local(marker_val),
                right: mir::Operand::Constant(mir::Constant::Int(0)),
            });
            let tagged_bb = self.new_block();
            let legacy_bb = self.new_block();
            let (tagged_id, legacy_id) = (tagged_bb.id, legacy_bb.id);
            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(marker_bool),
                then_block: tagged_id,
                else_block: legacy_id,
            };
            self.push_block(tagged_bb);
            {
                // Stage D.1 of Strong-Typed MIR Rewrite plan v2: the
                // marker-bit-set trampoline path is the documented
                // "guaranteed Tagged" producer (the runtime trampoline
                // boxes primitive results before returning). Mark the
                // temp as HeapAny (not Any) so downstream passes know
                // this slot can't carry raw bits.
                let any_temp = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_CALL_WITH_CAPTURES_AND_ARGS,
                    ),
                    vec![
                        mir::Operand::Local(real_func),
                        mir::Operand::Local(captures_tuple),
                        mir::Operand::Local(args_tuple_local),
                    ],
                    Type::Any,
                    mir_func,
                );
                self.emit_instruction(mir::InstructionKind::UnboxValue {
                    dest: result_local,
                    src: mir::Operand::Local(any_temp),
                    dest_type: result_ty.clone(),
                });
            }
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);
            self.push_block(legacy_bb);
            {
                let raw_temp = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_CALL_WITH_CAPTURES_AND_ARGS,
                    ),
                    vec![
                        mir::Operand::Local(real_func),
                        mir::Operand::Local(captures_tuple),
                        mir::Operand::Local(args_tuple_local),
                    ],
                    result_ty.clone(),
                    mir_func,
                );
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Local(raw_temp),
                });
            }
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);
        } else {
            let closure_result = self.emit_runtime_call(
                mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_CALL_WITH_CAPTURES_AND_ARGS,
                ),
                vec![
                    mir::Operand::Local(real_func),
                    mir::Operand::Local(captures_tuple),
                    mir::Operand::Local(args_tuple_local),
                ],
                result_ty.clone(),
                mir_func,
            );
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(closure_result),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);
        }

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

        // Extract func_ptr from index 0 — stored as a `Value::from_int`
        // tagged Value (§F.5); unwrap to recover the raw i64 function
        // pointer the trampoline needs. Phase 4: bit 63 may carry the
        // `PHASE4_TAGGED_USER_ARGS_MARKER`; the trampoline reads it and
        // masks it back out before invoking, so we pass the raw value
        // through verbatim.
        let tagged_func = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
            vec![
                mir::Operand::Local(closure_local),
                mir::Operand::Constant(mir::Constant::Int(0)),
            ],
            Type::Any,
            mir_func,
        );
        let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::UnboxValue {
            dest: func_ptr_local,
            src: mir::Operand::Local(tagged_func),
            dest_type: Type::Int,
        });

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
        // Build args tuple from user args (boxes primitives via
        // `emit_value_slot` per the established convention).
        let arg_types: Vec<Type> = user_args
            .iter()
            .map(|op| self.operand_type(op, mir_func))
            .collect();
        let args_tuple = self.create_tuple_from_operands_typed(
            &user_args,
            &Type::Any,
            Some(&arg_types),
            mir_func,
        );

        // Phase 4 (Storage-Uniform): route through the runtime trampoline
        // `rt_call_with_captures_and_args`. The trampoline reads the
        // `PHASE4_TAGGED_USER_ARGS_MARKER` bit on `func_ptr_local` and
        // dispatches user-arg extraction accordingly (tagged ABI for
        // phase4_safe lambdas, raw legacy ABI otherwise). It then
        // returns whatever the callee returns verbatim — tagged Value
        // bits for phase4-safe callees (return-flipped), raw primitive
        // bits for legacy callees (not flipped).
        //
        // Return ABI: when the caller wants a primitive `result_ty`,
        // dispatch on the marker bit at runtime — if set, the callee
        // returned tagged bits and we must `UnboxValue`; if unset,
        // the callee returned raw bits and we propagate them directly
        // into the primitive dest. This makes both shapes safe through
        // the same call site without static knowledge of the callee.
        let primitive_dest = matches!(result_ty, Type::Int | Type::Bool | Type::Float);

        if primitive_dest {
            // Compute `(func_ptr & MARKER) != 0` → marker_bool.
            let marker_val = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: marker_val,
                op: mir::BinOp::BitAnd,
                left: mir::Operand::Local(func_ptr_local),
                right: mir::Operand::Constant(mir::Constant::Int(
                    crate::PHASE4_TAGGED_USER_ARGS_MARKER,
                )),
            });
            let marker_bool = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: marker_bool,
                op: mir::BinOp::NotEq,
                left: mir::Operand::Local(marker_val),
                right: mir::Operand::Constant(mir::Constant::Int(0)),
            });

            let tagged_bb = self.new_block();
            let legacy_bb = self.new_block();
            let merge_bb = self.new_block();
            let (tagged_id, legacy_id, merge_id) = (tagged_bb.id, legacy_bb.id, merge_bb.id);

            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(marker_bool),
                then_block: tagged_id,
                else_block: legacy_id,
            };

            // Tagged branch: trampoline returns tagged Value → unbox.
            // Stage D.1: temp local typed HeapAny — guaranteed Tagged
            // bit pattern when the marker-bit dispatched here.
            self.push_block(tagged_bb);
            {
                let any_temp = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_CALL_WITH_CAPTURES_AND_ARGS,
                    ),
                    vec![
                        mir::Operand::Local(func_ptr_local),
                        mir::Operand::Local(captures_tuple),
                        mir::Operand::Local(args_tuple),
                    ],
                    Type::Any,
                    mir_func,
                );
                self.emit_instruction(mir::InstructionKind::UnboxValue {
                    dest: result_local,
                    src: mir::Operand::Local(any_temp),
                    dest_type: result_ty.clone(),
                });
            }
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

            // Legacy branch: trampoline returns raw bits → assign verbatim.
            self.push_block(legacy_bb);
            {
                let raw_temp = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_CALL_WITH_CAPTURES_AND_ARGS,
                    ),
                    vec![
                        mir::Operand::Local(func_ptr_local),
                        mir::Operand::Local(captures_tuple),
                        mir::Operand::Local(args_tuple),
                    ],
                    result_ty.clone(),
                    mir_func,
                );
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Local(raw_temp),
                });
            }
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

            self.push_block(merge_bb);
        } else {
            // Non-primitive dest: trampoline returns the callee's tagged
            // Value verbatim (or pointer for heap types). Copy through.
            let trampoline_result = self.emit_runtime_call(
                mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_CALL_WITH_CAPTURES_AND_ARGS,
                ),
                vec![
                    mir::Operand::Local(func_ptr_local),
                    mir::Operand::Local(captures_tuple),
                    mir::Operand::Local(args_tuple),
                ],
                result_ty,
                mir_func,
            );
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(trampoline_result),
            });
        }

        result_local
    }
}
