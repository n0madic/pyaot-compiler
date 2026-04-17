//! Comparison expression lowering: equality, ordering, identity, containment

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a comparison expression.
    pub(in crate::expressions) fn lower_compare(
        &mut self,
        left: hir::ExprId,
        op: hir::CmpOp,
        right: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let left_expr = &hir_module.exprs[left];
        let left_op = self.lower_expr(left_expr, hir_module, mir_func)?;
        let right_expr = &hir_module.exprs[right];
        let right_op = self.lower_expr(right_expr, hir_module, mir_func)?;

        // Get types for comparison detection
        let left_type = self.get_type_of_expr_id(left, hir_module);
        let right_type = self.get_type_of_expr_id(right, hir_module);

        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Handle identity operators first (is/is not) - pointer/value comparison
        if matches!(op, hir::CmpOp::Is | hir::CmpOp::IsNot) {
            // Check if either operand was originally a Union type (before narrowing)
            // A variable may have been narrowed from Union to a specific type, but still
            // holds a boxed pointer value that needs pointer comparison
            let left_was_union = left_type.is_union()
                || self.is_narrowed_union_var(left_expr)
                || left_expr.ty.as_ref().is_some_and(|t| t.is_union());
            let right_was_union = right_type.is_union()
                || self.is_narrowed_union_var(right_expr)
                || right_expr.ty.as_ref().is_some_and(|t| t.is_union());

            // For Union types (or narrowed Union variables), box both operands and use pointer comparison
            if left_was_union || right_was_union {
                // If the operand was originally a Union (or still is), it's already boxed
                // Only box if the operand was never a Union type
                let boxed_left = if left_was_union {
                    left_op.clone()
                } else {
                    self.box_primitive_if_needed(left_op.clone(), &left_type, mir_func)
                };

                let boxed_right = if right_was_union {
                    right_op.clone()
                } else {
                    self.box_primitive_if_needed(right_op.clone(), &right_type, mir_func)
                };

                // When one side is a `None` literal we can't use a pointer
                // equality test: the other operand may be a null pointer
                // (default-filled optional) OR a NoneObj singleton (user-
                // level `None` boxed across a module boundary). The runtime
                // `rt_is_none` collapses both representations to true.
                let is_left_none_literal = matches!(left_type, Type::None);
                let is_right_none_literal = matches!(right_type, Type::None);
                if is_left_none_literal ^ is_right_none_literal {
                    let non_none = if is_left_none_literal {
                        boxed_right
                    } else {
                        boxed_left
                    };
                    let is_none_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_IS_NONE),
                        vec![non_none],
                        Type::Bool,
                        mir_func,
                    );
                    if matches!(op, hir::CmpOp::IsNot) {
                        self.emit_instruction(mir::InstructionKind::UnOp {
                            dest: result_local,
                            op: mir::UnOp::Not,
                            operand: mir::Operand::Local(is_none_local),
                        });
                    } else {
                        self.emit_instruction(mir::InstructionKind::Copy {
                            dest: result_local,
                            src: mir::Operand::Local(is_none_local),
                        });
                    }
                    return Ok(mir::Operand::Local(result_local));
                }

                let mir_op = match op {
                    hir::CmpOp::Is => mir::BinOp::Eq,
                    hir::CmpOp::IsNot => mir::BinOp::NotEq,
                    _ => unreachable!(),
                };
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir_op,
                    left: boxed_left,
                    right: boxed_right,
                });
                return Ok(mir::Operand::Local(result_local));
            }

            // For non-Union types: If types are different and neither is None, identity
            // comparison is always False (or True for IsNot).
            // When one side is None, we must emit a runtime check because variables with
            // pointer types (list, dict, str, etc.) may hold a null pointer from a `= None` default.
            if left_type != right_type
                && !matches!(left_type, Type::None)
                && !matches!(right_type, Type::None)
            {
                let result_value = matches!(op, hir::CmpOp::IsNot);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(result_value)),
                });
                return Ok(mir::Operand::Local(result_local));
            }

            let mir_op = match op {
                hir::CmpOp::Is => mir::BinOp::Eq,
                hir::CmpOp::IsNot => mir::BinOp::NotEq,
                _ => unreachable!(),
            };
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: result_local,
                op: mir_op,
                left: left_op,
                right: right_op,
            });
            return Ok(mir::Operand::Local(result_local));
        }

        // Check if either operand is Union type - use runtime dispatch
        if left_type.is_union() || right_type.is_union() {
            // For Union types, box the non-Union operand if needed and use runtime comparison
            let boxed_left = if left_type.is_union() {
                left_op
            } else {
                self.box_primitive_if_needed(left_op, &left_type, mir_func)
            };

            let boxed_right = if right_type.is_union() {
                right_op
            } else {
                self.box_primitive_if_needed(right_op, &right_type, mir_func)
            };

            match op {
                hir::CmpOp::Eq => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        args: vec![boxed_left, boxed_right],
                    });
                }
                hir::CmpOp::NotEq => {
                    // Obj Eq + NOT
                    let eq_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        vec![boxed_left, boxed_right],
                        Type::Bool,
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(eq_local),
                    });
                }
                hir::CmpOp::Lt => {
                    let op_tag = mir::Operand::Constant(mir::Constant::Int(
                        mir::ComparisonOp::Lt.to_tag() as i64,
                    ));
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Lt),
                        ),
                        args: vec![boxed_left, boxed_right, op_tag],
                    });
                }
                hir::CmpOp::LtE => {
                    let op_tag = mir::Operand::Constant(mir::Constant::Int(
                        mir::ComparisonOp::Lte.to_tag() as i64,
                    ));
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Lte),
                        ),
                        args: vec![boxed_left, boxed_right, op_tag],
                    });
                }
                hir::CmpOp::Gt => {
                    let op_tag = mir::Operand::Constant(mir::Constant::Int(
                        mir::ComparisonOp::Gt.to_tag() as i64,
                    ));
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Gt),
                        ),
                        args: vec![boxed_left, boxed_right, op_tag],
                    });
                }
                hir::CmpOp::GtE => {
                    let op_tag = mir::Operand::Constant(mir::Constant::Int(
                        mir::ComparisonOp::Gte.to_tag() as i64,
                    ));
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Gte),
                        ),
                        args: vec![boxed_left, boxed_right, op_tag],
                    });
                }
                hir::CmpOp::In => {
                    // Use runtime dispatch for containment check on Union containers
                    // Note: boxed_right is the container, boxed_left is the element
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_OBJ_CONTAINS,
                        ),
                        args: vec![boxed_right, boxed_left], // (container, element)
                    });
                }
                hir::CmpOp::NotIn => {
                    // ObjContains + NOT
                    let contains_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_OBJ_CONTAINS),
                        vec![boxed_right, boxed_left], // (container, element)
                        Type::Bool,
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(contains_local),
                    });
                }
                hir::CmpOp::Is | hir::CmpOp::IsNot => {
                    // These are handled at the beginning of the function
                    unreachable!("Is/IsNot should be handled before reaching Union comparison");
                }
            }
            return Ok(mir::Operand::Local(result_local));
        }

        // Check if we're comparing strings
        if matches!(left_type, Type::Str) && matches!(right_type, Type::Str) {
            // String comparison - use runtime function
            match op {
                hir::CmpOp::Eq => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Str.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        args: vec![left_op, right_op],
                    });
                }
                hir::CmpOp::NotEq => {
                    // For !=, compute == and then negate
                    let eq_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(
                            mir::CompareKind::Str.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        vec![left_op, right_op],
                        Type::Bool,
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(eq_local),
                    });
                }
                hir::CmpOp::In => {
                    // String substring check: left in right (needle in haystack)
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_STR_CONTAINS,
                        ),
                        args: vec![left_op, right_op],
                    });
                }
                hir::CmpOp::NotIn => {
                    // String substring check negated: left not in right
                    let contains_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STR_CONTAINS),
                        vec![left_op, right_op],
                        Type::Bool,
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(contains_local),
                    });
                }
                _ => {
                    // For string ordering comparisons (< > <= >=), use Obj compare
                    // which dispatches via type tag for lexicographic comparison
                    let cmp_op = match op {
                        hir::CmpOp::Lt => mir::ComparisonOp::Lt,
                        hir::CmpOp::LtE => mir::ComparisonOp::Lte,
                        hir::CmpOp::Gt => mir::ComparisonOp::Gt,
                        hir::CmpOp::GtE => mir::ComparisonOp::Gte,
                        _ => unreachable!(),
                    };
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(cmp_op),
                        ),
                        args: vec![left_op, right_op],
                    });
                }
            }
        } else if matches!(left_type, Type::Bytes) && matches!(right_type, Type::Bytes) {
            // Bytes comparison - use runtime function
            match op {
                hir::CmpOp::Eq => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Bytes.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        args: vec![left_op, right_op],
                    });
                }
                hir::CmpOp::NotEq => {
                    // For !=, compute == and then negate
                    let eq_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(
                            mir::CompareKind::Bytes.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        vec![left_op, right_op],
                        Type::Bool,
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(eq_local),
                    });
                }
                _ => {
                    // For bytes ordering comparisons (< > <= >=), use Obj compare
                    // which dispatches via type tag for lexicographic comparison
                    let cmp_op = match op {
                        hir::CmpOp::Lt => mir::ComparisonOp::Lt,
                        hir::CmpOp::LtE => mir::ComparisonOp::Lte,
                        hir::CmpOp::Gt => mir::ComparisonOp::Gt,
                        hir::CmpOp::GtE => mir::ComparisonOp::Gte,
                        _ => unreachable!(),
                    };
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Obj.runtime_func_def(cmp_op),
                        ),
                        args: vec![left_op, right_op],
                    });
                }
            }
        } else if matches!(op, hir::CmpOp::In | hir::CmpOp::NotIn) {
            // "in" / "not in" operator
            // left is the element, right is the container
            let is_not_in = matches!(op, hir::CmpOp::NotIn);

            match right_type {
                Type::Dict(_, _) | Type::DefaultDict(_, _) => {
                    // key in dict/defaultdict - use rt_dict_contains
                    // Box key if needed (int/bool keys need boxing)
                    let boxed_key = self.box_primitive_if_needed(left_op, &left_type, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DICT_CONTAINS,
                        ),
                        args: vec![right_op, boxed_key], // (dict, key)
                    });
                }
                Type::Set(_) => {
                    // elem in set - use rt_set_contains
                    // Box element if needed (int/bool elements need boxing)
                    let boxed_elem = self.box_primitive_if_needed(left_op, &left_type, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_SET_CONTAINS,
                        ),
                        args: vec![right_op, boxed_elem], // (set, elem)
                    });
                }
                Type::List(_) => {
                    // elem in list - use rt_list_index and check if >= 0
                    let idx_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_INDEX),
                        vec![right_op, left_op], // (list, value)
                        Type::Int,
                        mir_func,
                    );
                    // result = idx >= 0
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir::BinOp::GtE,
                        left: mir::Operand::Local(idx_local),
                        right: mir::Operand::Constant(mir::Constant::Int(0)),
                    });
                }
                Type::Tuple(_) | Type::TupleVar(_) => {
                    // elem in tuple - use rt_obj_contains (needs boxed element)
                    let boxed_elem = self.box_primitive_if_needed(left_op, &left_type, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_OBJ_CONTAINS,
                        ),
                        args: vec![right_op, boxed_elem], // (tuple, elem)
                    });
                }
                Type::Class { class_id, .. } => {
                    // Class with __contains__ dunder
                    let contains_func = self
                        .get_class_info(&class_id)
                        .and_then(|info| info.get_dunder_func("__contains__"));

                    if let Some(func_id) = contains_func {
                        self.emit_instruction(mir::InstructionKind::CallDirect {
                            dest: result_local,
                            func: func_id,
                            args: vec![right_op, left_op], // (self=container, item)
                        });
                    } else {
                        self.emit_instruction(mir::InstructionKind::Copy {
                            dest: result_local,
                            src: mir::Operand::Constant(mir::Constant::Bool(false)),
                        });
                    }
                }
                _ => {
                    // Unsupported container type
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: mir::Operand::Constant(mir::Constant::Bool(false)),
                    });
                }
            }

            // For "not in", negate the result
            if is_not_in {
                let temp_local = self.alloc_and_add_local(Type::Bool, mir_func);
                // Swap: temp = result, result = !temp
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: temp_local,
                    src: mir::Operand::Local(result_local),
                });
                self.emit_instruction(mir::InstructionKind::UnOp {
                    dest: result_local,
                    op: mir::UnOp::Not,
                    operand: mir::Operand::Local(temp_local),
                });
            }
        } else if let Type::List(_) = &left_type {
            // List comparison - use runtime function based on element type
            if matches!(op, hir::CmpOp::Eq | hir::CmpOp::NotEq)
                && matches!(right_type, Type::List(_))
            {
                let is_not_eq = matches!(op, hir::CmpOp::NotEq);

                // Unified list equality — runtime dispatches by elem_tag from ListObj
                let eq_func = mir::RuntimeFunc::Call(
                    mir::CompareKind::List.runtime_func_def(mir::ComparisonOp::Eq),
                );

                if is_not_eq {
                    // NotEq: compute eq and negate
                    let eq_local = self.emit_runtime_call(
                        eq_func,
                        vec![left_op, right_op],
                        Type::Bool,
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(eq_local),
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: eq_func,
                        args: vec![left_op, right_op],
                    });
                }
            } else {
                // List ordering comparisons (<, <=, >, >=) use lexicographic comparison
                let compare_op = match op {
                    hir::CmpOp::Lt => mir::ComparisonOp::Lt,
                    hir::CmpOp::LtE => mir::ComparisonOp::Lte,
                    hir::CmpOp::Gt => mir::ComparisonOp::Gt,
                    hir::CmpOp::GtE => mir::ComparisonOp::Gte,
                    _ => unreachable!("Already handled Eq/NotEq above"),
                };
                let op_tag = mir::Operand::Constant(mir::Constant::Int(compare_op.to_tag() as i64));
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        mir::CompareKind::List.runtime_func_def(compare_op),
                    ),
                    args: vec![left_op, right_op, op_tag],
                });
            }
        } else if matches!(&left_type, Type::Tuple(_) | Type::TupleVar(_)) {
            // Tuple comparison - use runtime function for element-wise comparison
            // (works uniformly on both fixed and variable-length tuples since
            // rt_tuple_eq dispatches per element tag at runtime).
            if matches!(op, hir::CmpOp::Eq | hir::CmpOp::NotEq)
                && matches!(right_type, Type::Tuple(_) | Type::TupleVar(_))
            {
                let is_not_eq = matches!(op, hir::CmpOp::NotEq);

                if is_not_eq {
                    // NotEq: compute eq and negate
                    let eq_local = self.emit_runtime_call(
                        mir::RuntimeFunc::Call(
                            mir::CompareKind::Tuple.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        vec![left_op, right_op],
                        Type::Bool,
                        mir_func,
                    );
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(eq_local),
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            mir::CompareKind::Tuple.runtime_func_def(mir::ComparisonOp::Eq),
                        ),
                        args: vec![left_op, right_op],
                    });
                }
            } else {
                // For ordering comparisons on tuples, use runtime lexicographic comparison
                let compare_op = match op {
                    hir::CmpOp::Lt => mir::ComparisonOp::Lt,
                    hir::CmpOp::LtE => mir::ComparisonOp::Lte,
                    hir::CmpOp::Gt => mir::ComparisonOp::Gt,
                    hir::CmpOp::GtE => mir::ComparisonOp::Gte,
                    _ => unreachable!("Already handled Eq/NotEq above"),
                };

                let op_tag = mir::Operand::Constant(mir::Constant::Int(compare_op.to_tag() as i64));
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        mir::CompareKind::Tuple.runtime_func_def(compare_op),
                    ),
                    args: vec![left_op, right_op, op_tag],
                });
            }
        } else if let Type::Class { class_id, .. } = &left_type {
            // Class type: dispatch to dunder methods if available.
            // `CmpOp::dunder_name()` is the single source of truth for
            // op → dunder mapping; `__ne__` falls back to `__eq__` below.
            let dunder_func = if let Some(class_info) = self.get_class_info(class_id) {
                match op.dunder_name() {
                    Some("__ne__") => class_info
                        .get_dunder_func("__ne__")
                        .or_else(|| class_info.get_dunder_func("__eq__")),
                    Some(name) => class_info.get_dunder_func(name),
                    None => None,
                }
            } else {
                None
            };

            if let Some(func_id) = dunder_func {
                // For NotEq: prefer __ne__ if available, otherwise use __eq__ + negate
                let use_eq_negated = matches!(op, hir::CmpOp::NotEq)
                    && self
                        .get_class_info(class_id)
                        .and_then(|ci| ci.get_dunder_func("__ne__"))
                        .is_none();

                let may_return_ni = self.func_may_return_not_implemented(func_id, hir_module);

                // Size the dunder result using the function's actual
                // return type (Bool, or Union[Bool, NotImplementedT]
                // when the dunder has any `return NotImplemented` arm).
                let dest = self.alloc_dunder_result(func_id, &Type::Bool, hir_module, mir_func);
                let boxed_right = self.box_dunder_arg_if_needed(
                    right_op.clone(),
                    &right_type,
                    func_id,
                    1,
                    hir_module,
                    mir_func,
                );
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest,
                    func: func_id,
                    args: vec![left_op.clone(), boxed_right],
                });

                let bool_result = if may_return_ni {
                    // §E.7 — dunder may return NotImplemented. Branch on
                    // identity with the singleton; on NI, dispatch the
                    // reflected dunder (or identity / TypeError fallback
                    // when none applies). Otherwise unbox the boxed Bool.
                    let reflected_name = op.reflected_dunder_name();
                    let reflected_func = match (&right_type, reflected_name) {
                        (Type::Class { class_id: r_id, .. }, Some(name)) => self
                            .get_class_info(r_id)
                            .and_then(|ci| ci.get_dunder_func(name)),
                        _ => None,
                    };
                    self.emit_comparison_ni_fallback(
                        dest,
                        reflected_func,
                        right_op,
                        right_type.clone(),
                        left_op,
                        left_type.clone(),
                        op,
                        hir_module,
                        mir_func,
                    )
                } else {
                    dest
                };

                if use_eq_negated {
                    let negated = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: negated,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(bool_result),
                    });
                    return Ok(mir::Operand::Local(negated));
                }
                return Ok(mir::Operand::Local(bool_result));
            }

            // No dunder method — fall through to default comparison
            let mir_op = match op {
                hir::CmpOp::Eq => mir::BinOp::Eq,
                hir::CmpOp::NotEq => mir::BinOp::NotEq,
                hir::CmpOp::Lt => mir::BinOp::Lt,
                hir::CmpOp::LtE => mir::BinOp::LtE,
                hir::CmpOp::Gt => mir::BinOp::Gt,
                hir::CmpOp::GtE => mir::BinOp::GtE,
                _ => unreachable!(),
            };
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: result_local,
                op: mir_op,
                left: left_op,
                right: right_op,
            });
        } else if matches!(left_type, Type::HeapAny) || matches!(right_type, Type::HeapAny) {
            // HeapAny comparison: runtime dispatch via rt_obj_eq/lt/etc.
            // Box the other operand if primitive.
            let boxed_left = self.box_primitive_if_needed(left_op, &left_type, mir_func);
            let boxed_right = self.box_primitive_if_needed(right_op, &right_type, mir_func);
            if matches!(op, hir::CmpOp::NotEq) {
                let eq_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Eq),
                    ),
                    vec![boxed_left, boxed_right],
                    Type::Bool,
                    mir_func,
                );
                self.emit_instruction(mir::InstructionKind::UnOp {
                    dest: result_local,
                    op: mir::UnOp::Not,
                    operand: mir::Operand::Local(eq_local),
                });
            } else {
                let compare_op = match op {
                    hir::CmpOp::Eq => mir::ComparisonOp::Eq,
                    hir::CmpOp::Lt => mir::ComparisonOp::Lt,
                    hir::CmpOp::LtE => mir::ComparisonOp::Lte,
                    hir::CmpOp::Gt => mir::ComparisonOp::Gt,
                    hir::CmpOp::GtE => mir::ComparisonOp::Gte,
                    _ => unreachable!(),
                };
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        mir::CompareKind::Obj.runtime_func_def(compare_op),
                    ),
                    args: vec![boxed_left, boxed_right],
                });
            }
        } else {
            // Primitives and raw Any comparison
            let mir_op = match op {
                hir::CmpOp::Eq => mir::BinOp::Eq,
                hir::CmpOp::NotEq => mir::BinOp::NotEq,
                hir::CmpOp::Lt => mir::BinOp::Lt,
                hir::CmpOp::LtE => mir::BinOp::LtE,
                hir::CmpOp::Gt => mir::BinOp::Gt,
                hir::CmpOp::GtE => mir::BinOp::GtE,
                hir::CmpOp::In | hir::CmpOp::NotIn | hir::CmpOp::Is | hir::CmpOp::IsNot => {
                    unreachable!()
                }
            };

            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: result_local,
                op: mir_op,
                left: left_op,
                right: right_op,
            });
        }
        Ok(mir::Operand::Local(result_local))
    }

    /// Emit the §E.7 comparison-dunder NotImplemented fallback.
    ///
    /// `forward_result` holds the outcome of the forward dunder, typed
    /// as `Union[Bool, NotImplementedT]`. Control flow:
    ///
    /// ```ignore
    /// if forward_result is NotImplemented:
    ///     if reflected_func.is_some():
    ///         refl = right.__rop__(left)
    ///         if refl is NotImplemented:
    ///             <default fallback>
    ///         else:
    ///             final = unbox_bool(refl)
    ///     else:
    ///         <default fallback>
    /// else:
    ///     final = unbox_bool(forward_result)
    /// ```
    ///
    /// Default fallback:
    ///   - `Eq`  → identity-pointer-eq (`left == right`).
    ///   - `NotEq` → identity-pointer-ne.
    ///   - ordering → raise `TypeError`.
    ///
    /// Returns the `Bool` local that holds the final comparison result.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::expressions) fn emit_comparison_ni_fallback(
        &mut self,
        forward_result: pyaot_utils::LocalId,
        reflected_func: Option<pyaot_utils::FuncId>,
        right_op: mir::Operand,
        right_ty: Type,
        left_op: mir::Operand,
        left_ty: Type,
        op: hir::CmpOp,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> pyaot_utils::LocalId {
        let final_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // is_ni = forward_result == NotImplementedSingleton (pointer eq).
        let ni_local = self.alloc_and_add_local(Type::NotImplementedT, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: ni_local,
            func: mir::RuntimeFunc::Call(
                &pyaot_core_defs::runtime_func_def::RT_NOT_IMPLEMENTED_SINGLETON,
            ),
            args: vec![],
        });
        let is_ni = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: is_ni,
            op: mir::BinOp::Eq,
            left: mir::Operand::Local(forward_result),
            right: mir::Operand::Local(ni_local),
        });

        let ni_path = self.new_block();
        let ok_path = self.new_block();
        let cont = self.new_block();
        let ni_id = ni_path.id;
        let ok_id = ok_path.id;
        let cont_id = cont.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(is_ni),
            then_block: ni_id,
            else_block: ok_id,
        };

        // `then` (forward returned NI): try reflected, else default fallback.
        self.push_block(ni_path);
        match reflected_func {
            Some(rfunc) => {
                let boxed_left = self.box_dunder_arg_if_needed(
                    left_op.clone(),
                    &left_ty,
                    rfunc,
                    1,
                    hir_module,
                    mir_func,
                );
                let refl_dest = self.alloc_dunder_result(rfunc, &Type::Bool, hir_module, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: refl_dest,
                    func: rfunc,
                    args: vec![right_op.clone(), boxed_left],
                });
                let refl_may_ni = self.func_may_return_not_implemented(rfunc, hir_module);
                if refl_may_ni {
                    // Second NI check on reflected result.
                    let ni2 = self.alloc_and_add_local(Type::NotImplementedT, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: ni2,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_NOT_IMPLEMENTED_SINGLETON,
                        ),
                        args: vec![],
                    });
                    let refl_is_ni = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: refl_is_ni,
                        op: mir::BinOp::Eq,
                        left: mir::Operand::Local(refl_dest),
                        right: mir::Operand::Local(ni2),
                    });
                    let both_ni = self.new_block();
                    let refl_ok = self.new_block();
                    let both_ni_id = both_ni.id;
                    let refl_ok_id = refl_ok.id;
                    self.current_block_mut().terminator = mir::Terminator::Branch {
                        cond: mir::Operand::Local(refl_is_ni),
                        then_block: both_ni_id,
                        else_block: refl_ok_id,
                    };
                    self.push_block(both_ni);
                    self.emit_default_compare_fallback(
                        final_local,
                        &left_op,
                        &left_ty,
                        &right_op,
                        &right_ty,
                        op,
                        mir_func,
                    );
                    self.current_block_mut().terminator = mir::Terminator::Goto(cont_id);
                    self.push_block(refl_ok);
                    self.emit_unbox_into_bool(final_local, refl_dest, mir_func);
                    self.current_block_mut().terminator = mir::Terminator::Goto(cont_id);
                } else {
                    self.emit_unbox_into_bool(final_local, refl_dest, mir_func);
                    self.current_block_mut().terminator = mir::Terminator::Goto(cont_id);
                }
            }
            None => {
                self.emit_default_compare_fallback(
                    final_local,
                    &left_op,
                    &left_ty,
                    &right_op,
                    &right_ty,
                    op,
                    mir_func,
                );
                self.current_block_mut().terminator = mir::Terminator::Goto(cont_id);
            }
        }

        // `else` (forward produced a concrete Bool): unbox.
        self.push_block(ok_path);
        self.emit_unbox_into_bool(final_local, forward_result, mir_func);
        self.current_block_mut().terminator = mir::Terminator::Goto(cont_id);

        self.push_block(cont);
        final_local
    }

    /// Unbox a boxed-bool `*mut Obj` into the typed `Bool` local.
    fn emit_unbox_into_bool(
        &mut self,
        dest: pyaot_utils::LocalId,
        src: pyaot_utils::LocalId,
        mir_func: &mut mir::Function,
    ) {
        let tmp = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: tmp,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_UNBOX_BOOL),
            args: vec![mir::Operand::Local(src)],
        });
        self.emit_instruction(mir::InstructionKind::Copy {
            dest,
            src: mir::Operand::Local(tmp),
        });
    }

    /// Emit the "both sides returned NotImplemented" fallback.
    /// Equality ops fall back to identity (pointer equality); ordering
    /// ops raise `TypeError`.
    #[allow(clippy::too_many_arguments)]
    fn emit_default_compare_fallback(
        &mut self,
        dest: pyaot_utils::LocalId,
        left_op: &mir::Operand,
        _left_ty: &Type,
        right_op: &mir::Operand,
        _right_ty: &Type,
        op: hir::CmpOp,
        _mir_func: &mut mir::Function,
    ) {
        if op.is_ordering() {
            // TypeError — CPython shape: "'<' not supported between
            // instances of 'X' and 'Y'". We elide the class names here
            // (not trivially available at lowering time) and emit the
            // canonical op string.
            let msg = match op {
                hir::CmpOp::Lt => "'<' not supported between instances",
                hir::CmpOp::LtE => "'<=' not supported between instances",
                hir::CmpOp::Gt => "'>' not supported between instances",
                hir::CmpOp::GtE => "'>=' not supported between instances",
                _ => unreachable!(),
            };
            let msg_interned = self.interner.intern(msg);
            self.current_block_mut().terminator = mir::Terminator::Raise {
                exc_type: pyaot_core_defs::exceptions::BuiltinExceptionKind::TypeError.tag(),
                message: Some(mir::Operand::Constant(mir::Constant::Str(msg_interned))),
                cause: None,
                suppress_context: false,
            };
            // Continue codegen in an unreachable block to satisfy the
            // rest of the fallback's control-flow structure (it
            // expects to Goto the continuation block).
            let unreachable_bb = self.new_block();
            self.push_block(unreachable_bb);
            // Dead store so `dest` remains typed.
            self.emit_instruction(mir::InstructionKind::Copy {
                dest,
                src: mir::Operand::Constant(mir::Constant::Bool(false)),
            });
        } else {
            // Equality: identity pointer comparison.
            let op_kind = match op {
                hir::CmpOp::Eq => mir::BinOp::Eq,
                hir::CmpOp::NotEq => mir::BinOp::NotEq,
                _ => unreachable!(),
            };
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest,
                op: op_kind,
                left: left_op.clone(),
                right: right_op.clone(),
            });
        }
    }
}
