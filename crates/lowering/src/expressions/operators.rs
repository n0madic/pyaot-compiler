//! Operator expression lowering: BinOp, Compare, UnOp, LogicalOp

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

/// Minimum number of string operands to use StringBuilder pattern
/// Below this threshold, regular StrConcat is used (simpler and still efficient for 2 strings)
const STRING_BUILDER_THRESHOLD: usize = 3;

impl<'a> Lowering<'a> {
    /// Collect all string operands from a left-associative concatenation chain.
    /// Returns true if the expression is a string add operation, false otherwise.
    /// Operands are collected left-to-right (evaluation order).
    fn collect_str_concat_chain(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
        chain: &mut Vec<hir::ExprId>,
    ) -> bool {
        let expr = &hir_module.exprs[expr_id];

        // Check if this is a string add operation
        if let hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left,
            right,
        } = &expr.kind
        {
            let left_type = self.get_expr_type(&hir_module.exprs[*left], hir_module);
            if matches!(left_type, Type::Str) {
                // Recursively collect from left side first (left-to-right evaluation)
                self.collect_str_concat_chain(*left, hir_module, chain);
                // Then add the right operand
                chain.push(*right);
                return true;
            }
        }

        // Not a string add - this is a leaf node, add it to the chain
        chain.push(expr_id);
        false
    }

    /// Lower a binary operation expression.
    pub(super) fn lower_binop(
        &mut self,
        op: hir::BinOp,
        left: hir::ExprId,
        right: hir::ExprId,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Get operand types using get_expr_type for better inference
        let left_expr = &hir_module.exprs[left];
        let left_ty = self.get_expr_type(left_expr, hir_module);

        // Check for string concatenation chain optimization
        if matches!(op, hir::BinOp::Add) && matches!(left_ty, Type::Str) {
            // Collect the full chain starting from the current expression
            let mut chain = Vec::new();

            // Get the current expression's ID from the HIR module
            // We need to reconstruct the chain from the current BinOp
            self.collect_str_concat_chain(left, hir_module, &mut chain);
            chain.push(right);

            // If we have 3+ operands, use StringBuilder
            if chain.len() >= STRING_BUILDER_THRESHOLD {
                return self.lower_str_concat_with_builder(&chain, hir_module, mir_func);
            }
        }

        // Standard lowering path
        let left_op = self.lower_expr(left_expr, hir_module, mir_func)?;
        let right_expr = &hir_module.exprs[right];
        let right_op = self.lower_expr(right_expr, hir_module, mir_func)?;

        let right_ty = self.get_expr_type(right_expr, hir_module);

        // Infer result type based on operand types
        let result_ty = if matches!(left_ty, Type::Class { .. }) {
            // Class with arithmetic dunders returns the class type
            left_ty.clone()
        } else if matches!(right_ty, Type::Class { .. }) {
            // Reverse dunder case: right operand is a class, result is that class type
            right_ty.clone()
        } else if matches!(left_ty, Type::Str) {
            Type::Str // String operations return strings
        } else if matches!(left_ty, Type::Bytes) && matches!(op, hir::BinOp::Add | hir::BinOp::Mul)
        {
            Type::Bytes // Bytes operations return bytes
        } else if matches!(op, hir::BinOp::Div) {
            // Python 3: true division always returns float
            Type::Float
        } else if matches!(left_ty, Type::Float) || matches!(right_ty, Type::Float) {
            // Float + anything numeric = Float
            Type::Float
        } else if matches!(left_ty, Type::Int) && matches!(right_ty, Type::Int) {
            Type::Int
        } else {
            expr.ty.clone().unwrap_or(Type::Any)
        };
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Check for string operations
        if matches!(left_ty, Type::Str) {
            match op {
                hir::BinOp::Add => {
                    // String concatenation (2 operands - use simple concat)
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::StrConcat,
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                hir::BinOp::Mul => {
                    // String multiplication: "abc" * 3
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::StrMul,
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                _ => {}
            }
        }

        // Check for bytes operations (concatenation, repetition)
        if matches!(left_ty, Type::Bytes) {
            match op {
                hir::BinOp::Add => {
                    // Bytes concatenation
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::BytesConcat,
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                hir::BinOp::Mul => {
                    // Bytes repetition: b"abc" * 3
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::BytesRepeat,
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                _ => {}
            }
        }

        // Check for list concatenation (+)
        if let Type::List(elem_ty) = &left_ty {
            if matches!(op, hir::BinOp::Add) {
                let list_result = self.alloc_and_add_local(Type::List(elem_ty.clone()), mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: list_result,
                    func: mir::RuntimeFunc::ListConcat,
                    args: vec![left_op, right_op],
                });
                return Ok(mir::Operand::Local(list_result));
            }
        }

        // Check for dict merge operation (|)
        if let Type::Dict(key_ty, value_ty) = &left_ty {
            if matches!(op, hir::BinOp::BitOr) {
                let dict_result = self
                    .alloc_and_add_local(Type::Dict(key_ty.clone(), value_ty.clone()), mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dict_result,
                    func: mir::RuntimeFunc::DictMerge,
                    args: vec![left_op, right_op],
                });
                return Ok(mir::Operand::Local(dict_result));
            }
        }

        // Check for set operations (|, &, -, ^)
        if let Type::Set(elem_ty) = &left_ty {
            let set_func = match op {
                hir::BinOp::BitOr => Some(mir::RuntimeFunc::SetUnion),
                hir::BinOp::BitAnd => Some(mir::RuntimeFunc::SetIntersection),
                hir::BinOp::Sub => Some(mir::RuntimeFunc::SetDifference),
                hir::BinOp::BitXor => Some(mir::RuntimeFunc::SetSymmetricDifference),
                _ => None,
            };
            if let Some(runtime_func) = set_func {
                let set_result = self.alloc_and_add_local(Type::Set(elem_ty.clone()), mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: set_result,
                    func: runtime_func,
                    args: vec![left_op, right_op],
                });
                return Ok(mir::Operand::Local(set_result));
            }
        }

        // Check for class type with arithmetic dunders
        if let Type::Class { class_id, .. } = &left_ty {
            let dunder_func = if let Some(class_info) = self.get_class_info(class_id) {
                match op {
                    hir::BinOp::Add => class_info.add_func,
                    hir::BinOp::Sub => class_info.sub_func,
                    hir::BinOp::Mul => class_info.mul_func,
                    hir::BinOp::Div => class_info.truediv_func,
                    hir::BinOp::FloorDiv => class_info.floordiv_func,
                    hir::BinOp::Mod => class_info.mod_func,
                    hir::BinOp::Pow => class_info.pow_func,
                    hir::BinOp::BitAnd => class_info.and_func,
                    hir::BinOp::BitOr => class_info.or_func,
                    hir::BinOp::BitXor => class_info.xor_func,
                    hir::BinOp::LShift => class_info.lshift_func,
                    hir::BinOp::RShift => class_info.rshift_func,
                    hir::BinOp::MatMul => class_info.matmul_func,
                }
            } else {
                None
            };

            if let Some(func_id) = dunder_func {
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: func_id,
                    args: vec![left_op, right_op],
                });
                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Check for right operand's reverse arithmetic dunders
        // e.g., 2 + custom_obj -> custom_obj.__radd__(2)
        if let Type::Class { class_id, .. } = &right_ty {
            let rdunder_func = if let Some(class_info) = self.get_class_info(class_id) {
                match op {
                    hir::BinOp::Add => class_info.radd_func,
                    hir::BinOp::Sub => class_info.rsub_func,
                    hir::BinOp::Mul => class_info.rmul_func,
                    hir::BinOp::Div => class_info.rtruediv_func,
                    hir::BinOp::FloorDiv => class_info.rfloordiv_func,
                    hir::BinOp::Mod => class_info.rmod_func,
                    hir::BinOp::Pow => class_info.rpow_func,
                    hir::BinOp::BitAnd => class_info.rand_func,
                    hir::BinOp::BitOr => class_info.ror_func,
                    hir::BinOp::BitXor => class_info.rxor_func,
                    hir::BinOp::LShift => class_info.rlshift_func,
                    hir::BinOp::RShift => class_info.rrshift_func,
                    hir::BinOp::MatMul => class_info.rmatmul_func,
                }
            } else {
                None
            };

            if let Some(func_id) = rdunder_func {
                // Reverse dunders: self is the right operand, other is the left
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: func_id,
                    args: vec![right_op, left_op],
                });
                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Check if operands are stored as Union (boxed pointers), even if inference
        // narrowed the type. The storage type determines the runtime representation.
        let left_is_union = left_ty.is_union()
            || matches!(&left_op, mir::Operand::Local(id) if mir_func.locals.get(id).is_some_and(|l| l.ty.is_union()));
        let right_is_union = right_ty.is_union()
            || matches!(&right_op, mir::Operand::Local(id) if mir_func.locals.get(id).is_some_and(|l| l.ty.is_union()));

        // Union arithmetic: operands are already boxed pointers — use runtime dispatch
        if left_is_union || right_is_union {
            let obj_func = match op {
                hir::BinOp::Add => Some(mir::RuntimeFunc::ObjAdd),
                hir::BinOp::Sub => Some(mir::RuntimeFunc::ObjSub),
                hir::BinOp::Mul => Some(mir::RuntimeFunc::ObjMul),
                hir::BinOp::Div => Some(mir::RuntimeFunc::ObjDiv),
                hir::BinOp::FloorDiv => Some(mir::RuntimeFunc::ObjFloorDiv),
                hir::BinOp::Mod => Some(mir::RuntimeFunc::ObjMod),
                hir::BinOp::Pow => Some(mir::RuntimeFunc::ObjPow),
                _ => None, // Bitwise ops not supported on Union (yet)
            };

            if let Some(rt_func) = obj_func {
                let boxed_left = if left_is_union {
                    left_op
                } else {
                    self.box_primitive_if_needed(left_op, &left_ty, mir_func)
                };
                let boxed_right = if right_is_union {
                    right_op
                } else {
                    self.box_primitive_if_needed(right_op, &right_ty, mir_func)
                };
                // Result is Union (boxed pointer)
                let union_result =
                    self.alloc_and_add_local(Type::Union(vec![Type::Int, Type::Float]), mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: union_result,
                    func: rt_func,
                    args: vec![boxed_left, boxed_right],
                });
                return Ok(mir::Operand::Local(union_result));
            }
        }

        // MatMul (@) is only supported via class dunders — no primitive meaning
        if matches!(op, hir::BinOp::MatMul) {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                "unsupported operand type(s) for @: only classes with __matmul__ support this operator".to_string(),
                expr.span,
            ));
        }

        let mir_op = match op {
            hir::BinOp::Add => mir::BinOp::Add,
            hir::BinOp::Sub => mir::BinOp::Sub,
            hir::BinOp::Mul => mir::BinOp::Mul,
            hir::BinOp::Div => mir::BinOp::Div,
            hir::BinOp::FloorDiv => mir::BinOp::FloorDiv,
            hir::BinOp::Mod => mir::BinOp::Mod,
            hir::BinOp::Pow => mir::BinOp::Pow,
            // Bitwise operators
            hir::BinOp::BitAnd => mir::BinOp::BitAnd,
            hir::BinOp::BitOr => mir::BinOp::BitOr,
            hir::BinOp::BitXor => mir::BinOp::BitXor,
            hir::BinOp::LShift => mir::BinOp::LShift,
            hir::BinOp::RShift => mir::BinOp::RShift,
            hir::BinOp::MatMul => unreachable!("MatMul handled above"),
        };

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: result_local,
            op: mir_op,
            left: left_op,
            right: right_op,
        });
        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a comparison expression.
    pub(super) fn lower_compare(
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
        let left_type = self.get_expr_type(left_expr, hir_module);
        let right_type = self.get_expr_type(right_expr, hir_module);

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
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: mir::ComparisonOp::Eq,
                        },
                        args: vec![boxed_left, boxed_right],
                    });
                }
                hir::CmpOp::NotEq => {
                    // Obj Eq + NOT
                    let eq_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: eq_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: mir::ComparisonOp::Eq,
                        },
                        args: vec![boxed_left, boxed_right],
                    });
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(eq_local),
                    });
                }
                hir::CmpOp::Lt => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: mir::ComparisonOp::Lt,
                        },
                        args: vec![boxed_left, boxed_right],
                    });
                }
                hir::CmpOp::LtE => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: mir::ComparisonOp::Lte,
                        },
                        args: vec![boxed_left, boxed_right],
                    });
                }
                hir::CmpOp::Gt => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: mir::ComparisonOp::Gt,
                        },
                        args: vec![boxed_left, boxed_right],
                    });
                }
                hir::CmpOp::GtE => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: mir::ComparisonOp::Gte,
                        },
                        args: vec![boxed_left, boxed_right],
                    });
                }
                hir::CmpOp::In => {
                    // Use runtime dispatch for containment check on Union containers
                    // Note: boxed_right is the container, boxed_left is the element
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::ObjContains,
                        args: vec![boxed_right, boxed_left], // (container, element)
                    });
                }
                hir::CmpOp::NotIn => {
                    // ObjContains + NOT
                    let contains_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: contains_local,
                        func: mir::RuntimeFunc::ObjContains,
                        args: vec![boxed_right, boxed_left], // (container, element)
                    });
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
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Str,
                            op: mir::ComparisonOp::Eq,
                        },
                        args: vec![left_op, right_op],
                    });
                }
                hir::CmpOp::NotEq => {
                    // For !=, compute == and then negate
                    let eq_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: eq_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Str,
                            op: mir::ComparisonOp::Eq,
                        },
                        args: vec![left_op, right_op],
                    });
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
                        func: mir::RuntimeFunc::StrContains,
                        args: vec![left_op, right_op],
                    });
                }
                hir::CmpOp::NotIn => {
                    // String substring check negated: left not in right
                    let contains_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: contains_local,
                        func: mir::RuntimeFunc::StrContains,
                        args: vec![left_op, right_op],
                    });
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
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: cmp_op,
                        },
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
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Bytes,
                            op: mir::ComparisonOp::Eq,
                        },
                        args: vec![left_op, right_op],
                    });
                }
                hir::CmpOp::NotEq => {
                    // For !=, compute == and then negate
                    let eq_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: eq_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Bytes,
                            op: mir::ComparisonOp::Eq,
                        },
                        args: vec![left_op, right_op],
                    });
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
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Obj,
                            op: cmp_op,
                        },
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
                        func: mir::RuntimeFunc::DictContains,
                        args: vec![right_op, boxed_key], // (dict, key)
                    });
                }
                Type::Set(_) => {
                    // elem in set - use rt_set_contains
                    // Box element if needed (int/bool elements need boxing)
                    let boxed_elem = self.box_primitive_if_needed(left_op, &left_type, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::SetContains,
                        args: vec![right_op, boxed_elem], // (set, elem)
                    });
                }
                Type::List(_) => {
                    // elem in list - use rt_list_index and check if >= 0
                    let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: idx_local,
                        func: mir::RuntimeFunc::ListIndex,
                        args: vec![right_op, left_op], // (list, value)
                    });
                    // result = idx >= 0
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir::BinOp::GtE,
                        left: mir::Operand::Local(idx_local),
                        right: mir::Operand::Constant(mir::Constant::Int(0)),
                    });
                }
                Type::Tuple(_) => {
                    // elem in tuple - use rt_obj_contains (needs boxed element)
                    let boxed_elem = self.box_primitive_if_needed(left_op, &left_type, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::ObjContains,
                        args: vec![right_op, boxed_elem], // (tuple, elem)
                    });
                }
                Type::Class { class_id, .. } => {
                    // Class with __contains__ dunder
                    let contains_func = self
                        .get_class_info(&class_id)
                        .and_then(|info| info.contains_func);

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
        } else if let Type::List(left_elem_type) = &left_type {
            // List comparison - use runtime function based on element type
            if matches!(op, hir::CmpOp::Eq | hir::CmpOp::NotEq)
                && matches!(right_type, Type::List(_))
            {
                let is_not_eq = matches!(op, hir::CmpOp::NotEq);

                // Get element type from right side if left is Any
                let right_elem_type = if let Type::List(rt) = &right_type {
                    rt.as_ref()
                } else {
                    &Type::Any
                };

                // Choose runtime function based on element type (prefer non-Any type)
                let elem_type = if matches!(left_elem_type.as_ref(), Type::Any) {
                    right_elem_type
                } else {
                    left_elem_type.as_ref()
                };

                let compare_kind = match elem_type {
                    Type::Float => mir::CompareKind::ListFloat,
                    Type::Str => mir::CompareKind::ListStr,
                    _ => mir::CompareKind::ListInt, // Default to int comparison
                };

                let eq_func = mir::RuntimeFunc::Compare {
                    kind: compare_kind,
                    op: mir::ComparisonOp::Eq,
                };

                if is_not_eq {
                    // NotEq: compute eq and negate
                    let eq_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: eq_local,
                        func: eq_func,
                        args: vec![left_op, right_op],
                    });
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
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Compare {
                        kind: mir::CompareKind::List,
                        op: compare_op,
                    },
                    args: vec![left_op, right_op],
                });
            }
        } else if let Type::Tuple(_) = &left_type {
            // Tuple comparison - use runtime function for element-wise comparison
            if matches!(op, hir::CmpOp::Eq | hir::CmpOp::NotEq)
                && matches!(right_type, Type::Tuple(_))
            {
                let is_not_eq = matches!(op, hir::CmpOp::NotEq);

                if is_not_eq {
                    // NotEq: compute eq and negate
                    let eq_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: eq_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Tuple,
                            op: mir::ComparisonOp::Eq,
                        },
                        args: vec![left_op, right_op],
                    });
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(eq_local),
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Compare {
                            kind: mir::CompareKind::Tuple,
                            op: mir::ComparisonOp::Eq,
                        },
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

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Compare {
                        kind: mir::CompareKind::Tuple,
                        op: compare_op,
                    },
                    args: vec![left_op, right_op],
                });
            }
        } else if let Type::Class { class_id, .. } = &left_type {
            // Class type: dispatch to dunder methods if available
            let dunder_func = if let Some(class_info) = self.get_class_info(class_id) {
                match op {
                    hir::CmpOp::Eq => class_info.eq_func,
                    hir::CmpOp::NotEq => class_info.ne_func.or(class_info.eq_func),
                    hir::CmpOp::Lt => class_info.lt_func,
                    hir::CmpOp::LtE => class_info.le_func,
                    hir::CmpOp::Gt => class_info.gt_func,
                    hir::CmpOp::GtE => class_info.ge_func,
                    _ => None,
                }
            } else {
                None
            };

            if let Some(func_id) = dunder_func {
                // For NotEq: prefer __ne__ if available, otherwise use __eq__ + negate
                let use_eq_negated = matches!(op, hir::CmpOp::NotEq)
                    && self
                        .get_class_info(class_id)
                        .and_then(|ci| ci.ne_func)
                        .is_none();

                if use_eq_negated {
                    // No __ne__, use __eq__ + NOT
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: func_id,
                        args: vec![left_op, right_op],
                    });
                    let negated = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: negated,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(result_local),
                    });
                    return Ok(mir::Operand::Local(negated));
                }

                // Direct dunder call
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: func_id,
                    args: vec![left_op, right_op],
                });
                return Ok(mir::Operand::Local(result_local));
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
                let eq_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: eq_local,
                    func: mir::RuntimeFunc::Compare {
                        kind: mir::CompareKind::Obj,
                        op: mir::ComparisonOp::Eq,
                    },
                    args: vec![boxed_left, boxed_right],
                });
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
                    func: mir::RuntimeFunc::Compare {
                        kind: mir::CompareKind::Obj,
                        op: compare_op,
                    },
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

    /// Lower a unary operation expression.
    pub(super) fn lower_unop(
        &mut self,
        op: hir::UnOp,
        operand: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let operand_expr = &hir_module.exprs[operand];
        let operand_op = self.lower_expr(operand_expr, hir_module, mir_func)?;

        // Determine result type based on operation and operand type
        let operand_ty = self.get_expr_type(operand_expr, hir_module);
        let result_type = match op {
            hir::UnOp::Not => Type::Bool,         // not always returns bool
            hir::UnOp::Neg => operand_ty.clone(), // neg preserves operand type
            hir::UnOp::Invert => Type::Int,       // bitwise NOT always returns Int
            hir::UnOp::Pos => operand_ty.clone(), // unary plus preserves type
        };

        let result_local = self.alloc_and_add_local(result_type.clone(), mir_func);

        // Check for class type with unary dunders
        if let Type::Class { class_id, .. } = &operand_ty {
            let dunder_func = if let Some(class_info) = self.get_class_info(class_id) {
                match op {
                    hir::UnOp::Neg => class_info.neg_func,
                    hir::UnOp::Not => class_info.bool_func,
                    hir::UnOp::Pos => class_info.pos_func,
                    hir::UnOp::Invert => class_info.invert_func,
                }
            } else {
                None
            };

            if let Some(func_id) = dunder_func {
                if matches!(op, hir::UnOp::Not) {
                    // __bool__ returns bool, then negate
                    let bool_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: bool_local,
                        func: func_id,
                        args: vec![operand_op],
                    });
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(bool_local),
                    });
                } else {
                    // __neg__ returns same type
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: func_id,
                        args: vec![operand_op],
                    });
                }
                return Ok(mir::Operand::Local(result_local));
            }
        }

        let mir_op = match op {
            hir::UnOp::Neg => mir::UnOp::Neg,
            hir::UnOp::Not => mir::UnOp::Not,
            hir::UnOp::Invert => mir::UnOp::Invert,
            hir::UnOp::Pos => {
                // For primitives, +x is identity (no-op copy)
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: operand_op,
                });
                return Ok(mir::Operand::Local(result_local));
            }
        };

        // For Not operation, we need to convert the operand to bool first
        // if it's not already a boolean (e.g., Union types need rt_is_truthy)
        let final_operand = if matches!(op, hir::UnOp::Not) {
            self.convert_to_bool(operand_op, &operand_ty, mir_func)
        } else {
            operand_op
        };

        self.emit_instruction(mir::InstructionKind::UnOp {
            dest: result_local,
            op: mir_op,
            operand: final_operand,
        });
        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a logical operation (and/or) with short-circuit evaluation.
    ///
    /// Python semantics:
    /// - `a and b` returns `b` if `a` is truthy, else returns `a`
    /// - `a or b` returns `a` if `a` is truthy, else returns `b`
    pub(super) fn lower_logical_op(
        &mut self,
        op: hir::LogicalOp,
        left: hir::ExprId,
        right: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Infer result type from operand types
        let left_expr = &hir_module.exprs[left];
        let right_expr = &hir_module.exprs[right];
        let left_type = self.get_expr_type(left_expr, hir_module);
        let right_type = self.get_expr_type(right_expr, hir_module);

        // Determine result type based on operator
        let result_type = match op {
            hir::LogicalOp::And => {
                // `and` returns right operand if left is truthy, else left
                // So type is union of both types (simplified to right if same)
                if left_type == right_type {
                    right_type.clone()
                } else {
                    Type::Union(vec![left_type.clone(), right_type.clone()])
                }
            }
            hir::LogicalOp::Or => {
                // `or` returns left operand if truthy, else right
                // So type is union of both types (simplified to left if same)
                if left_type == right_type {
                    left_type.clone()
                } else {
                    Type::Union(vec![left_type.clone(), right_type.clone()])
                }
            }
        };

        let result_local = self.alloc_and_add_local(result_type, mir_func);

        match op {
            hir::LogicalOp::And => {
                // Evaluate left operand
                let left_op = self.lower_expr(left_expr, hir_module, mir_func)?;

                // Convert left operand to bool for branching
                let left_bool = self.convert_to_bool(left_op.clone(), &left_type, mir_func);

                let then_bb = self.new_block();
                let else_bb = self.new_block();
                let merge_bb = self.new_block();

                let then_id = then_bb.id;
                let else_id = else_bb.id;
                let merge_id = merge_bb.id;

                // Branch on left operand's truthiness
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: left_bool,
                    then_block: then_id,
                    else_block: else_id,
                };

                // Then block: left is truthy, evaluate and return right
                self.push_block(then_bb);
                let right_op = self.lower_expr(right_expr, hir_module, mir_func)?;
                // Box primitive if result is Union (mismatched types)
                let right_val = if left_type != right_type {
                    self.box_primitive_if_needed(right_op, &right_type, mir_func)
                } else {
                    right_op
                };
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: right_val,
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

                // Else block: left is falsy, return left value
                self.push_block(else_bb);
                // Box primitive if result is Union (mismatched types)
                let left_val = if left_type != right_type {
                    self.box_primitive_if_needed(left_op, &left_type, mir_func)
                } else {
                    left_op
                };
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: left_val,
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

                // Merge block
                self.push_block(merge_bb);
            }
            hir::LogicalOp::Or => {
                // Evaluate left operand
                let left_op = self.lower_expr(left_expr, hir_module, mir_func)?;

                // Convert left operand to bool for branching
                let left_bool = self.convert_to_bool(left_op.clone(), &left_type, mir_func);

                let then_bb = self.new_block();
                let else_bb = self.new_block();
                let merge_bb = self.new_block();

                let then_id = then_bb.id;
                let else_id = else_bb.id;
                let merge_id = merge_bb.id;

                // Branch on left operand's truthiness
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: left_bool,
                    then_block: then_id,
                    else_block: else_id,
                };

                // Then block: left is truthy, return left value
                self.push_block(then_bb);
                // Box primitive if result is Union (mismatched types)
                let left_val = if left_type != right_type {
                    self.box_primitive_if_needed(left_op, &left_type, mir_func)
                } else {
                    left_op
                };
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: left_val,
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

                // Else block: left is falsy, evaluate and return right
                self.push_block(else_bb);
                let right_op = self.lower_expr(right_expr, hir_module, mir_func)?;
                // Box primitive if result is Union (mismatched types)
                let right_val = if left_type != right_type {
                    self.box_primitive_if_needed(right_op, &right_type, mir_func)
                } else {
                    right_op
                };
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: right_val,
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

                // Merge block
                self.push_block(merge_bb);
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower ternary expression: `value_if_true if condition else value_if_false`
    pub(super) fn lower_if_expr(
        &mut self,
        cond: hir::ExprId,
        then_val: hir::ExprId,
        else_val: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Get the result type from branch types
        let then_expr = &hir_module.exprs[then_val];
        let else_expr = &hir_module.exprs[else_val];
        let then_ty = self.get_expr_type(then_expr, hir_module);
        let else_ty = self.get_expr_type(else_expr, hir_module);

        let types_differ = then_ty != else_ty;
        let result_ty = if types_differ {
            Type::Union(vec![then_ty.clone(), else_ty.clone()])
        } else {
            then_ty.clone()
        };

        // Allocate result local
        let result_local = self.alloc_and_add_local(result_ty, mir_func);

        // Evaluate condition and convert to bool if needed
        let cond_expr = &hir_module.exprs[cond];
        let cond_type = self.get_expr_type(cond_expr, hir_module);
        let cond_op = self.lower_expr(cond_expr, hir_module, mir_func)?;
        let final_cond_op = if matches!(cond_type, Type::Bool) {
            cond_op
        } else {
            self.convert_to_bool(cond_op, &cond_type, mir_func)
        };

        // Create blocks for branches
        let then_bb = self.new_block();
        let else_bb = self.new_block();
        let merge_bb = self.new_block();

        let then_id = then_bb.id;
        let else_id = else_bb.id;
        let merge_id = merge_bb.id;

        // Branch based on condition
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: final_cond_op,
            then_block: then_id,
            else_block: else_id,
        };

        // Then block: evaluate then_val and store in result
        self.push_block(then_bb);
        let then_op = self.lower_expr(then_expr, hir_module, mir_func)?;
        // Box primitive if result is Union (mismatched types)
        let then_val = if types_differ {
            self.box_primitive_if_needed(then_op, &then_ty, mir_func)
        } else {
            then_op
        };
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: then_val,
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // Else block: evaluate else_val and store in result
        self.push_block(else_bb);
        let else_op = self.lower_expr(else_expr, hir_module, mir_func)?;
        // Box primitive if result is Union (mismatched types)
        let else_val = if types_differ {
            self.box_primitive_if_needed(else_op, &else_ty, mir_func)
        } else {
            else_op
        };
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: else_val,
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(merge_id);

        // Merge block: continue execution
        self.push_block(merge_bb);

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a string concatenation chain using StringBuilder for O(n) complexity.
    ///
    /// For `a + b + c + d`:
    /// 1. Create StringBuilder with estimated capacity
    /// 2. Append each string operand in evaluation order
    /// 3. Finalize to produce the result string
    fn lower_str_concat_with_builder(
        &mut self,
        chain: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Estimate capacity: sum of string literal lengths + heuristic for non-literals
        let mut estimated_capacity: i64 = 0;
        for &expr_id in chain {
            let expr = &hir_module.exprs[expr_id];
            if let hir::ExprKind::Str(s) = &expr.kind {
                let str_content = self.interner.resolve(*s);
                estimated_capacity += str_content.len() as i64;
            } else {
                // Heuristic: assume 20 bytes for non-literal strings
                estimated_capacity += 20;
            }
        }

        // Create StringBuilder with estimated capacity
        let builder_local = self.alloc_and_add_local(Type::Str, mir_func); // Using Str type for GC root tracking
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: builder_local,
            func: mir::RuntimeFunc::MakeStringBuilder,
            args: vec![mir::Operand::Constant(mir::Constant::Int(
                estimated_capacity,
            ))],
        });

        // Append each string to the builder
        let dummy_local = self.alloc_and_add_local(Type::Int, mir_func); // For void call result
        for &expr_id in chain {
            let expr = &hir_module.exprs[expr_id];
            let str_op = self.lower_expr(expr, hir_module, mir_func)?;

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::StringBuilderAppend,
                args: vec![mir::Operand::Local(builder_local), str_op],
            });
        }

        // Finalize and return the result string
        let result_local = self.alloc_and_add_local(Type::Str, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::StringBuilderToStr,
            args: vec![mir::Operand::Local(builder_local)],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
