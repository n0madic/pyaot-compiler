//! Min/max functions lowering: min(), max(), min/max on containers and ranges

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir::{self as mir, ContainerKind, ElementKind, MinMaxOp};
use pyaot_types::Type;

use crate::context::{FuncOrBuiltin, Lowering};

impl<'a> Lowering<'a> {
    /// Lower min() or max() builtin.
    /// The is_min parameter controls the comparison: true = Lt (min), false = Gt (max).
    pub(in crate::expressions::builtins) fn lower_minmax_builtin(
        &mut self,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
        is_min: bool,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!(
                    "{}() requires at least 1 argument",
                    if is_min { "min" } else { "max" }
                ),
                self.call_span(),
            ));
        }

        // Extract key kwarg if provided (supports both user functions and builtins)
        let mut key_func: Option<FuncOrBuiltin> = None;
        for kwarg in kwargs {
            let name = self.resolve(kwarg.name);
            if name == "key" {
                let kwarg_expr = &hir_module.exprs[kwarg.value];
                // key=None is a no-op
                if !matches!(kwarg_expr.kind, hir::ExprKind::None) {
                    key_func = self.extract_func_or_builtin(kwarg_expr, hir_module);
                }
            }
        }

        // Validate: CPython doesn't allow key= with multiple arguments
        if key_func.is_some() && args.len() > 1 {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!(
                    "{}() with key= requires exactly one iterable argument",
                    if is_min { "min" } else { "max" }
                ),
                self.call_span(),
            ));
        }

        // Area C §C.3 extension: dispatch through `__lt__` / `__gt__` when
        // the iterable's elements are a user class. Falls through to the
        // numeric fast path below for primitive elements.
        if key_func.is_none() && args.len() == 1 {
            if let Some(result) =
                self.try_lower_minmax_class_elem(args[0], is_min, hir_module, mir_func)?
            {
                return Ok(result);
            }
        }

        if args.len() == 1 {
            // Single argument - check if it's an iterable (list, tuple, set, or range)
            let arg_expr = &hir_module.exprs[args[0]];
            let arg_type = self.get_type_of_expr_id(args[0], hir_module);

            // Check if it's a range() call - handle specially
            if let hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                args: range_args,
                ..
            } = &arg_expr.kind
            {
                return self.lower_minmax_range(range_args, hir_module, mir_func, is_min);
            }

            // If it's a list, call runtime function to find min/max element
            if let Type::List(elem_type) = &arg_type {
                let list_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;

                let op = if is_min { MinMaxOp::Min } else { MinMaxOp::Max };

                // If key function provided, use with_key variant
                if let Some(ref func_or_builtin) = key_func {
                    use crate::context::KeyFuncSource;
                    let key_source = match func_or_builtin {
                        FuncOrBuiltin::UserFunc(func_id, captures) => {
                            KeyFuncSource::UserFunc(*func_id, captures.clone())
                        }
                        FuncOrBuiltin::Builtin(builtin_kind) => {
                            KeyFuncSource::Builtin(*builtin_kind)
                        }
                    };
                    let resolved = self
                        .emit_key_func_with_captures(Some(&key_source), hir_module, mir_func)?
                        .expect("key_source is Some");

                    // Determine elem_tag for boxing raw elements before calling key function.
                    // Only builtin wrappers need boxing - user functions work with raw values.
                    let elem_tag =
                        Self::elem_tag_for_func_or_builtin(func_or_builtin, elem_type.as_ref());
                    let elem_tag_operand = mir::Operand::Constant(mir::Constant::Int(elem_tag));

                    // Result type matches element type (for heap types, use heap object type)
                    let result_type = elem_type.as_ref().clone();
                    let is_min_operand =
                        mir::Operand::Constant(mir::Constant::Int(op.to_tag() as i64));
                    let result_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(ContainerKind::List.minmax_with_key_def()),
                        vec![
                            list_operand,
                            resolved.func_addr,
                            elem_tag_operand,
                            resolved.captures,
                            resolved.capture_count,
                            is_min_operand,
                        ],
                        result_type,
                        mir_func,
                    );

                    return Ok(mir::Operand::Local(result_local));
                }

                // Original logic for non-key case
                // Determine if element type is float
                let is_float_list = matches!(elem_type.as_ref(), Type::Float);
                let (result_type, elem_kind) = if is_float_list {
                    (Type::Float, ElementKind::Float)
                } else {
                    (Type::Int, ElementKind::Int)
                };
                let is_min_operand = mir::Operand::Constant(mir::Constant::Int(op.to_tag() as i64));
                let elem_kind_val: u8 = if matches!(elem_kind, ElementKind::Float) {
                    1
                } else {
                    0
                };
                let elem_kind_operand =
                    mir::Operand::Constant(mir::Constant::Int(elem_kind_val as i64));
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(ContainerKind::List.minmax_def()),
                    vec![list_operand, is_min_operand, elem_kind_operand],
                    result_type,
                    mir_func,
                );

                return Ok(mir::Operand::Local(result_local));
            }

            // If it's a tuple, call runtime function to find min/max element
            if let Type::Tuple(elem_types) = &arg_type {
                let tuple_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
                let op = if is_min { MinMaxOp::Min } else { MinMaxOp::Max };

                // If key function provided, use with_key variant
                if let Some(ref func_or_builtin) = key_func {
                    use crate::context::KeyFuncSource;
                    let key_source = match func_or_builtin {
                        FuncOrBuiltin::UserFunc(func_id, captures) => {
                            KeyFuncSource::UserFunc(*func_id, captures.clone())
                        }
                        FuncOrBuiltin::Builtin(builtin_kind) => {
                            KeyFuncSource::Builtin(*builtin_kind)
                        }
                    };
                    let resolved = self
                        .emit_key_func_with_captures(Some(&key_source), hir_module, mir_func)?
                        .expect("key_source is Some");

                    // Determine elem_tag for boxing raw elements before calling key function.
                    let first_elem_type = elem_types.first().cloned().unwrap_or(Type::Int);
                    let elem_tag =
                        Self::elem_tag_for_func_or_builtin(func_or_builtin, &first_elem_type);
                    let elem_tag_operand = mir::Operand::Constant(mir::Constant::Int(elem_tag));

                    // Result type matches first element type (for heterogeneous tuples)
                    let is_min_operand =
                        mir::Operand::Constant(mir::Constant::Int(op.to_tag() as i64));
                    let result_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(ContainerKind::Tuple.minmax_with_key_def()),
                        vec![
                            tuple_operand,
                            resolved.func_addr,
                            elem_tag_operand,
                            resolved.captures,
                            resolved.capture_count,
                            is_min_operand,
                        ],
                        first_elem_type,
                        mir_func,
                    );

                    return Ok(mir::Operand::Local(result_local));
                }

                // Original logic for non-key case
                // Determine if element type is float (use first element type)
                let is_float_tuple = elem_types.first().is_some_and(|t| matches!(t, Type::Float));
                let (result_type, elem_kind) = if is_float_tuple {
                    (Type::Float, ElementKind::Float)
                } else {
                    (Type::Int, ElementKind::Int)
                };
                let is_min_operand = mir::Operand::Constant(mir::Constant::Int(op.to_tag() as i64));
                let elem_kind_val: u8 = if matches!(elem_kind, ElementKind::Float) {
                    1
                } else {
                    0
                };
                let elem_kind_operand =
                    mir::Operand::Constant(mir::Constant::Int(elem_kind_val as i64));
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(ContainerKind::Tuple.minmax_def()),
                    vec![tuple_operand, is_min_operand, elem_kind_operand],
                    result_type,
                    mir_func,
                );

                return Ok(mir::Operand::Local(result_local));
            }

            // If it's a set, call runtime function to find min/max element
            if let Type::Set(elem_type) = &arg_type {
                let set_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
                let op = if is_min { MinMaxOp::Min } else { MinMaxOp::Max };

                // If key function provided, use with_key variant
                if let Some(ref func_or_builtin) = key_func {
                    use crate::context::KeyFuncSource;
                    let key_source = match func_or_builtin {
                        FuncOrBuiltin::UserFunc(func_id, captures) => {
                            KeyFuncSource::UserFunc(*func_id, captures.clone())
                        }
                        FuncOrBuiltin::Builtin(builtin_kind) => {
                            KeyFuncSource::Builtin(*builtin_kind)
                        }
                    };
                    let resolved = self
                        .emit_key_func_with_captures(Some(&key_source), hir_module, mir_func)?
                        .expect("key_source is Some");

                    // For sets, the semantics of the third parameter is different:
                    // - needs_unbox=0: builtin key functions expect boxed objects (no unboxing)
                    // - needs_unbox=1: user functions expect raw values (unbox integers)
                    let needs_unbox = match (func_or_builtin, elem_type.as_ref()) {
                        (FuncOrBuiltin::UserFunc(_, _), Type::Int) => 1,
                        _ => 0,
                    };
                    let needs_unbox_operand =
                        mir::Operand::Constant(mir::Constant::Int(needs_unbox));

                    // Runtime returns *mut Obj, need to unbox for primitives
                    let is_min_operand =
                        mir::Operand::Constant(mir::Constant::Int(op.to_tag() as i64));
                    let heap_result_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(ContainerKind::Set.minmax_with_key_def()),
                        vec![
                            set_operand,
                            resolved.func_addr,
                            needs_unbox_operand,
                            resolved.captures,
                            resolved.capture_count,
                            is_min_operand,
                        ],
                        Type::Int,
                        mir_func,
                    );

                    // Unbox the result if it's a primitive type
                    let elem_t = elem_type.as_ref();
                    if matches!(elem_t, Type::Int) {
                        let unboxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_INT,
                            ),
                            vec![mir::Operand::Local(heap_result_local)],
                            Type::Int,
                            mir_func,
                        );
                        return Ok(mir::Operand::Local(unboxed_local));
                    } else if matches!(elem_t, Type::Float) {
                        let unboxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                            ),
                            vec![mir::Operand::Local(heap_result_local)],
                            Type::Float,
                            mir_func,
                        );
                        return Ok(mir::Operand::Local(unboxed_local));
                    } else {
                        // For heap types (str, etc.), return the pointer directly
                        return Ok(mir::Operand::Local(heap_result_local));
                    }
                }

                // Original logic for non-key case
                // Determine if element type is float
                let is_float_set = matches!(elem_type.as_ref(), Type::Float);
                let (result_type, elem_kind) = if is_float_set {
                    (Type::Float, ElementKind::Float)
                } else {
                    (Type::Int, ElementKind::Int)
                };
                let is_min_operand = mir::Operand::Constant(mir::Constant::Int(op.to_tag() as i64));
                let elem_kind_val: u8 = if matches!(elem_kind, ElementKind::Float) {
                    1
                } else {
                    0
                };
                let elem_kind_operand =
                    mir::Operand::Constant(mir::Constant::Int(elem_kind_val as i64));
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(ContainerKind::Set.minmax_def()),
                    vec![set_operand, is_min_operand, elem_kind_operand],
                    result_type,
                    mir_func,
                );

                return Ok(mir::Operand::Local(result_local));
            }

            // Iterator/generator: use IterNextNoExc + GeneratorIsExhausted protocol
            if let Type::Iterator(elem_ty) = &arg_type {
                // Area G §G.4: tuple-yielding iterators need lexicographic
                // compare via `rt_tuple_cmp`. The primitive fast-path below
                // uses raw `BinOp::Lt` / `BinOp::Gt` on the i64 return of
                // `rt_iter_next_no_exc`, which for tuple elements is the
                // pointer value — not lexicographic ordering.
                if matches!(elem_ty.as_ref(), Type::Tuple(_) | Type::TupleVar(_)) {
                    let iter_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
                    let iter_local = self.alloc_and_add_local(arg_type.clone(), mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: iter_local,
                        src: iter_operand,
                    });
                    let result_local = self.lower_minmax_tuple_iter_fold(
                        iter_local,
                        elem_ty.as_ref().clone(),
                        is_min,
                        mir_func,
                    );
                    return Ok(mir::Operand::Local(result_local));
                }

                let iter_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
                let iter_local = self.alloc_and_add_local(arg_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: iter_local,
                    src: iter_operand,
                });

                let is_float_iter = matches!(elem_ty.as_ref(), Type::Float);
                let iter_result_type = if is_float_iter {
                    Type::Float
                } else {
                    Type::Int
                };
                let cmp_op = if is_min {
                    mir::BinOp::Lt
                } else {
                    mir::BinOp::Gt
                };

                // Get first element to initialize result.
                // IterNextNoExc always returns a raw i64 (either an integer value or float
                // bits). Allocate first_local as Int to match this return type.
                let first_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
                    vec![mir::Operand::Local(iter_local)],
                    Type::Int,
                    mir_func,
                );

                let result_local = self.alloc_and_add_local(iter_result_type.clone(), mir_func);
                if is_float_iter {
                    // The iterator yields a pointer to a boxed float object (since list[float]
                    // always uses ELEM_HEAP_OBJ storage). Unbox to get the raw f64 value.
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                        ),
                        args: vec![mir::Operand::Local(first_local)],
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: mir::Operand::Local(first_local),
                    });
                }

                // Loop over remaining elements
                let loop_header = self.new_block();
                let loop_body = self.new_block();
                let loop_exit = self.new_block();
                let loop_header_id = loop_header.id;
                let loop_body_id = loop_body.id;
                let loop_exit_id = loop_exit.id;

                self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

                // Header: call next(), check exhausted
                self.push_block(loop_header);
                // next_local receives the raw i64 from IterNextNoExc.
                let next_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
                    vec![mir::Operand::Local(iter_local)],
                    Type::Int,
                    mir_func,
                );

                let exhausted_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED,
                    ),
                    vec![mir::Operand::Local(iter_local)],
                    Type::Bool,
                    mir_func,
                );

                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(exhausted_local),
                    then_block: loop_exit_id,
                    else_block: loop_body_id,
                };

                // Body: compare and update result
                self.push_block(loop_body);

                // For float iterators, unbox the boxed float pointer from IterNextNoExc
                // to get a raw f64 for comparison (list[float] uses ELEM_HEAP_OBJ storage,
                // so the iterator returns a pointer to a boxed float, not raw float bits).
                let item_operand = if is_float_iter {
                    let float_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT),
                        vec![mir::Operand::Local(next_local)],
                        Type::Float,
                        mir_func,
                    );
                    mir::Operand::Local(float_local)
                } else {
                    mir::Operand::Local(next_local)
                };

                let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: cmp_local,
                    op: cmp_op,
                    left: item_operand.clone(),
                    right: mir::Operand::Local(result_local),
                });

                let then_bb = self.new_block();
                let merge_bb = self.new_block();
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(cmp_local),
                    then_block: then_bb.id,
                    else_block: merge_bb.id,
                };

                self.push_block(then_bb);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: item_operand,
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);

                self.push_block(merge_bb);
                self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

                self.push_block(loop_exit);
                return Ok(mir::Operand::Local(result_local));
            }

            // For non-iterable single argument, just return it
            return self.lower_expr(arg_expr, hir_module, mir_func);
        }

        // Determine result type (float if any arg is float)
        let mut is_float = false;
        for &arg_id in args {
            if self.get_type_of_expr_id(arg_id, hir_module) == Type::Float {
                is_float = true;
                break;
            }
        }

        let result_type = if is_float { Type::Float } else { Type::Int };

        // Evaluate all arguments and promote to float if needed
        let mut operands = Vec::new();
        for &arg_id in args {
            let arg_expr = &hir_module.exprs[arg_id];
            let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
            let arg_type = self.get_type_of_expr_id(arg_id, hir_module);

            let final_operand = if is_float {
                self.promote_to_float_if_needed(mir_func, arg_operand, &arg_type)
            } else {
                arg_operand
            };

            operands.push(final_operand);
        }

        // Create result local and initialize with first argument
        let result_local = self.alloc_and_add_local(result_type, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: operands[0].clone(),
        });

        // For each subsequent argument, compare and update if better
        // min uses Lt (smaller is better), max uses Gt (larger is better)
        let cmp_op = if is_min {
            mir::BinOp::Lt
        } else {
            mir::BinOp::Gt
        };

        for operand in operands.iter().skip(1) {
            let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);

            // cmp = (operand < result) for min, (operand > result) for max
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: cmp_local,
                op: cmp_op,
                left: operand.clone(),
                right: mir::Operand::Local(result_local),
            });

            // if (cmp) result = operand
            let then_bb = self.new_block();
            let merge_bb = self.new_block();

            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(cmp_local),
                then_block: then_bb.id,
                else_block: merge_bb.id,
            };

            // Then block: update result
            self.push_block(then_bb);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: operand.clone(),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);

            // Merge block (continue with next comparison)
            self.push_block(merge_bb);
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower min(range(...)) or max(range(...)) - compute directly from range parameters
    fn lower_minmax_range(
        &mut self,
        range_args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
        is_min: bool,
    ) -> Result<mir::Operand> {
        // Helper to try to extract a constant int from an HIR expression
        fn try_extract_const_int(expr: &hir::Expr, hir_module: &hir::Module) -> Option<i64> {
            match &expr.kind {
                hir::ExprKind::Int(val) => Some(*val),
                hir::ExprKind::UnOp {
                    op: hir::UnOp::Neg,
                    operand,
                } => {
                    let inner = &hir_module.exprs[*operand];
                    if let hir::ExprKind::Int(val) = &inner.kind {
                        Some(-val)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }

        // Parse range arguments and try to detect constant step for compile-time optimization
        let step_const: Option<i64> = if range_args.len() == 3 {
            let step_expr = &hir_module.exprs[range_args[2]];
            try_extract_const_int(step_expr, hir_module)
        } else {
            Some(1) // Default step is 1
        };

        let (start, stop, step) = match range_args.len() {
            1 => {
                let stop_expr = &hir_module.exprs[range_args[0]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    stop,
                    mir::Operand::Constant(mir::Constant::Int(1)),
                )
            }
            2 => {
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (start, stop, mir::Operand::Constant(mir::Constant::Int(1)))
            }
            3 => {
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let step_expr = &hir_module.exprs[range_args[2]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                let step = self.lower_expr(step_expr, hir_module, mir_func)?;
                (start, stop, step)
            }
            _ => {
                return Ok(mir::Operand::Constant(mir::Constant::Int(0)));
            }
        };

        // For min/max of range, we can compute the result directly:
        // For positive step: min is start, max is stop - step + ((stop - start - 1) % step) adjustment
        // For negative step: min is stop + 1 adjusted, max is start
        // Simplified: min(range) with step>0 returns start, max returns last element

        let result_local = self.alloc_and_add_local(Type::Int, mir_func);

        // We need to handle this at runtime since step can be negative
        // For now, compute directly for the common case of positive step
        // min(range(start, stop, step)) = start (if step > 0)
        // max(range(start, stop, step)) = start + ((stop - start - 1) / step) * step (if step > 0)

        if is_min {
            // For min(range), if step > 0, return start
            // if step < 0, return last element
            // Use step_const extracted from HIR to handle negative literals like -1
            if let Some(step_val) = step_const {
                if step_val > 0 {
                    // Positive step: min is start
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: start,
                    });
                } else if step_val < 0 {
                    // Negative step: min is the last element
                    // For negative step, last = start + ((stop + 1 - start) // step) * step
                    // e.g. range(5, 0, -1) -> [5,4,3,2,1], last = 5 + ((1 - 5) // -1) * -1 = 5 + 4*-1 = 1
                    let stop_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: stop_local,
                        src: stop,
                    });

                    let start_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: start_local,
                        src: start,
                    });

                    // stop_plus_1 = stop + 1 (for negative step, we add 1 instead of subtracting)
                    let one = mir::Operand::Constant(mir::Constant::Int(1));
                    let stop_plus_1 = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: stop_plus_1,
                        op: mir::BinOp::Add,
                        left: mir::Operand::Local(stop_local),
                        right: one,
                    });

                    // diff = stop + 1 - start
                    let diff = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: diff,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(stop_plus_1),
                        right: mir::Operand::Local(start_local),
                    });

                    let step_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: step_local,
                        src: step,
                    });

                    // n_steps = diff // step
                    let n_steps = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: n_steps,
                        op: mir::BinOp::FloorDiv,
                        left: mir::Operand::Local(diff),
                        right: mir::Operand::Local(step_local),
                    });

                    // offset = n_steps * step
                    let offset = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: offset,
                        op: mir::BinOp::Mul,
                        left: mir::Operand::Local(n_steps),
                        right: mir::Operand::Local(step_local),
                    });

                    // result = start + offset
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir::BinOp::Add,
                        left: mir::Operand::Local(start_local),
                        right: mir::Operand::Local(offset),
                    });
                } else {
                    // step == 0, invalid range, return 0
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: result_local,
                        value: mir::Constant::Int(0),
                    });
                }
            } else {
                // Dynamic step - assume positive, return start
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: start,
                });
            }
        } else {
            // For max(range), if step > 0, return last element
            // if step < 0, return start
            // Use step_const extracted from HIR
            if let Some(step_val) = step_const {
                if step_val > 0 {
                    // Positive step: max is the last element
                    // last = start + ((stop - start - 1) / step) * step
                    let stop_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: stop_local,
                        src: stop,
                    });

                    let start_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: start_local,
                        src: start,
                    });

                    let one = mir::Operand::Constant(mir::Constant::Int(1));
                    let stop_minus_1 = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: stop_minus_1,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(stop_local),
                        right: one,
                    });

                    let diff = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: diff,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(stop_minus_1),
                        right: mir::Operand::Local(start_local),
                    });

                    let step_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: step_local,
                        src: step,
                    });

                    let n_steps = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: n_steps,
                        op: mir::BinOp::FloorDiv,
                        left: mir::Operand::Local(diff),
                        right: mir::Operand::Local(step_local),
                    });

                    let offset = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: offset,
                        op: mir::BinOp::Mul,
                        left: mir::Operand::Local(n_steps),
                        right: mir::Operand::Local(step_local),
                    });

                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir::BinOp::Add,
                        left: mir::Operand::Local(start_local),
                        right: mir::Operand::Local(offset),
                    });
                } else if step_val < 0 {
                    // Negative step: max is start
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: start,
                    });
                } else {
                    // step == 0, invalid range, return 0
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: result_local,
                        value: mir::Constant::Int(0),
                    });
                }
            } else {
                // Dynamic step - assume positive, compute last element
                let stop_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: stop_local,
                    src: stop,
                });

                let start_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: start_local,
                    src: start,
                });

                let one = mir::Operand::Constant(mir::Constant::Int(1));
                let stop_minus_1 = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: stop_minus_1,
                    op: mir::BinOp::Sub,
                    left: mir::Operand::Local(stop_local),
                    right: one,
                });

                let diff = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: diff,
                    op: mir::BinOp::Sub,
                    left: mir::Operand::Local(stop_minus_1),
                    right: mir::Operand::Local(start_local),
                });

                let step_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: step_local,
                    src: step,
                });

                let n_steps = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: n_steps,
                    op: mir::BinOp::FloorDiv,
                    left: mir::Operand::Local(diff),
                    right: mir::Operand::Local(step_local),
                });

                let offset = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: offset,
                    op: mir::BinOp::Mul,
                    left: mir::Operand::Local(n_steps),
                    right: mir::Operand::Local(step_local),
                });

                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir::BinOp::Add,
                    left: mir::Operand::Local(start_local),
                    right: mir::Operand::Local(offset),
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Area G §G.4: lexicographic min/max fold over an iterator yielding
    /// tuples. Seeds with the first element from `rt_iter_next_no_exc`,
    /// then loops using `rt_tuple_cmp(candidate, best, op_tag)` per
    /// element; `op_tag` = 0 (Lt) for min, 2 (Gt) for max. Both use strict
    /// comparison — on tie, the first-seen best stays (matches CPython).
    ///
    /// Parallel to `lower_minmax_class_fold` in
    /// `crates/lowering/src/expressions/builtins/reductions/mod.rs`; both
    /// share the seed / header / body / update / exit block shape.
    fn lower_minmax_tuple_iter_fold(
        &mut self,
        iter_local: pyaot_utils::LocalId,
        elem_ty: Type,
        is_min: bool,
        mir_func: &mut mir::Function,
    ) -> pyaot_utils::LocalId {
        let op_tag: i64 = if is_min { 0 } else { 2 };
        let best_local = self.alloc_gc_local(elem_ty.clone(), mir_func);

        let seed_bb = self.new_block();
        let seed_bb_id = seed_bb.id;
        let raise_bb = self.new_block();
        let raise_bb_id = raise_bb.id;
        let seed_ok_bb = self.new_block();
        let seed_ok_bb_id = seed_ok_bb.id;
        let header_bb = self.new_block();
        let header_bb_id = header_bb.id;
        let body_bb = self.new_block();
        let body_bb_id = body_bb.id;
        let update_bb = self.new_block();
        let update_bb_id = update_bb.id;
        let continue_bb = self.new_block();
        let continue_bb_id = continue_bb.id;
        let exit_bb = self.new_block();
        let exit_bb_id = exit_bb.id;

        // Entry: seed `best` with the first element.
        self.current_block_mut().terminator = mir::Terminator::Goto(seed_bb_id);
        self.push_block(seed_bb);
        let first_val = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            elem_ty.clone(),
            mir_func,
        );
        let first_exhausted = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(first_exhausted),
            then_block: raise_bb_id,
            else_block: seed_ok_bb_id,
        };

        // Empty iterable — mirror `lower_minmax_class_fold` behaviour: null
        // accumulator and exit. CPython-strict ValueError is out of scope.
        self.push_block(raise_bb);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: best_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(exit_bb_id);

        self.push_block(seed_ok_bb);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: best_local,
            src: mir::Operand::Local(first_val),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);

        // Loop header: fetch next, check exhausted.
        self.push_block(header_bb);
        let cand_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            elem_ty.clone(),
            mir_func,
        );
        let exhausted_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: exit_bb_id,
            else_block: body_bb_id,
        };

        // Body: rt_tuple_cmp(cand, best, op_tag) returns i8 (0 or 1).
        self.push_block(body_bb);
        let cmp_dest = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: cmp_dest,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_CMP_TUPLE_ORD),
            args: vec![
                mir::Operand::Local(cand_local),
                mir::Operand::Local(best_local),
                mir::Operand::Constant(mir::Constant::Int(op_tag)),
            ],
        });
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cmp_dest),
            then_block: update_bb_id,
            else_block: continue_bb_id,
        };

        // Update: best := cand.
        self.push_block(update_bb);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: best_local,
            src: mir::Operand::Local(cand_local),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(continue_bb_id);

        // Continue: back to header.
        self.push_block(continue_bb);
        self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);

        self.push_block(exit_bb);
        best_local
    }
}
