//! Composite iteration lowering: zip(), map(), filter(), reduce(), captures

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::{FuncOrBuiltin, Lowering};

impl<'a> Lowering<'a> {
    /// Lower zip(iter1, iter2, ...) - create a zip iterator
    /// Returns an iterator that yields tuples from all iterables
    pub(in crate::expressions::builtins) fn lower_zip(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // zip() with no args returns an empty iterator
            let result_local = self
                .alloc_and_add_local(Type::Iterator(Box::new(Type::tuple_of(vec![]))), mir_func);
            // Create empty tuple iterator
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_LIST),
                args: vec![mir::Operand::Constant(mir::Constant::Int(0))],
            });
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(
                    mir::IterSourceKind::List.iterator_def(mir::IterDirection::Forward),
                ),
                args: vec![mir::Operand::Local(result_local)],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        // Handle 3+ arguments separately
        if args.len() == 3 {
            // Use Zip3New for exactly 3 iterables
            let mut iter_locals = Vec::new();
            let mut elem_types = Vec::new();

            for arg_id in args {
                let (iter_local, elem_type) =
                    self.make_iter_from_expr(*arg_id, hir_module, mir_func)?;
                iter_locals.push(mir::Operand::Local(iter_local));
                elem_types.push(elem_type);
            }

            let result_local = self.emit_runtime_call_gc(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ZIP3_NEW),
                iter_locals,
                Type::Iterator(Box::new(Type::tuple_of(elem_types))),
                mir_func,
            );

            return Ok(mir::Operand::Local(result_local));
        } else if args.len() > 3 {
            // Use ZipNNew for 4+ iterables
            let mut iter_locals = Vec::new();
            let mut elem_types = Vec::new();

            for arg_id in args {
                let (iter_local, elem_type) =
                    self.make_iter_from_expr(*arg_id, hir_module, mir_func)?;
                iter_locals.push(mir::Operand::Local(iter_local));
                elem_types.push(elem_type);
            }

            // Create a list of iterators
            let count = args.len() as i64;
            let iter_list_local = self
                .alloc_and_add_local(Type::list_of(Type::Iterator(Box::new(Type::Any))), mir_func);

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: iter_list_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_LIST),
                args: vec![mir::Operand::Constant(mir::Constant::Int(count))],
            });

            // Push each iterator to the list
            for (i, iter_op) in iter_locals.iter().enumerate() {
                self.emit_runtime_call_void(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_SET),
                    vec![
                        mir::Operand::Local(iter_list_local),
                        mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        iter_op.clone(),
                    ],
                    mir_func,
                );
            }

            let result_local = self.emit_runtime_call_gc(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ZIPN_NEW),
                vec![
                    mir::Operand::Local(iter_list_local),
                    mir::Operand::Constant(mir::Constant::Int(count)),
                ],
                Type::Iterator(Box::new(Type::tuple_of(elem_types))),
                mir_func,
            );

            return Ok(mir::Operand::Local(result_local));
        }

        // Get first iterable and create iterator
        let first_expr = &hir_module.exprs[args[0]];

        // Check if first is a range() call - needs special handling
        let first_is_range = matches!(
            &first_expr.kind,
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                ..
            }
        );

        let first_type = self.seed_expr_type(args[0], hir_module);
        let first_elem_type = if first_is_range {
            Type::Int
        } else {
            crate::type_planning::infer::extract_iterable_first_element_type(&first_type)
        };

        // Create first iterator
        let first_iter_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(first_elem_type.clone())), mir_func);

        // Handle range() specially for first iterator
        if first_is_range {
            if let hir::ExprKind::BuiltinCall {
                args: range_args, ..
            } = &first_expr.kind
            {
                let (start, stop, step) =
                    self.parse_range_args(range_args, hir_module, mir_func)?;
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: first_iter_local,
                    func: mir::RuntimeFunc::Call(
                        mir::IterSourceKind::Range.iterator_def(mir::IterDirection::Forward),
                    ),
                    args: vec![start, stop, step],
                });
            }
        } else {
            let first_operand =
                self.lower_expr_expecting(first_expr, None, hir_module, mir_func)?;
            let first_source = if first_type.is_list_like() {
                mir::IterSourceKind::List
            } else if first_type.is_tuple_like() {
                mir::IterSourceKind::Tuple
            } else if first_type.is_dict_like() {
                mir::IterSourceKind::Dict
            } else if first_type.is_set_like() {
                mir::IterSourceKind::Set
            } else {
                match &first_type {
                    Type::Str => mir::IterSourceKind::Str,
                    Type::Bytes => mir::IterSourceKind::Bytes,
                    Type::Iterator(_) => mir::IterSourceKind::Generator,
                    _ => mir::IterSourceKind::List,
                }
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: first_iter_local,
                func: mir::RuntimeFunc::Call(
                    first_source.iterator_def(mir::IterDirection::Forward),
                ),
                args: vec![first_operand],
            });
        }

        // If only one argument, just return its iterator (unusual but valid)
        if args.len() == 1 {
            return Ok(mir::Operand::Local(first_iter_local));
        }

        // Get second iterable and create iterator
        let second_expr = &hir_module.exprs[args[1]];

        // Check if second is a range() call - needs special handling
        let second_is_range = matches!(
            &second_expr.kind,
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                ..
            }
        );

        let second_type = self.seed_expr_type(args[1], hir_module);
        let second_elem_type = if second_is_range {
            Type::Int
        } else {
            crate::type_planning::infer::extract_iterable_first_element_type(&second_type)
        };

        // Create second iterator
        let second_iter_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(second_elem_type.clone())), mir_func);

        // Handle range() specially for second iterator
        if second_is_range {
            if let hir::ExprKind::BuiltinCall {
                args: range_args, ..
            } = &second_expr.kind
            {
                let (start, stop, step) =
                    self.parse_range_args(range_args, hir_module, mir_func)?;
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: second_iter_local,
                    func: mir::RuntimeFunc::Call(
                        mir::IterSourceKind::Range.iterator_def(mir::IterDirection::Forward),
                    ),
                    args: vec![start, stop, step],
                });
            }
        } else {
            let second_operand =
                self.lower_expr_expecting(second_expr, None, hir_module, mir_func)?;
            let second_source = if second_type.is_list_like() {
                mir::IterSourceKind::List
            } else if second_type.is_tuple_like() {
                mir::IterSourceKind::Tuple
            } else if second_type.is_dict_like() {
                mir::IterSourceKind::Dict
            } else if second_type.is_set_like() {
                mir::IterSourceKind::Set
            } else {
                match &second_type {
                    Type::Str => mir::IterSourceKind::Str,
                    Type::Bytes => mir::IterSourceKind::Bytes,
                    Type::Iterator(_) => mir::IterSourceKind::Generator,
                    _ => mir::IterSourceKind::List,
                }
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: second_iter_local,
                func: mir::RuntimeFunc::Call(
                    second_source.iterator_def(mir::IterDirection::Forward),
                ),
                args: vec![second_operand],
            });
        }

        // Create zip iterator
        let result_local = self.emit_runtime_call_gc(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ZIP_NEW),
            vec![
                mir::Operand::Local(first_iter_local),
                mir::Operand::Local(second_iter_local),
            ],
            Type::Iterator(Box::new(Type::tuple_of(vec![
                first_elem_type,
                second_elem_type,
            ]))),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower map(func, iterable) - create iterator that applies func to each element
    /// Supports closures with captures - captures are stored in a tuple and passed to runtime
    /// Also supports first-class builtins (len, str, int, etc.)
    pub(in crate::expressions::builtins) fn lower_map(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        if args.len() < 2 {
            return Err(CompilerError::codegen_error(
                "map() requires at least 2 arguments",
                None,
            ));
        }

        // Extract function or builtin from first argument
        let func_expr = &hir_module.exprs[args[0]];
        let func_or_builtin = self
            .extract_func_or_builtin(func_expr, hir_module)
            .ok_or_else(|| {
                CompilerError::codegen_error(
                    "map() first argument must be a function",
                    Some(func_expr.span),
                )
            })?;

        // Phase 4+ Extension E2d: route to the tagged-delivery runtime
        // variant when the callback is a phase4-safe user lambda.
        // Builtin callbacks stay on legacy (they expect tagged Values
        // anyway via their own dispatchers and have no prologue to
        // worry about).
        let phase4_safe_callback = matches!(&func_or_builtin,
            FuncOrBuiltin::UserFunc(fid, _) if self.is_phase4_safe(*fid));

        // Get function pointer and captures based on whether it's a user function or builtin
        let (func_ptr_operand, captures_operand, capture_count, result_elem_type) =
            match func_or_builtin {
                FuncOrBuiltin::UserFunc(func_id, captures) => {
                    // Record capture types for inline closures so lambda type inference works correctly.
                    if !captures.is_empty() && !self.has_closure_capture_types(&func_id) {
                        let mut capture_types = Vec::new();
                        for capture_id in &captures {
                            let capture_type = self.seed_expr_type(*capture_id, hir_module);
                            capture_types.push(capture_type);
                        }
                        self.insert_closure_capture_types(func_id, capture_types);
                    }

                    // Get function address
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: func_ptr_local,
                        func: func_id,
                    });

                    // After §F.7c BigBang: encode elem_unbox_kind in bits 8-15 and
                    // result_box_kind in bits 16-23 so iter_next_map unboxes tagged
                    // Values for typed Int/Bool elem params and re-tags the lambda's
                    // raw scalar return so callers receive uniform tagged bits.
                    let elem_unbox_kind =
                        self.callback_elem_unbox_kind(func_id, captures.len(), hir_module);

                    // Determine result element type from the callback function's return type
                    let result_type = self.infer_callback_return_type(func_id, hir_module);
                    // Phase 4+ Extension E2d: when the callback is a regular
                    // function whose return ABI was flipped by Phase 4
                    // Commit 4, the callback ALREADY returns a tagged Value
                    // — re-wrapping in the runtime would double-tag. Set
                    // `result_box_kind = 0` (pass-through) in that case.
                    // Lambdas are never return-flipped, so they keep the
                    // raw-→-tagged re-wrap path.
                    let callback_return_already_tagged =
                        self.is_return_abi_flipped(func_id, hir_module);
                    let result_box_kind = if callback_return_already_tagged {
                        0i64
                    } else {
                        match &result_type {
                            Type::Int => 1i64,
                            Type::Bool => 2i64,
                            _ => 0i64,
                        }
                    };
                    let encoding = (result_box_kind << 16) | (elem_unbox_kind << 8);

                    // Lower captures to a tuple (if any)
                    let (cap_op, cap_count) = if captures.is_empty() {
                        (
                            mir::Operand::Constant(mir::Constant::Int(0)), // null pointer
                            mir::Operand::Constant(mir::Constant::Int(encoding)),
                        )
                    } else {
                        let captures_tuple = self.lower_captures_to_tuple_for(
                            Some(func_id),
                            &captures,
                            hir_module,
                            mir_func,
                        )?;
                        let count = captures.len() as i64;
                        (
                            captures_tuple,
                            mir::Operand::Constant(mir::Constant::Int(encoding | count)),
                        )
                    };

                    (
                        mir::Operand::Local(func_ptr_local),
                        cap_op,
                        cap_count,
                        result_type,
                    )
                }
                FuncOrBuiltin::Builtin(builtin_kind) => {
                    // Get builtin function pointer from runtime table
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                        dest: func_ptr_local,
                        builtin: builtin_kind,
                    });

                    // Builtins accept *mut Obj (tagged Value bits) — pass through.
                    // Bit 7 of low byte is the legacy needs_boxing flag (no-op now
                    // since iter_next_* return tagged Values; kept to avoid ABI churn).
                    let cap_op = mir::Operand::Constant(mir::Constant::Int(0));
                    let cap_count = mir::Operand::Constant(mir::Constant::Int(0x80));

                    // Infer result type based on builtin
                    let result_type = self.infer_builtin_return_type(builtin_kind);

                    (
                        mir::Operand::Local(func_ptr_local),
                        cap_op,
                        cap_count,
                        result_type,
                    )
                }
            };

        // Create iterator from second argument
        let iter_args = &args[1..2];
        let inner_iter = self.lower_iter(iter_args, hir_module, mir_func)?;

        let map_new_def: &pyaot_core_defs::RuntimeFuncDef = if phase4_safe_callback {
            &pyaot_core_defs::runtime_func_def::RT_MAP_NEW_TAGGED
        } else {
            &pyaot_core_defs::runtime_func_def::RT_MAP_NEW
        };
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(map_new_def),
            vec![
                func_ptr_operand,
                inner_iter,
                captures_operand,
                capture_count,
            ],
            Type::Iterator(Box::new(result_elem_type)),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Infer the return type of a builtin function
    fn infer_builtin_return_type(&self, builtin: mir::BuiltinFunctionKind) -> Type {
        match builtin {
            mir::BuiltinFunctionKind::Len => Type::Int,
            mir::BuiltinFunctionKind::Str => Type::Str,
            mir::BuiltinFunctionKind::Int => Type::Int,
            mir::BuiltinFunctionKind::Float => Type::Float,
            mir::BuiltinFunctionKind::Bool => Type::Bool,
            mir::BuiltinFunctionKind::Abs => Type::Any, // Could be Int or Float
            mir::BuiltinFunctionKind::Hash => Type::Int,
            mir::BuiltinFunctionKind::Ord => Type::Int,
            mir::BuiltinFunctionKind::Chr => Type::Str,
            mir::BuiltinFunctionKind::Repr => Type::Str,
            mir::BuiltinFunctionKind::Type => Type::Str,
        }
    }

    /// Lower captured expressions to a tuple
    /// Used by map/filter/reduce/sorted-key= to store HOF callback captures
    /// at runtime.
    /// §P.2.2 variant: takes the destination FuncId so wrapper-style fn-ptr
    /// captures can be `ValueFromInt`-wrapped at the producer (matching the
    /// callee's prologue `UnwrapValueInt`).
    pub(crate) fn lower_captures_to_tuple_for(
        &mut self,
        target_func: Option<pyaot_utils::FuncId>,
        captures: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let count = captures.len();

        // Lower all capture expressions first to know their operand types
        let mut capture_operands = Vec::with_capacity(count);
        for capture_id in captures {
            let capture_expr = &hir_module.exprs[*capture_id];
            capture_operands.push(self.lower_expr(capture_expr, hir_module, mir_func)?);
        }

        // After §F.7c: tuples store uniform tagged Values; no elem_tag needed.
        let tuple_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
            vec![mir::Operand::Constant(mir::Constant::Int(count as i64))],
            Type::tuple_of(vec![Type::Any; count]),
            mir_func,
        );

        // Box primitives / wrap fn-ptr captures before storing so retrieval
        // delivers tagged Value bits to the lambda's prologue unbox.
        let fn_ptr_idx = target_func.and_then(|f| self.wrapper_fn_ptr_capture_index(f, hir_module));
        for (i, capture_operand) in capture_operands.into_iter().enumerate() {
            let stored_op = if Some(i) == fn_ptr_idx {
                let wrapped =
                    self.alloc_and_add_local_with_mir_ty(Type::Any, mir::MirType::Tagged, mir_func);
                self.emit_instruction(mir::InstructionKind::BoxValue {
                    dest: wrapped,
                    src: capture_operand,
                    src_type: Type::Int,
                });
                mir::Operand::Local(wrapped)
            } else {
                let op_type = self.operand_type(&capture_operand, mir_func);
                self.emit_value_slot(capture_operand, &op_type, mir_func)
            };
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: tuple_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
                args: vec![
                    mir::Operand::Local(tuple_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    stored_op,
                ],
            });
        }

        Ok(mir::Operand::Local(tuple_local))
    }

    /// Lower reduce(func, iterable, initial?) - fold iterable to single value
    /// Follows the same pattern as map/filter for callable extraction
    pub(in crate::expressions::builtins) fn lower_reduce(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        if args.len() < 2 || args.len() > 3 {
            return Err(CompilerError::codegen_error(
                "reduce() requires 2 or 3 arguments",
                None,
            ));
        }

        // Note: lambda param type hints for reduce are registered during the
        // precompute_closure_capture_types phase (in scan_expr_for_closures)
        // since lambdas are lowered before the module init function.

        // Extract function or builtin from first argument
        let func_expr = &hir_module.exprs[args[0]];
        let func_or_builtin = self
            .extract_func_or_builtin(func_expr, hir_module)
            .ok_or_else(|| {
                CompilerError::codegen_error(
                    "reduce() first argument must be a function",
                    Some(func_expr.span),
                )
            })?;

        // Extract func_id before consuming func_or_builtin (for result type inference)
        let reduce_func_id = match &func_or_builtin {
            FuncOrBuiltin::UserFunc(func_id, _) => Some(*func_id),
            FuncOrBuiltin::Builtin(_) => None,
        };

        // Phase 4+ Extension E2d: route to tagged-delivery runtime
        // variant when the callback is a phase4-safe user lambda.
        let phase4_safe_callback = reduce_func_id.is_some_and(|fid| self.is_phase4_safe(fid));

        // Compute result_box_kind early — needed both for the capture_count
        // encoding AND for the initial-value boxing decision below.
        //   0 = passthrough (Any/heap return; callback already returns tagged or pass-through)
        //   1 = Int (callback returns raw i64; rt_reduce_tagged rewraps to tagged)
        //   2 = Bool (callback returns raw i8; rt_reduce_tagged rewraps to tagged)
        let result_box_kind: i64 = if let Some(fid) = reduce_func_id {
            if self.is_return_abi_flipped(fid, hir_module) {
                0
            } else {
                match self.get_func_return_type(&fid) {
                    Some(Type::Int) => 1,
                    Some(Type::Bool) => 2,
                    _ => 0,
                }
            }
        } else {
            0
        };

        // Get function pointer and captures
        let (func_ptr_operand, captures_operand, capture_count) = match func_or_builtin {
            FuncOrBuiltin::UserFunc(func_id, captures) => {
                // Record capture types for inline closures
                if !captures.is_empty() && !self.has_closure_capture_types(&func_id) {
                    let mut capture_types = Vec::new();
                    for capture_id in &captures {
                        let capture_type = self.seed_expr_type(*capture_id, hir_module);
                        capture_types.push(capture_type);
                    }
                    self.insert_closure_capture_types(func_id, capture_types);
                }

                // Get function address
                let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::FuncAddr {
                    dest: func_ptr_local,
                    func: func_id,
                });

                // After §F.7c BigBang: lambda's elem param (params[capture_count+1])
                // is the iter element. Encode its unbox kind in bits 8-15 so
                // rt_reduce unboxes tagged Values for typed Int/Bool params.
                // Phase 4+ Extension E2d: also encode `result_box_kind` in
                // bits 16-23 so the tagged `rt_reduce_tagged` variant can
                // re-wrap the callback's raw primitive return into a
                // tagged Value (lambdas are not return-ABI flipped). For
                // regular phase4-return-flipped callbacks the return is
                // already tagged — use 0 to skip the double-wrap.
                let elem_unbox_kind =
                    self.callback_elem_unbox_kind(func_id, captures.len() + 1, hir_module);
                let encoding = (result_box_kind << 16) | (elem_unbox_kind << 8);

                // Lower captures to a tuple (if any)
                let (cap_op, cap_count) = if captures.is_empty() {
                    (
                        mir::Operand::Constant(mir::Constant::Int(0)),
                        mir::Operand::Constant(mir::Constant::Int(encoding)),
                    )
                } else {
                    let captures_tuple = self.lower_captures_to_tuple_for(
                        Some(func_id),
                        &captures,
                        hir_module,
                        mir_func,
                    )?;
                    let count = captures.len() as i64;
                    (
                        captures_tuple,
                        mir::Operand::Constant(mir::Constant::Int(encoding | count)),
                    )
                };

                (mir::Operand::Local(func_ptr_local), cap_op, cap_count)
            }
            FuncOrBuiltin::Builtin(builtin_kind) => {
                let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                    dest: func_ptr_local,
                    builtin: builtin_kind,
                });

                (
                    mir::Operand::Local(func_ptr_local),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                )
            }
        };

        // Create iterator from second argument
        let iter_args = &args[1..2];
        let inner_iter = self.lower_iter(iter_args, hir_module, mir_func)?;

        // Infer result type from the callback's return type (needed before
        // initial value handling to determine result_box_kind).
        let result_type = reduce_func_id
            .and_then(|fid| self.get_func_return_type(&fid).cloned())
            .unwrap_or(Type::Any);

        // Get initial value (third argument, if provided).
        //
        // `rt_reduce_tagged` is the phase4-safe variant. Its runtime loop
        // carries the accumulator as a tagged Value:
        //   - For result_box_kind=1/2 (Int/Bool return): `rewrap` wraps the
        //     raw return bits into a tagged Value each iteration. The initial
        //     must arrive as raw bits so `rewrap` tags it correctly on the
        //     first iteration.
        //   - For result_box_kind=0 (passthrough / Any return): `rewrap` is a
        //     no-op. The initial must already be a tagged Value so the
        //     accumulator is tagged end-to-end. Box it here via `emit_value_slot`.
        //
        // `rt_reduce` (non-tagged path) always receives raw bits; no boxing.
        let (initial_operand, has_initial) = if args.len() > 2 {
            let initial_expr = &hir_module.exprs[args[2]];
            let initial_op = self.lower_expr(initial_expr, hir_module, mir_func)?;
            let initial_ty = self.operand_type(&initial_op, mir_func);
            // Box the initial value only when routing to rt_reduce_tagged with
            // result_box_kind=0 (passthrough); otherwise pass raw bits.
            let needs_boxing =
                phase4_safe_callback && result_box_kind == 0 && !matches!(initial_ty, Type::Any);
            let final_initial_op = if needs_boxing {
                self.emit_value_slot(initial_op, &initial_ty, mir_func)
            } else {
                initial_op
            };
            (
                final_initial_op,
                mir::Operand::Constant(mir::Constant::Int(1)),
            )
        } else {
            (
                mir::Operand::Constant(mir::Constant::Int(0)),
                mir::Operand::Constant(mir::Constant::Int(0)),
            )
        };
        // Phase 4+ Extension E2d: route to tagged-delivery variant for
        // phase4-safe lambdas. The tagged variant re-wraps callback's
        // raw primitive return into tagged Value per `result_box_kind`
        // encoded above, so the accumulator stays uniformly tagged
        // across iterations.
        let reduce_def: &pyaot_core_defs::RuntimeFuncDef = if phase4_safe_callback {
            &pyaot_core_defs::runtime_func_def::RT_REDUCE_TAGGED
        } else {
            &pyaot_core_defs::runtime_func_def::RT_REDUCE
        };
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(reduce_def),
            vec![
                func_ptr_operand,
                inner_iter,
                initial_operand,
                has_initial,
                captures_operand,
                capture_count,
            ],
            result_type,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower filter(func, iterable) - create iterator that yields elements where func returns true
    /// When func is None, filter by truthiness (filter out falsy values)
    /// Supports closures with captures - captures are stored in a tuple and passed to runtime
    /// Also supports first-class builtins (bool, etc.)
    pub(in crate::expressions::builtins) fn lower_filter(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        if args.len() < 2 {
            return Err(CompilerError::codegen_error(
                "filter() requires at least 2 arguments",
                None,
            ));
        }

        // Check if first argument is None (truthiness filtering)
        let func_expr = &hir_module.exprs[args[0]];
        let is_none_predicate = matches!(func_expr.kind, hir::ExprKind::None);

        // Phase 4+ Extension E2d: tagged-delivery routing detection. We
        // need to know the callback's func_id BEFORE we move it into the
        // match below, so do an early extract for the safety check.
        //
        // `rt_filter_new_tagged` calls the predicate as `-> i64` (tagged
        // Value) and then passes the result through `rt_is_truthy`. This
        // is correct ONLY when the callback is BOTH:
        //   (a) phase4_safe: receives tagged-Value user args (with prologue
        //       unbox in its body), AND
        //   (b) return-ABI-flipped: boxes its Bool/Int/Float return value
        //       into a tagged Value before returning.
        //
        // If the callback is phase4_safe but NOT return-flipped (no explicit
        // return type annotation), it returns raw i8 (0 or 1). Interpreting
        // that raw i8 as a tagged Value gives Value(1) = Int(0) = falsy for
        // `true` — causing the filter to always return empty. Fall back to
        // the legacy `rt_filter_new` path in that case.
        let phase4_safe_callback = if is_none_predicate {
            false
        } else {
            self.extract_func_or_builtin(func_expr, hir_module)
                .as_ref()
                .is_some_and(|fob| match fob {
                    FuncOrBuiltin::UserFunc(fid, _) => {
                        self.is_phase4_safe(*fid) && self.is_return_abi_flipped(*fid, hir_module)
                    }
                    FuncOrBuiltin::Builtin(_) => false,
                })
        };

        // Get function pointer operand and captures
        let (func_ptr_operand, captures_operand, capture_count) = if is_none_predicate {
            // Use 0 to indicate truthiness filtering, no captures
            (
                mir::Operand::Constant(mir::Constant::Int(0)),
                mir::Operand::Constant(mir::Constant::Int(0)), // null pointer
                mir::Operand::Constant(mir::Constant::Int(0)),
            )
        } else {
            // Extract function or builtin from first argument
            let func_or_builtin = self
                .extract_func_or_builtin(func_expr, hir_module)
                .ok_or_else(|| {
                    CompilerError::codegen_error(
                        "filter() first argument must be a function or None",
                        Some(func_expr.span),
                    )
                })?;

            match func_or_builtin {
                FuncOrBuiltin::UserFunc(func_id, captures) => {
                    // Record capture types for inline closures
                    if !captures.is_empty() && !self.has_closure_capture_types(&func_id) {
                        let mut capture_types = Vec::new();
                        for capture_id in &captures {
                            let capture_type = self.seed_expr_type(*capture_id, hir_module);
                            capture_types.push(capture_type);
                        }
                        self.insert_closure_capture_types(func_id, capture_types);
                    }

                    // Get function address
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: func_ptr_local,
                        func: func_id,
                    });

                    // After §F.7c BigBang: encode elem_unbox_kind in bits 8-15.
                    // When the callback is phase4_safe, it receives tagged
                    // Values and unboxes them itself in its prologue — the
                    // runtime must NOT pre-unbox the element (elem_unbox_kind
                    // must be 0). For legacy (phase4_unsafe) callbacks, the
                    // runtime pre-unboxes before calling.
                    let elem_unbox_kind = if self.is_phase4_safe(func_id) {
                        0
                    } else {
                        self.callback_elem_unbox_kind(func_id, captures.len(), hir_module)
                    };

                    // Lower captures to a tuple (if any)
                    let (cap_op, cap_count) = if captures.is_empty() {
                        (
                            mir::Operand::Constant(mir::Constant::Int(0)),
                            mir::Operand::Constant(mir::Constant::Int(elem_unbox_kind << 8)),
                        )
                    } else {
                        let captures_tuple = self.lower_captures_to_tuple_for(
                            Some(func_id),
                            &captures,
                            hir_module,
                            mir_func,
                        )?;
                        let count = captures.len() as i64;
                        (
                            captures_tuple,
                            mir::Operand::Constant(mir::Constant::Int(
                                (elem_unbox_kind << 8) | count,
                            )),
                        )
                    };

                    (mir::Operand::Local(func_ptr_local), cap_op, cap_count)
                }
                FuncOrBuiltin::Builtin(builtin_kind) => {
                    // Get builtin function pointer from runtime table
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                        dest: func_ptr_local,
                        builtin: builtin_kind,
                    });

                    // Builtins have no captures and accept tagged Values directly.
                    (
                        mir::Operand::Local(func_ptr_local),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                    )
                }
            }
        };

        // Create iterator from second argument
        let iter_args = &args[1..2];
        let inner_iter = self.lower_iter(iter_args, hir_module, mir_func)?;

        // Element type is same as input iterator
        let iterable_type = self.seed_expr_type(args[1], hir_module);
        let elem_type =
            crate::type_planning::infer::extract_iterable_first_element_type(&iterable_type);

        // After §F.7c: containers store uniform tagged Values; rt_filter no
        // longer needs an elem_tag hint.
        let filter_new_def: &pyaot_core_defs::RuntimeFuncDef = if phase4_safe_callback {
            &pyaot_core_defs::runtime_func_def::RT_FILTER_NEW_TAGGED
        } else {
            &pyaot_core_defs::runtime_func_def::RT_FILTER_NEW
        };
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(filter_new_def),
            vec![
                func_ptr_operand,
                inner_iter,
                captures_operand,
                capture_count,
            ],
            Type::Iterator(Box::new(elem_type)),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }
}
