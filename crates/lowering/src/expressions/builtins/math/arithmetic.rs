//! Arithmetic functions lowering: abs(), pow(), round(), sum(), divmod()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower abs(x)
    pub(in crate::expressions::builtins) fn lower_abs(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "abs", self.call_span())?;

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        match arg_type {
            Type::Int => {
                // abs(int) -> emit instructions: result = (x >= 0) ? x : -x
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                // Compute negation
                let neg_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::UnOp {
                    dest: neg_local,
                    op: mir::UnOp::Neg,
                    operand: arg_operand.clone(),
                });

                // Test: x < 0
                let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);
                let zero = mir::Operand::Constant(mir::Constant::Int(0));
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: cmp_local,
                    op: mir::BinOp::Lt,
                    left: arg_operand.clone(),
                    right: zero,
                });

                // Create blocks
                let then_bb = self.new_block(); // x < 0, use negation
                let else_bb = self.new_block(); // x >= 0, use original
                let merge_bb = self.new_block();

                // Branch on condition
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(cmp_local),
                    then_block: then_bb.id,
                    else_block: else_bb.id,
                };

                // Then block: result = -x
                self.push_block(then_bb);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Local(neg_local),
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);

                // Else block: result = x
                self.push_block(else_bb);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: arg_operand,
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);

                // Merge block
                self.push_block(merge_bb);

                Ok(mir::Operand::Local(result_local))
            }
            Type::Float => {
                // abs(float) -> use FloatAbs instruction
                let result_local = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::FloatAbs {
                    dest: result_local,
                    src: arg_operand,
                });
                Ok(mir::Operand::Local(result_local))
            }
            Type::Class { class_id, .. } => {
                // abs(obj) -> call __abs__ dunder if defined
                if let Some(abs_func) = self
                    .get_class_info(&class_id)
                    .and_then(|ci| ci.get_dunder_func("__abs__"))
                {
                    let result_local = self.alloc_and_add_local(arg_type.clone(), mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: abs_func,
                        args: vec![arg_operand],
                    });
                    Ok(mir::Operand::Local(result_local))
                } else {
                    Ok(arg_operand)
                }
            }
            _ => {
                // For other types, return the value as-is (fallback)
                Ok(arg_operand)
            }
        }
    }

    /// Lower pow(base, exp)
    pub(in crate::expressions::builtins) fn lower_pow(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.len() != 2 {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                "pow() requires exactly 2 arguments",
                self.call_span(),
            ));
        }

        // Get both arguments
        let base_expr = &hir_module.exprs[args[0]];
        let exp_expr = &hir_module.exprs[args[1]];

        let base_operand = self.lower_expr(base_expr, hir_module, mir_func)?;
        let exp_operand = self.lower_expr(exp_expr, hir_module, mir_func)?;

        let base_type = self.seed_expr_type(args[0], hir_module);
        let exp_type = self.seed_expr_type(args[1], hir_module);

        // Convert both operands to float if needed
        let base_float = self.promote_to_float_if_needed(mir_func, base_operand, &base_type);
        let exp_float = self.promote_to_float_if_needed(mir_func, exp_operand, &exp_type);

        // Create result local and emit runtime call
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_POW_FLOAT),
            vec![base_float, exp_float],
            Type::Float,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower round(x) or round(x, ndigits)
    pub(in crate::expressions::builtins) fn lower_round(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_min_args(args, 1, "round", self.call_span())?;

        let x_expr = &hir_module.exprs[args[0]];
        let x_operand = self.lower_expr(x_expr, hir_module, mir_func)?;

        if args.len() == 1 {
            // round(x) -> int
            let result_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ROUND_TO_INT),
                vec![x_operand],
                Type::Int,
                mir_func,
            );

            Ok(mir::Operand::Local(result_local))
        } else {
            // round(x, ndigits) -> float
            let ndigits_expr = &hir_module.exprs[args[1]];
            let ndigits_operand = self.lower_expr(ndigits_expr, hir_module, mir_func)?;

            let result_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ROUND_TO_DIGITS),
                vec![x_operand, ndigits_operand],
                Type::Float,
                mir_func,
            );

            Ok(mir::Operand::Local(result_local))
        }
    }

    /// Lower sum(iterable, start=0)
    pub(in crate::expressions::builtins) fn lower_sum(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::Int(0)));
        }

        // Area C §C.3: when the iterable's elements are user-class
        // instances, fold via the `__add__` / `__radd__` dunder state
        // machine (extracted in `binary_ops::dispatch_class_binop`).
        // Falls through to the primitive fast path below for non-class
        // elements, matching legacy behaviour bit-for-bit.
        if let Some(class_result) = self.try_lower_sum_class_elem(args, hir_module, mir_func)? {
            return Ok(class_result);
        }

        let iterable_expr = &hir_module.exprs[args[0]];
        let iterable_type = self.seed_expr_type(args[0], hir_module);

        // Infer element type from list or iterator type annotation
        let element_type = match &iterable_type {
            Type::List(elem_ty) => (**elem_ty).clone(),
            Type::Iterator(elem_ty) => (**elem_ty).clone(),
            _ => Type::Int, // fallback for other iterables
        };

        // Check if start value is provided and its type
        let start_type = if args.len() > 1 {
            self.seed_expr_type(args[1], hir_module)
        } else {
            Type::Int // default start is 0 (int)
        };

        // Result type promotion: float if either element or start is float
        let result_type = if element_type == Type::Float || start_type == Type::Float {
            Type::Float
        } else {
            Type::Int
        };

        // Get start value with correct type
        let start_operand = if args.len() > 1 {
            let start_expr = &hir_module.exprs[args[1]];
            let start_op = self.lower_expr(start_expr, hir_module, mir_func)?;

            // Promote start to float if needed
            if result_type == Type::Float {
                self.promote_to_float_if_needed(mir_func, start_op, &start_type)
            } else {
                start_op
            }
        } else {
            // Default start value based on result type
            if result_type == Type::Float {
                mir::Operand::Constant(mir::Constant::Float(0.0))
            } else {
                mir::Operand::Constant(mir::Constant::Int(0))
            }
        };

        // Create result accumulator and initialize
        let result_local = self.alloc_and_add_local(result_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: start_operand,
        });

        // Lower the iterable
        let iterable_operand =
            self.lower_expr_expecting(iterable_expr, None, hir_module, mir_func)?;

        // Iterator path: use IterNextNoExc + GeneratorIsExhausted protocol
        if matches!(iterable_type, Type::Iterator(_)) {
            let iter_local = self.alloc_and_add_local(iterable_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: iter_local,
                src: iterable_operand,
            });

            let loop_header = self.new_block();
            let loop_body = self.new_block();
            let loop_exit = self.new_block();

            let loop_header_id = loop_header.id;
            let loop_body_id = loop_body.id;
            let loop_exit_id = loop_exit.id;

            self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

            // Header: call next(), check exhausted
            self.push_block(loop_header);

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

            // Body: accumulate
            self.push_block(loop_body);

            let item_operand = if result_type == Type::Float {
                self.promote_to_float_if_needed(
                    mir_func,
                    mir::Operand::Local(next_local),
                    &element_type,
                )
            } else {
                mir::Operand::Local(next_local)
            };

            let temp_result = self.alloc_and_add_local(result_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: temp_result,
                op: mir::BinOp::Add,
                left: mir::Operand::Local(result_local),
                right: item_operand,
            });

            self.emit_instruction(mir::InstructionKind::Copy {
                dest: result_local,
                src: mir::Operand::Local(temp_result),
            });

            self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

            self.push_block(loop_exit);

            return Ok(mir::Operand::Local(result_local));
        }

        // List path: indexed iteration via ListLen + ListGet
        let len_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN),
            vec![iterable_operand.clone()],
            Type::Int,
            mir_func,
        );

        // Create loop counter and initialize
        let counter_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        // Create loop blocks
        let loop_header = self.new_block();
        let loop_body = self.new_block();
        let loop_exit = self.new_block();

        let loop_header_id = loop_header.id;
        let loop_body_id = loop_body.id;
        let loop_exit_id = loop_exit.id;

        // Jump to loop header
        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop header: check counter < len
        self.push_block(loop_header);

        let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cmp_local,
            op: mir::BinOp::Lt,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Local(len_local),
        });

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cmp_local),
            then_block: loop_body_id,
            else_block: loop_exit_id,
        };

        // Loop body: item = iterable[counter]; result += item; counter++
        self.push_block(loop_body);

        // Get item at index
        // For floats, ListGet returns a boxed pointer (i64), not the float value
        let item_type = if element_type == Type::Float {
            Type::HeapAny // Guaranteed heap pointer (*mut Obj) returned by ListGet for floats
        } else {
            element_type.clone()
        };
        // Note: For floats, we need special gc_root handling
        let item_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
            vec![iterable_operand.clone(), mir::Operand::Local(counter_local)],
            item_type,
            mir_func,
        );

        // Unbox float elements (ListGet returns boxed pointer for floats)
        let unboxed_item = if element_type == Type::Float {
            let unboxed_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT),
                vec![mir::Operand::Local(item_local)],
                Type::Float,
                mir_func,
            );
            mir::Operand::Local(unboxed_local)
        } else {
            mir::Operand::Local(item_local)
        };

        // Promote item to float if needed (when summing int list with float start)
        let item_operand = if result_type == Type::Float {
            self.promote_to_float_if_needed(mir_func, unboxed_item, &element_type)
        } else {
            unboxed_item
        };

        // result = result + item
        let temp_result = self.alloc_and_add_local(result_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: temp_result,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(result_local),
            right: item_operand,
        });

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: mir::Operand::Local(temp_result),
        });

        // counter = counter + 1
        let temp_counter = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: temp_counter,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Local(temp_counter),
        });

        // Jump back to header
        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop exit
        self.push_block(loop_exit);

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower divmod(a, b) -> (a // b, a % b)
    pub(in crate::expressions::builtins) fn lower_divmod(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.len() != 2 {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                "divmod() requires exactly 2 arguments",
                self.call_span(),
            ));
        }

        let a_expr = &hir_module.exprs[args[0]];
        let b_expr = &hir_module.exprs[args[1]];

        let a_type = self.seed_expr_type(args[0], hir_module);
        let b_type = self.seed_expr_type(args[1], hir_module);

        let a_operand = self.lower_expr(a_expr, hir_module, mir_func)?;
        let b_operand = self.lower_expr(b_expr, hir_module, mir_func)?;

        // Determine result type: float if either arg is float, otherwise int
        let is_float = matches!(a_type, Type::Float) || matches!(b_type, Type::Float);
        let result_elem_ty = if is_float { Type::Float } else { Type::Int };

        // Compute a // b
        let quot_local = self.alloc_and_add_local(result_elem_ty.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: quot_local,
            op: mir::BinOp::FloorDiv,
            left: a_operand.clone(),
            right: b_operand.clone(),
        });

        // Compute a % b
        let rem_local = self.alloc_and_add_local(result_elem_ty.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: rem_local,
            op: mir::BinOp::Mod,
            left: a_operand,
            right: b_operand,
        });

        // For int results use ELEM_RAW_INT (1), for float use ELEM_HEAP_OBJ (0)
        let elem_tag: i64 = if is_float { 0 } else { 1 };

        // Create tuple (quot, rem)
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
            vec![
                mir::Operand::Constant(mir::Constant::Int(2)),
                mir::Operand::Constant(mir::Constant::Int(elem_tag)),
            ],
            Type::Tuple(vec![result_elem_ty.clone(), result_elem_ty.clone()]),
            mir_func,
        );

        // Box float results before storing in tuple
        let quot_operand = if is_float {
            let boxed = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT),
                vec![mir::Operand::Local(quot_local)],
                Type::HeapAny,
                mir_func,
            );
            mir::Operand::Local(boxed)
        } else {
            mir::Operand::Local(quot_local)
        };

        let rem_operand = if is_float {
            let boxed = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT),
                vec![mir::Operand::Local(rem_local)],
                Type::HeapAny,
                mir_func,
            );
            mir::Operand::Local(boxed)
        } else {
            mir::Operand::Local(rem_local)
        };

        // Set tuple elements
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
            args: vec![
                mir::Operand::Local(result_local),
                mir::Operand::Constant(mir::Constant::Int(0)),
                quot_operand,
            ],
        });

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_SET),
            args: vec![
                mir::Operand::Local(result_local),
                mir::Operand::Constant(mir::Constant::Int(1)),
                rem_operand,
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
