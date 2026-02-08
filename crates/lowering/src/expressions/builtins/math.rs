//! Math functions lowering: abs(), pow(), round(), min(), max(), sum(), divmod(), bin(), hex(), oct()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir::{self as mir, ContainerKind, ElementKind, MinMaxOp};
use pyaot_types::Type;

use crate::context::{FuncOrBuiltin, Lowering};

impl<'a> Lowering<'a> {
    /// Lower abs(x)
    pub(super) fn lower_abs(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "abs");

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

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
            _ => {
                // For other types, return the value as-is (fallback)
                Ok(arg_operand)
            }
        }
    }

    /// Lower pow(base, exp)
    pub(super) fn lower_pow(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.len() != 2 {
            panic!("pow() requires 2 arguments");
        }

        // Get both arguments
        let base_expr = &hir_module.exprs[args[0]];
        let exp_expr = &hir_module.exprs[args[1]];

        let base_operand = self.lower_expr(base_expr, hir_module, mir_func)?;
        let exp_operand = self.lower_expr(exp_expr, hir_module, mir_func)?;

        let base_type = self.get_expr_type(base_expr, hir_module);
        let exp_type = self.get_expr_type(exp_expr, hir_module);

        // Convert both operands to float if needed
        let base_float = self.promote_to_float_if_needed(mir_func, base_operand, &base_type);
        let exp_float = self.promote_to_float_if_needed(mir_func, exp_operand, &exp_type);

        // Create result local and emit runtime call
        let result_local = self.alloc_typed_local(mir_func, Type::Float);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::PowFloat,
            args: vec![base_float, exp_float],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower round(x) or round(x, ndigits)
    pub(super) fn lower_round(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_min_args(args, 1, "round");

        let x_expr = &hir_module.exprs[args[0]];
        let x_operand = self.lower_expr(x_expr, hir_module, mir_func)?;

        if args.len() == 1 {
            // round(x) -> int
            let result_local = self.alloc_and_add_local(Type::Int, mir_func);

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::RoundToInt,
                args: vec![x_operand],
            });

            Ok(mir::Operand::Local(result_local))
        } else {
            // round(x, ndigits) -> float
            let ndigits_expr = &hir_module.exprs[args[1]];
            let ndigits_operand = self.lower_expr(ndigits_expr, hir_module, mir_func)?;

            let result_local = self.alloc_and_add_local(Type::Float, mir_func);

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::RoundToDigits,
                args: vec![x_operand, ndigits_operand],
            });

            Ok(mir::Operand::Local(result_local))
        }
    }

    /// Lower min() or max() builtin.
    /// The is_min parameter controls the comparison: true = Lt (min), false = Gt (max).
    pub(super) fn lower_minmax_builtin(
        &mut self,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
        is_min: bool,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            panic!(
                "{}() requires at least 1 argument",
                if is_min { "min" } else { "max" }
            );
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
            panic!(
                "{}() with key= requires exactly one iterable argument",
                if is_min { "min" } else { "max" }
            );
        }

        if args.len() == 1 {
            // Single argument - check if it's an iterable (list, tuple, set, or range)
            let arg_expr = &hir_module.exprs[args[0]];
            let arg_type = self.get_expr_type(arg_expr, hir_module);

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
                    let key_fn_local = self.alloc_and_add_local(Type::Int, mir_func);
                    match func_or_builtin {
                        FuncOrBuiltin::UserFunc(func_id, _) => {
                            self.emit_instruction(mir::InstructionKind::FuncAddr {
                                dest: key_fn_local,
                                func: *func_id,
                            });
                        }
                        FuncOrBuiltin::Builtin(builtin_kind) => {
                            self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                                dest: key_fn_local,
                                builtin: *builtin_kind,
                            });
                        }
                    }

                    // Determine elem_tag for boxing raw elements before calling key function.
                    // Only builtin wrappers need boxing - user functions work with raw values.
                    let elem_tag =
                        Self::elem_tag_for_func_or_builtin(func_or_builtin, elem_type.as_ref());
                    let elem_tag_operand = mir::Operand::Constant(mir::Constant::Int(elem_tag));

                    // Result type matches element type (for heap types, use heap object type)
                    let result_type = elem_type.as_ref().clone();
                    let result_local = self.alloc_typed_local(mir_func, result_type);

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::ContainerMinMax {
                            container: ContainerKind::List,
                            op,
                            elem: ElementKind::WithKey,
                        },
                        args: vec![
                            list_operand,
                            mir::Operand::Local(key_fn_local),
                            elem_tag_operand,
                        ],
                    });

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
                let result_local = self.alloc_typed_local(mir_func, result_type);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ContainerMinMax {
                        container: ContainerKind::List,
                        op,
                        elem: elem_kind,
                    },
                    args: vec![list_operand],
                });

                return Ok(mir::Operand::Local(result_local));
            }

            // If it's a tuple, call runtime function to find min/max element
            if let Type::Tuple(elem_types) = &arg_type {
                let tuple_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
                let op = if is_min { MinMaxOp::Min } else { MinMaxOp::Max };

                // If key function provided, use with_key variant
                if let Some(ref func_or_builtin) = key_func {
                    let key_fn_local = self.alloc_and_add_local(Type::Int, mir_func);
                    match func_or_builtin {
                        FuncOrBuiltin::UserFunc(func_id, _) => {
                            self.emit_instruction(mir::InstructionKind::FuncAddr {
                                dest: key_fn_local,
                                func: *func_id,
                            });
                        }
                        FuncOrBuiltin::Builtin(builtin_kind) => {
                            self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                                dest: key_fn_local,
                                builtin: *builtin_kind,
                            });
                        }
                    }

                    // Determine elem_tag for boxing raw elements before calling key function.
                    // Only builtin wrappers need boxing - user functions work with raw values.
                    // Use first element type for tuples.
                    let first_elem_type = elem_types.first().cloned().unwrap_or(Type::Int);
                    let elem_tag =
                        Self::elem_tag_for_func_or_builtin(func_or_builtin, &first_elem_type);
                    let elem_tag_operand = mir::Operand::Constant(mir::Constant::Int(elem_tag));

                    // Result type matches first element type (for heterogeneous tuples)
                    let result_local = self.alloc_typed_local(mir_func, first_elem_type);

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::ContainerMinMax {
                            container: ContainerKind::Tuple,
                            op,
                            elem: ElementKind::WithKey,
                        },
                        args: vec![
                            tuple_operand,
                            mir::Operand::Local(key_fn_local),
                            elem_tag_operand,
                        ],
                    });

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
                let result_local = self.alloc_typed_local(mir_func, result_type);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ContainerMinMax {
                        container: ContainerKind::Tuple,
                        op,
                        elem: elem_kind,
                    },
                    args: vec![tuple_operand],
                });

                return Ok(mir::Operand::Local(result_local));
            }

            // If it's a set, call runtime function to find min/max element
            if let Type::Set(elem_type) = &arg_type {
                let set_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
                let op = if is_min { MinMaxOp::Min } else { MinMaxOp::Max };

                // If key function provided, use with_key variant
                if let Some(ref func_or_builtin) = key_func {
                    let key_fn_local = self.alloc_and_add_local(Type::Int, mir_func);
                    match func_or_builtin {
                        FuncOrBuiltin::UserFunc(func_id, _) => {
                            self.emit_instruction(mir::InstructionKind::FuncAddr {
                                dest: key_fn_local,
                                func: *func_id,
                            });
                        }
                        FuncOrBuiltin::Builtin(builtin_kind) => {
                            self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                                dest: key_fn_local,
                                builtin: *builtin_kind,
                            });
                        }
                    }

                    // For sets, the semantics of the third parameter is different:
                    // - needs_unbox=0: builtin key functions expect boxed objects (no unboxing)
                    // - needs_unbox=1: user functions expect raw values (unbox integers)
                    // This is the OPPOSITE of lists where we pass elem_tag for boxing.
                    let needs_unbox = match (func_or_builtin, elem_type.as_ref()) {
                        (FuncOrBuiltin::UserFunc(_, _), Type::Int) => 1, // Unbox ints for user funcs
                        _ => 0, // Builtins or non-int types: no unboxing
                    };
                    let needs_unbox_operand =
                        mir::Operand::Constant(mir::Constant::Int(needs_unbox));

                    // Runtime returns *mut Obj, need to unbox for primitives
                    let heap_result_local = self.alloc_and_add_local(Type::Int, mir_func); // Temporary for pointer

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: heap_result_local,
                        func: mir::RuntimeFunc::ContainerMinMax {
                            container: ContainerKind::Set,
                            op,
                            elem: ElementKind::WithKey,
                        },
                        args: vec![
                            set_operand,
                            mir::Operand::Local(key_fn_local),
                            needs_unbox_operand,
                        ],
                    });

                    // Unbox the result if it's a primitive type
                    let elem_t = elem_type.as_ref();
                    if matches!(elem_t, Type::Int) {
                        let unboxed_local = self.alloc_typed_local(mir_func, Type::Int);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: unboxed_local,
                            func: mir::RuntimeFunc::UnboxInt,
                            args: vec![mir::Operand::Local(heap_result_local)],
                        });
                        return Ok(mir::Operand::Local(unboxed_local));
                    } else if matches!(elem_t, Type::Float) {
                        let unboxed_local = self.alloc_typed_local(mir_func, Type::Float);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: unboxed_local,
                            func: mir::RuntimeFunc::UnboxFloat,
                            args: vec![mir::Operand::Local(heap_result_local)],
                        });
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
                let result_local = self.alloc_typed_local(mir_func, result_type);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ContainerMinMax {
                        container: ContainerKind::Set,
                        op,
                        elem: elem_kind,
                    },
                    args: vec![set_operand],
                });

                return Ok(mir::Operand::Local(result_local));
            }

            // For non-iterable single argument, just return it
            return self.lower_expr(arg_expr, hir_module, mir_func);
        }

        // Determine result type (float if any arg is float)
        let mut is_float = false;
        for &arg_id in args {
            let arg_expr = &hir_module.exprs[arg_id];
            if self.get_expr_type(arg_expr, hir_module) == Type::Float {
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
            let arg_type = self.get_expr_type(arg_expr, hir_module);

            let final_operand = if is_float {
                self.promote_to_float_if_needed(mir_func, arg_operand, &arg_type)
            } else {
                arg_operand
            };

            operands.push(final_operand);
        }

        // Create result local and initialize with first argument
        let result_local = self.alloc_typed_local(mir_func, result_type);
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
            let cmp_local = self.alloc_typed_local(mir_func, Type::Bool);

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

    /// Lower sum(iterable, start=0)
    pub(super) fn lower_sum(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::Int(0)));
        }

        let iterable_expr = &hir_module.exprs[args[0]];
        let iterable_type = self.get_expr_type(iterable_expr, hir_module);

        // Infer element type from list type annotation
        let element_type = match &iterable_type {
            Type::List(elem_ty) => (**elem_ty).clone(),
            _ => Type::Int, // fallback for non-list iterables
        };

        // Check if start value is provided and its type
        let start_type = if args.len() > 1 {
            let start_expr = &hir_module.exprs[args[1]];
            self.get_expr_type(start_expr, hir_module)
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
        let result_local = self.alloc_typed_local(mir_func, result_type.clone());
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: start_operand,
        });

        // Lower the iterable
        let iterable_operand = self.lower_expr(iterable_expr, hir_module, mir_func)?;

        // Get length
        let len_local = self.alloc_typed_local(mir_func, Type::Int);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: len_local,
            func: mir::RuntimeFunc::ListLen,
            args: vec![iterable_operand.clone()],
        });

        // Create loop counter and initialize
        let counter_local = self.alloc_typed_local(mir_func, Type::Int);
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

        let cmp_local = self.alloc_typed_local(mir_func, Type::Bool);
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
            Type::Str // Use Str as proxy for pointer type (maps to I64)
        } else {
            element_type.clone()
        };
        // Note: For floats, we need special gc_root handling
        let item_local = self.alloc_and_add_local(item_type, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: item_local,
            func: mir::RuntimeFunc::ListGet,
            args: vec![iterable_operand.clone(), mir::Operand::Local(counter_local)],
        });

        // Unbox float elements (ListGet returns boxed pointer for floats)
        let unboxed_item = if element_type == Type::Float {
            let unboxed_local = self.alloc_typed_local(mir_func, Type::Float);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: unboxed_local,
                func: mir::RuntimeFunc::UnboxFloat,
                args: vec![mir::Operand::Local(item_local)],
            });
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
        let temp_result = self.alloc_typed_local(mir_func, result_type.clone());
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
        let temp_counter = self.alloc_typed_local(mir_func, Type::Int);
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
    pub(super) fn lower_divmod(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.len() != 2 {
            panic!("divmod() requires 2 arguments");
        }

        let a_expr = &hir_module.exprs[args[0]];
        let b_expr = &hir_module.exprs[args[1]];

        let a_operand = self.lower_expr(a_expr, hir_module, mir_func)?;
        let b_operand = self.lower_expr(b_expr, hir_module, mir_func)?;

        // Compute a // b
        let quot_local = self.alloc_typed_local(mir_func, Type::Int);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: quot_local,
            op: mir::BinOp::FloorDiv,
            left: a_operand.clone(),
            right: b_operand.clone(),
        });

        // Compute a % b
        let rem_local = self.alloc_typed_local(mir_func, Type::Int);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: rem_local,
            op: mir::BinOp::Mod,
            left: a_operand,
            right: b_operand,
        });

        // Create tuple (quot, rem)
        let result_local =
            self.alloc_and_add_local(Type::Tuple(vec![Type::Int, Type::Int]), mir_func);

        // Allocate tuple with 2 elements (elem_tag=1 for ELEM_RAW_INT)
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeTuple,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(2)),
                mir::Operand::Constant(mir::Constant::Int(1)), // ELEM_RAW_INT
            ],
        });

        // Set tuple elements
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::TupleSet,
            args: vec![
                mir::Operand::Local(result_local),
                mir::Operand::Constant(mir::Constant::Int(0)),
                mir::Operand::Local(quot_local),
            ],
        });

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::TupleSet,
            args: vec![
                mir::Operand::Local(result_local),
                mir::Operand::Constant(mir::Constant::Int(1)),
                mir::Operand::Local(rem_local),
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower bin(n) -> str (e.g., '0b1010')
    pub(super) fn lower_bin(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "bin");

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::IntToBin,
            args: vec![n_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower hex(n) -> str (e.g., '0xff')
    pub(super) fn lower_hex(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "hex");

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::IntToHex,
            args: vec![n_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower oct(n) -> str (e.g., '0o10')
    pub(super) fn lower_oct(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "oct");

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::IntToOct,
            args: vec![n_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower format-specific integer conversion (hex/oct/bin without prefix)
    pub(super) fn lower_fmt_int(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
        runtime_func: mir::RuntimeFunc,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "fmt_int");

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: runtime_func,
            args: vec![n_operand],
        });

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

        let result_local = self.alloc_typed_local(mir_func, Type::Int);

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
                    let stop_local = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: stop_local,
                        src: stop,
                    });

                    let start_local = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: start_local,
                        src: start,
                    });

                    // stop_plus_1 = stop + 1 (for negative step, we add 1 instead of subtracting)
                    let one = mir::Operand::Constant(mir::Constant::Int(1));
                    let stop_plus_1 = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: stop_plus_1,
                        op: mir::BinOp::Add,
                        left: mir::Operand::Local(stop_local),
                        right: one,
                    });

                    // diff = stop + 1 - start
                    let diff = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: diff,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(stop_plus_1),
                        right: mir::Operand::Local(start_local),
                    });

                    let step_local = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: step_local,
                        src: step,
                    });

                    // n_steps = diff // step
                    let n_steps = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: n_steps,
                        op: mir::BinOp::FloorDiv,
                        left: mir::Operand::Local(diff),
                        right: mir::Operand::Local(step_local),
                    });

                    // offset = n_steps * step
                    let offset = self.alloc_typed_local(mir_func, Type::Int);
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
                    let stop_local = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: stop_local,
                        src: stop,
                    });

                    let start_local = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: start_local,
                        src: start,
                    });

                    let one = mir::Operand::Constant(mir::Constant::Int(1));
                    let stop_minus_1 = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: stop_minus_1,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(stop_local),
                        right: one,
                    });

                    let diff = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: diff,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(stop_minus_1),
                        right: mir::Operand::Local(start_local),
                    });

                    let step_local = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: step_local,
                        src: step,
                    });

                    let n_steps = self.alloc_typed_local(mir_func, Type::Int);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: n_steps,
                        op: mir::BinOp::FloorDiv,
                        left: mir::Operand::Local(diff),
                        right: mir::Operand::Local(step_local),
                    });

                    let offset = self.alloc_typed_local(mir_func, Type::Int);
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
                let stop_local = self.alloc_typed_local(mir_func, Type::Int);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: stop_local,
                    src: stop,
                });

                let start_local = self.alloc_typed_local(mir_func, Type::Int);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: start_local,
                    src: start,
                });

                let one = mir::Operand::Constant(mir::Constant::Int(1));
                let stop_minus_1 = self.alloc_typed_local(mir_func, Type::Int);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: stop_minus_1,
                    op: mir::BinOp::Sub,
                    left: mir::Operand::Local(stop_local),
                    right: one,
                });

                let diff = self.alloc_typed_local(mir_func, Type::Int);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: diff,
                    op: mir::BinOp::Sub,
                    left: mir::Operand::Local(stop_minus_1),
                    right: mir::Operand::Local(start_local),
                });

                let step_local = self.alloc_typed_local(mir_func, Type::Int);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: step_local,
                    src: step,
                });

                let n_steps = self.alloc_typed_local(mir_func, Type::Int);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: n_steps,
                    op: mir::BinOp::FloorDiv,
                    left: mir::Operand::Local(diff),
                    right: mir::Operand::Local(step_local),
                });

                let offset = self.alloc_typed_local(mir_func, Type::Int);
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
}
