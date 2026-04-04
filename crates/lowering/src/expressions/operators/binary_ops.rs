//! Binary operation lowering: arithmetic, bitwise, string concat, collection ops

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
            let left_type = self.get_type_of_expr_id(*left, hir_module);
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
    pub(in crate::expressions) fn lower_binop(
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
        let left_ty = self.get_type_of_expr_id(left, hir_module);

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

        let right_ty = self.get_type_of_expr_id(right, hir_module);

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
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_STR_CONCAT,
                        ),
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                hir::BinOp::Mul => {
                    // String multiplication: "abc" * 3
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_STR_MUL,
                        ),
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
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BYTES_CONCAT,
                        ),
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                hir::BinOp::Mul => {
                    // Bytes repetition: b"abc" * 3
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BYTES_REPEAT,
                        ),
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
                let list_result = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_CONCAT),
                    vec![left_op, right_op],
                    Type::List(elem_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(list_result));
            }
        }

        // Check for dict merge operation (|)
        if let Type::Dict(key_ty, value_ty) = &left_ty {
            if matches!(op, hir::BinOp::BitOr) {
                let dict_result = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_MERGE),
                    vec![left_op, right_op],
                    Type::Dict(key_ty.clone(), value_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(dict_result));
            }
        }

        // Check for set operations (|, &, -, ^)
        if let Type::Set(elem_ty) = &left_ty {
            let set_func = match op {
                hir::BinOp::BitOr => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_UNION,
                )),
                hir::BinOp::BitAnd => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_INTERSECTION,
                )),
                hir::BinOp::Sub => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_DIFFERENCE,
                )),
                hir::BinOp::BitXor => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_SYMMETRIC_DIFFERENCE,
                )),
                _ => None,
            };
            if let Some(runtime_func) = set_func {
                let set_result = self.emit_runtime_call(
                    runtime_func,
                    vec![left_op, right_op],
                    Type::Set(elem_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(set_result));
            }
        }

        // Check for class type with arithmetic dunders
        if let Type::Class { class_id, .. } = &left_ty {
            let dunder_name = match op {
                hir::BinOp::Add => "__add__",
                hir::BinOp::Sub => "__sub__",
                hir::BinOp::Mul => "__mul__",
                hir::BinOp::Div => "__truediv__",
                hir::BinOp::FloorDiv => "__floordiv__",
                hir::BinOp::Mod => "__mod__",
                hir::BinOp::Pow => "__pow__",
                hir::BinOp::BitAnd => "__and__",
                hir::BinOp::BitOr => "__or__",
                hir::BinOp::BitXor => "__xor__",
                hir::BinOp::LShift => "__lshift__",
                hir::BinOp::RShift => "__rshift__",
                hir::BinOp::MatMul => "__matmul__",
            };
            let dunder_func = self
                .get_class_info(class_id)
                .and_then(|ci| ci.get_dunder_func(dunder_name));

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
            let rdunder_name = match op {
                hir::BinOp::Add => "__radd__",
                hir::BinOp::Sub => "__rsub__",
                hir::BinOp::Mul => "__rmul__",
                hir::BinOp::Div => "__rtruediv__",
                hir::BinOp::FloorDiv => "__rfloordiv__",
                hir::BinOp::Mod => "__rmod__",
                hir::BinOp::Pow => "__rpow__",
                hir::BinOp::BitAnd => "__rand__",
                hir::BinOp::BitOr => "__ror__",
                hir::BinOp::BitXor => "__rxor__",
                hir::BinOp::LShift => "__rlshift__",
                hir::BinOp::RShift => "__rrshift__",
                hir::BinOp::MatMul => "__rmatmul__",
            };
            let rdunder_func = self
                .get_class_info(class_id)
                .and_then(|ci| ci.get_dunder_func(rdunder_name));

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
                hir::BinOp::Add => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_ADD,
                )),
                hir::BinOp::Sub => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_SUB,
                )),
                hir::BinOp::Mul => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_MUL,
                )),
                hir::BinOp::Div => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_DIV,
                )),
                hir::BinOp::FloorDiv => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_FLOORDIV,
                )),
                hir::BinOp::Mod => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_MOD,
                )),
                hir::BinOp::Pow => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_POW,
                )),
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
                let union_result = self.emit_runtime_call(
                    rt_func,
                    vec![boxed_left, boxed_right],
                    Type::Union(vec![Type::Int, Type::Float]),
                    mir_func,
                );
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

        // Create StringBuilder with estimated capacity (Using Str type for GC root tracking)
        let builder_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_STRING_BUILDER),
            vec![mir::Operand::Constant(mir::Constant::Int(
                estimated_capacity,
            ))],
            Type::Str,
            mir_func,
        );

        // Append each string to the builder
        let dummy_local = self.alloc_and_add_local(Type::Int, mir_func); // For void call result
        for &expr_id in chain {
            let expr = &hir_module.exprs[expr_id];
            let str_op = self.lower_expr(expr, hir_module, mir_func)?;

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_STRING_BUILDER_APPEND,
                ),
                args: vec![mir::Operand::Local(builder_local), str_op],
            });
        }

        // Finalize and return the result string
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STRING_BUILDER_TO_STR),
            vec![mir::Operand::Local(builder_local)],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }
}
