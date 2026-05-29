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
                // abs(float) -> use FloatAbs instruction.
                //
                // The seed type says `Float` but the operand may have
                // been lowered through a path (e.g. Union arithmetic via
                // `rt_obj_*`) that produced a tagged-`Value` (i64) MIR
                // local. `FloatAbs` requires an f64 operand — feed in
                // an `rt_unbox_float` (whose ABI shim handles tagged
                // INT / BOOL / Float-pointer uniformly) when the
                // operand's MIR type isn't already `Float`.
                let operand_mir_ty = self.operand_type(&arg_operand, mir_func);
                let coerced_operand = if matches!(operand_mir_ty, Type::Float) {
                    arg_operand
                } else {
                    let unboxed = self.alloc_and_add_local(Type::Float, mir_func);
                    self.emit_instruction(mir::InstructionKind::UnboxValue {
                        dest: unboxed,
                        src: arg_operand,
                        dest_type: Type::Float,
                    });
                    mir::Operand::Local(unboxed)
                };
                let result_local = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::FloatAbs {
                    dest: result_local,
                    src: coerced_operand,
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

        // RT_POW_FLOAT takes two raw f64 operands. Coerce each operand based on
        // its actual MIR representation rather than blindly emitting IntToFloat,
        // which hard-errors the verifier on a Bool (Raw I8) and reinterprets a
        // tagged-Value (Any/Union) pointer's bits as an integer.
        let base_float = self.coerce_operand_to_f64(base_operand, mir_func);
        let exp_float = self.coerce_operand_to_f64(exp_operand, mir_func);

        // Create result local and emit runtime call
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_POW_FLOAT),
            vec![base_float, exp_float],
            Type::Float,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Coerce an operand to a raw `f64` for a runtime call that takes f64
    /// (e.g. `RT_POW_FLOAT`). Dispatches on the operand's *physical* MIR
    /// representation so the result is verifier-clean:
    /// - `Raw(F64)` → pass through (already an f64)
    /// - `Raw(I64)` (Int) → `IntToFloat`
    /// - `Raw(I8)` (Bool) → `BoolToInt` then `IntToFloat`
    /// - `Tagged` / anything else (`Any`, `Union`, tagged-`Float`) →
    ///   `UnboxValue { dest_type: Float }`, which routes through the
    ///   tag-dispatching `rt_unbox_float` ABI shim (handles tagged INT / BOOL /
    ///   FloatObj). The old code blindly emitted `IntToFloat`, which
    ///   hard-errors the verifier on a Bool (Raw I8) and reinterprets tagged
    ///   pointer bits as an integer.
    fn coerce_operand_to_f64(
        &mut self,
        operand: mir::Operand,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        use pyaot_mir::{MirType, RawKind};
        let raw_kind = match &operand {
            mir::Operand::Constant(mir::Constant::Float(_)) => Some(RawKind::F64),
            mir::Operand::Constant(mir::Constant::Int(_)) => Some(RawKind::I64),
            mir::Operand::Constant(mir::Constant::Bool(_)) => Some(RawKind::I8),
            mir::Operand::Constant(_) => None,
            mir::Operand::Local(id) => match mir_func.locals.get(id).map(|l| l.resolved_mir_type())
            {
                Some(MirType::Raw(k)) => Some(k),
                _ => None,
            },
        };
        match raw_kind {
            Some(RawKind::F64) => operand,
            Some(RawKind::I64) => {
                let dest = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::IntToFloat { dest, src: operand });
                mir::Operand::Local(dest)
            }
            Some(RawKind::I8) => {
                let temp_int = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BoolToInt {
                    dest: temp_int,
                    src: operand,
                });
                let dest = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::IntToFloat {
                    dest,
                    src: mir::Operand::Local(temp_int),
                });
                mir::Operand::Local(dest)
            }
            _ => {
                let dest = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::UnboxValue {
                    dest,
                    src: operand,
                    dest_type: Type::Float,
                });
                mir::Operand::Local(dest)
            }
        }
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

        // Element-type extraction unified through the shared helper.
        // Handles list / tuple (fixed + var) / set / dict-keys / str /
        // bytes / iterator / Union end-to-end so the lowering path
        // matches `resolve_builtin_call_type::Sum` (helpers.rs:629).
        //
        // For tuple/set/dict/str/bytes iterables the unified Iterator
        // branch below converts the container to an `IteratorObj` via
        // the appropriate `rt_iter_*` factory and runs the standard
        // next/exhausted loop — so the element-type extraction here
        // must reflect the actual yield type rather than fall back to
        // `Type::Int` (the prior narrow form was a placeholder for the
        // pre-alpha "list-only sum" limitation).
        let element_type =
            crate::type_planning::infer::extract_iterable_element_type(&iterable_type);

        // Check if start value is provided and its type
        let start_type = if args.len() > 1 {
            self.seed_expr_type(args[1], hir_module)
        } else {
            Type::Int // default start is 0 (int)
        };

        // Result-type classification (3-way). `classify_reduction_elem`
        // gives `Float` when the element or start is float — or a `Union`
        // containing `Float`, the microgpt polymorphic-dunder seed
        // `Union[Float, Class[Self]]`; `Tagged` when the element/start is
        // bare `Any`, in which case the accumulator stays a tagged `Value`
        // and folds through `rt_obj_add` so int vs float is preserved per
        // CPython; `Int` otherwise. Must agree with `lower_minmax_builtin`
        // and `resolve_builtin_call_type`.
        use crate::type_planning::helpers::{classify_reduction_elem, ReductionResult};
        let result_kind =
            classify_reduction_elem(&element_type).join(classify_reduction_elem(&start_type));
        let result_type = result_kind.result_type();
        let accumulate_tagged = result_kind == ReductionResult::Tagged;

        // Get the start value in the representation the accumulator wants.
        let start_operand = if args.len() > 1 {
            let start_expr = &hir_module.exprs[args[1]];
            let start_op = self.lower_expr(start_expr, hir_module, mir_func)?;

            if accumulate_tagged {
                // Tagged accumulator: the start must be a tagged `Value`.
                // Route through `emit_value_slot` so every primitive start
                // (Int / Bool / Float / None) gets boxed to a tagged slot
                // and every already-tagged operand passes through
                // unchanged. Previously this branch boxed Int/Bool but
                // passed a raw F64 start through verbatim — when
                // `sum(any_iter, 1.5)` selected the Tagged accumulator
                // (now the join's preferred result), codegen's
                // `store_result` then panicked on the
                // expected(Tagged=I64) vs actual(F64) mismatch.
                self.emit_value_slot(start_op, &start_type, mir_func)
            } else if result_type == Type::Float && start_type != Type::Float {
                // Promote an int start to float.
                let temp = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::IntToFloat {
                    dest: temp,
                    src: start_op,
                });
                mir::Operand::Local(temp)
            } else {
                start_op
            }
        } else if accumulate_tagged {
            // Default start `0` as a tagged int `Value` (CPython `sum([])` == 0).
            let zero = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: zero,
                src: mir::Operand::Constant(mir::Constant::Int(0)),
            });
            let boxed = self.alloc_and_add_local(Type::Any, mir_func);
            self.emit_instruction(mir::InstructionKind::BoxValue {
                dest: boxed,
                src: mir::Operand::Local(zero),
                src_type: Type::Int,
            });
            mir::Operand::Local(boxed)
        } else if result_type == Type::Float {
            mir::Operand::Constant(mir::Constant::Float(0.0))
        } else {
            mir::Operand::Constant(mir::Constant::Int(0))
        };

        // Create the result accumulator and initialise it from the start
        // value. A tagged accumulator must be GC-tracked: `rt_obj_add` can
        // box a float result, so the slot may hold a heap pointer.
        let result_local = if accumulate_tagged {
            self.alloc_and_add_local(Type::Any, mir_func)
        } else {
            self.alloc_and_add_local(result_type.clone(), mir_func)
        };
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: start_operand,
        });

        // Lower the iterable
        let iterable_operand =
            self.lower_expr_expecting(iterable_expr, None, hir_module, mir_func)?;

        // Unified Iterator path for everything except `list` (which keeps
        // its dedicated indexed-access fast-path below). For
        // set/tuple/tuple-var/dict/str/bytes containers we materialise an
        // `IteratorObj` via the appropriate `rt_iter_*` factory and run
        // the same next/exhausted loop as for `iter()` results and
        // generators.
        //
        // Previously the function fell through to the List path's
        // `rt_list_len + rt_list_get` for these containers, which read a
        // SetObj / TupleObj as if it were a ListObj — returning garbage
        // values (review finding #2). Now the conversion is explicit and
        // exhaustive for the supported container shapes.
        let iter_factory_def: Option<&'static pyaot_core_defs::runtime_func_def::RuntimeFuncDef> = {
            use pyaot_core_defs::runtime_func_def as r;
            if matches!(iterable_type, Type::Iterator(_)) {
                None // already an iterator — copy iterable_operand directly
            } else if iterable_type.set_elem().is_some() {
                Some(&r::RT_ITER_SET)
            } else if iterable_type.tuple_elems().is_some()
                || iterable_type.tuple_var_elem().is_some()
            {
                Some(&r::RT_ITER_TUPLE)
            } else if iterable_type.dict_kv().is_some() {
                Some(&r::RT_ITER_DICT)
            } else {
                // `Str` / `Bytes` are intentionally NOT supported as `sum`
                // iterables: CPython raises `TypeError` for `sum("abc")`
                // (the int accumulator + str element has no `__add__`),
                // and emitting the iterator branch here would just trade
                // garbage for a verifier reject downstream.
                None
            }
        };

        let use_iter_branch =
            matches!(iterable_type, Type::Iterator(_)) || iter_factory_def.is_some();

        // Iterator path: use IterNextNoExc + IterIsExhausted protocol.
        // `rt_iter_is_exhausted` is the universal predicate that handles
        // both `IteratorObj` (set/tuple/dict/str/bytes/list iterators
        // created via the factory above) AND `GeneratorObj` (generator
        // function call results) by dispatching on the runtime type tag.
        // Previously this branch used `rt_generator_is_exhausted` which
        // is GeneratorObj-only — that worked for pre-existing
        // `iter()`/generator inputs but would have been wrong for any
        // factory-converted container.
        if use_iter_branch {
            let iter_local = self.alloc_and_add_local(Type::Any, mir_func);
            let iter_seed = if let Some(factory) = iter_factory_def {
                self.emit_runtime_call_gc(
                    mir::RuntimeFunc::Call(factory),
                    vec![iterable_operand],
                    Type::Any,
                    mir_func,
                )
            } else {
                // iterable_operand is already an IteratorObj / GeneratorObj.
                let tmp = self.alloc_and_add_local(Type::Any, mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: tmp,
                    src: iterable_operand,
                });
                tmp
            };
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: iter_local,
                src: mir::Operand::Local(iter_seed),
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

            // After §F.7c BigBang: iter_next returns tagged Value bits; unbox
            // for typed Int/Bool element types so BinOp Add sees raw scalars.
            let raw_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
                vec![mir::Operand::Local(iter_local)],
                Type::Any,
                mir_func,
            );

            let exhausted_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_IS_EXHAUSTED),
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

            let next_local = match &element_type {
                Type::Int => {
                    let dest = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::UnboxValue {
                        dest,
                        src: mir::Operand::Local(raw_local),
                        dest_type: Type::Int,
                    });
                    dest
                }
                Type::Bool => {
                    let dest = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::UnboxValue {
                        dest,
                        src: mir::Operand::Local(raw_local),
                        dest_type: Type::Bool,
                    });
                    dest
                }
                // `rt_iter_next_no_exc` returns a tagged Value — for a
                // Float-element iterator the bits are a `*mut FloatObj`
                // pointer, not a raw f64. Without this unboxing step the
                // raw pointer would feed `BinOp::Add` (whose codegen
                // bitcasts I64→F64) and produce a denormal garbage
                // accumulator. Pre-existing latent bug — never
                // triggered before because the Iterator branch only ran
                // for `Type::Iterator(_)` inputs, which in practice
                // yielded Int elements. Now reachable through the
                // set/tuple/dict factories below.
                Type::Float => {
                    let dest = self.alloc_and_add_local(Type::Float, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                        ),
                        args: vec![mir::Operand::Local(raw_local)],
                    });
                    dest
                }
                _ => raw_local,
            };

            let item_operand = if result_type == Type::Float && element_type != Type::Float {
                let temp = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::IntToFloat {
                    dest: temp,
                    src: mir::Operand::Local(next_local),
                });
                mir::Operand::Local(temp)
            } else {
                mir::Operand::Local(next_local)
            };

            // Accumulate: the tagged path dispatches through `rt_obj_add`
            // (runtime preserves int vs float by tag); the typed path uses
            // a raw `BinOp::Add`.
            let temp_result = if accumulate_tagged {
                self.emit_runtime_call_gc(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_OBJ_ADD),
                    vec![mir::Operand::Local(result_local), item_operand],
                    Type::Any,
                    mir_func,
                )
            } else {
                let dest = self.alloc_and_add_local(result_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest,
                    op: mir::BinOp::Add,
                    left: mir::Operand::Local(result_local),
                    right: item_operand,
                });
                dest
            };

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

        // Get item at index; emit_list_get handles Int/Bool/Float unwrapping.
        let item_local = self.emit_list_get(
            iterable_operand.clone(),
            mir::Operand::Local(counter_local),
            &element_type,
            mir_func,
        );
        let unboxed_item = mir::Operand::Local(item_local);

        // Promote item to float if needed (when summing int list with float start)
        let item_operand = if result_type == Type::Float && element_type != Type::Float {
            let temp = self.alloc_and_add_local(Type::Float, mir_func);
            self.emit_instruction(mir::InstructionKind::IntToFloat {
                dest: temp,
                src: unboxed_item,
            });
            mir::Operand::Local(temp)
        } else {
            unboxed_item
        };

        // result = result + item — tagged path via `rt_obj_add`, typed
        // path via raw `BinOp::Add` (see the iterator branch above).
        let temp_result = if accumulate_tagged {
            self.emit_runtime_call_gc(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_OBJ_ADD),
                vec![mir::Operand::Local(result_local), item_operand],
                Type::Any,
                mir_func,
            )
        } else {
            let dest = self.alloc_and_add_local(result_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest,
                op: mir::BinOp::Add,
                left: mir::Operand::Local(result_local),
                right: item_operand,
            });
            dest
        };

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

        // After §F.7c: tuples store uniform tagged Values; box every primitive.
        let _ = is_float;
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
            vec![mir::Operand::Constant(mir::Constant::Int(2))],
            Type::tuple_of(vec![result_elem_ty.clone(), result_elem_ty.clone()]),
            mir_func,
        );

        let quot_operand =
            self.emit_value_slot(mir::Operand::Local(quot_local), &result_elem_ty, mir_func);
        let rem_operand =
            self.emit_value_slot(mir::Operand::Local(rem_local), &result_elem_ty, mir_func);

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
