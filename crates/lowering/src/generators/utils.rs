//! Helper functions for generator expression lowering
//!
//! This module provides utility functions for lowering expressions within
//! generators, particularly for yield values and filter conditions.

use std::collections::HashMap;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{LocalId, Span, VarId};

use super::YieldInfo;
use crate::context::Lowering;

/// Convert a HIR binary operator to a MIR binary operator.
/// Returns an error for unsupported operators (e.g., MatMul outside class context).
pub(super) fn hir_binop_to_mir(op: &hir::BinOp, span: Span) -> Result<mir::BinOp> {
    match op {
        hir::BinOp::Add => Ok(mir::BinOp::Add),
        hir::BinOp::Sub => Ok(mir::BinOp::Sub),
        hir::BinOp::Mul => Ok(mir::BinOp::Mul),
        hir::BinOp::Div => Ok(mir::BinOp::Div),
        hir::BinOp::Mod => Ok(mir::BinOp::Mod),
        hir::BinOp::FloorDiv => Ok(mir::BinOp::FloorDiv),
        hir::BinOp::Pow => Ok(mir::BinOp::Pow),
        hir::BinOp::BitAnd => Ok(mir::BinOp::BitAnd),
        hir::BinOp::BitOr => Ok(mir::BinOp::BitOr),
        hir::BinOp::BitXor => Ok(mir::BinOp::BitXor),
        hir::BinOp::LShift => Ok(mir::BinOp::LShift),
        hir::BinOp::RShift => Ok(mir::BinOp::RShift),
        hir::BinOp::MatMul => Err(pyaot_diagnostics::CompilerError::type_error(
            "@ operator is only supported on classes with __matmul__".to_string(),
            span,
        )),
    }
}

/// Get an operand for a simple expression (Int literal or Var reference).
/// Pure HIR analysis — no Lowering state needed.
pub(super) fn get_operand_for_expr(
    expr: &hir::Expr,
    var_to_mir_local: &HashMap<VarId, LocalId>,
) -> Result<mir::Operand> {
    match &expr.kind {
        hir::ExprKind::Int(n) => Ok(mir::Operand::Constant(mir::Constant::Int(*n))),
        hir::ExprKind::Var(var_id) => {
            if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                Ok(mir::Operand::Local(mir_local))
            } else {
                Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                    "unresolved variable in generator operand expression",
                    expr.span,
                ))
            }
        }
        other => Err(pyaot_diagnostics::CompilerError::codegen_error_at(
            format!("unsupported expression in generator operand: {:?}", other),
            expr.span,
        )),
    }
}

/// Collect yield information from the function body (in order).
/// Returns YieldInfo for each yield, including assignment targets.
/// Pure HIR analysis — no Lowering state needed.
pub(super) fn collect_yield_info(body: &[hir::StmtId], hir_module: &hir::Module) -> Vec<YieldInfo> {
    let mut yields = Vec::new();
    for stmt_id in body {
        collect_yields_from_stmt_with_target(*stmt_id, hir_module, &mut yields);
    }
    yields
}

fn collect_yields_from_stmt_with_target(
    stmt_id: hir::StmtId,
    hir_module: &hir::Module,
    yields: &mut Vec<YieldInfo>,
) {
    let stmt = &hir_module.stmts[stmt_id];
    match &stmt.kind {
        hir::StmtKind::Expr(expr_id) => {
            collect_yields_from_expr_with_target(*expr_id, None, hir_module, yields);
        }
        hir::StmtKind::Assign { target, value, .. } => {
            // Check if the value is a yield expression
            let value_expr = &hir_module.exprs[*value];
            if matches!(value_expr.kind, hir::ExprKind::Yield(_)) {
                // This is `target = yield value` - record the assignment target
                collect_yields_from_expr_with_target(*value, Some(*target), hir_module, yields);
            } else {
                collect_yields_from_expr_with_target(*value, None, hir_module, yields);
            }
        }
        hir::StmtKind::Return(Some(expr_id)) => {
            collect_yields_from_expr_with_target(*expr_id, None, hir_module, yields);
        }
        hir::StmtKind::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
            for s in then_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
            for s in else_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
        }
        hir::StmtKind::While {
            cond,
            body,
            else_block,
        } => {
            collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
            for s in body {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
            for s in else_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
        }
        hir::StmtKind::For {
            iter,
            body,
            else_block,
            ..
        }
        | hir::StmtKind::ForUnpack {
            iter,
            body,
            else_block,
            ..
        } => {
            collect_yields_from_expr_with_target(*iter, None, hir_module, yields);
            for s in body {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
            for s in else_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
        }
        _ => {}
    }
}

fn collect_yields_from_expr_with_target(
    expr_id: hir::ExprId,
    assignment_target: Option<VarId>,
    hir_module: &hir::Module,
    yields: &mut Vec<YieldInfo>,
) {
    let expr = &hir_module.exprs[expr_id];
    match &expr.kind {
        hir::ExprKind::Yield(value) => {
            yields.push(YieldInfo {
                yield_value: *value,
                assignment_target,
            });
        }
        hir::ExprKind::BinOp { left, right, .. } => {
            collect_yields_from_expr_with_target(*left, None, hir_module, yields);
            collect_yields_from_expr_with_target(*right, None, hir_module, yields);
        }
        hir::ExprKind::UnOp { operand, .. } => {
            collect_yields_from_expr_with_target(*operand, None, hir_module, yields);
        }
        hir::ExprKind::Call { func, args, .. } => {
            collect_yields_from_expr_with_target(*func, None, hir_module, yields);
            for a in args {
                let arg_id = match a {
                    hir::CallArg::Regular(id) => id,
                    hir::CallArg::Starred(id) => id,
                };
                collect_yields_from_expr_with_target(*arg_id, None, hir_module, yields);
            }
        }
        hir::ExprKind::IfExpr {
            cond,
            then_val,
            else_val,
        } => {
            collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
            collect_yields_from_expr_with_target(*then_val, None, hir_module, yields);
            collect_yields_from_expr_with_target(*else_val, None, hir_module, yields);
        }
        _ => {}
    }
}

/// Lower a simple statement in generator context.
/// Handles: assignments with simple expressions, expression statements.
/// Pure HIR/MIR construction — no Lowering state needed.
pub(super) fn lower_simple_stmt_for_generator(
    stmt_id: hir::StmtId,
    hir_module: &hir::Module,
    block: &mut mir::BasicBlock,
    var_to_mir_local: &HashMap<VarId, LocalId>,
) -> Result<()> {
    let stmt = &hir_module.stmts[stmt_id];

    match &stmt.kind {
        hir::StmtKind::Assign { target, value, .. } => {
            if let Some(&dest_mir_local) = var_to_mir_local.get(target) {
                let value_expr = &hir_module.exprs[*value];
                // Handle simple cases: int literal, variable reference, binary operations
                match &value_expr.kind {
                    hir::ExprKind::Int(n) => {
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::Copy {
                                dest: dest_mir_local,
                                src: mir::Operand::Constant(mir::Constant::Int(*n)),
                            },
                            span: None,
                        });
                    }
                    hir::ExprKind::Var(src_var) => {
                        if let Some(&src_mir_local) = var_to_mir_local.get(src_var) {
                            block.instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::Copy {
                                    dest: dest_mir_local,
                                    src: mir::Operand::Local(src_mir_local),
                                },
                                span: None,
                            });
                        }
                    }
                    hir::ExprKind::BinOp { left, op, right } => {
                        let left_expr = &hir_module.exprs[*left];
                        let right_expr = &hir_module.exprs[*right];
                        let left_op = get_operand_for_expr(left_expr, var_to_mir_local)?;
                        let right_op = get_operand_for_expr(right_expr, var_to_mir_local)?;
                        let mir_op = hir_binop_to_mir(op, value_expr.span)?;
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::BinOp {
                                dest: dest_mir_local,
                                op: mir_op,
                                left: left_op,
                                right: right_op,
                            },
                            span: None,
                        });
                    }
                    _ => {
                        // Unsupported expression type
                    }
                }
            }
        }
        hir::StmtKind::Expr(_) => {
            // Expression statements (non-yield) - currently no-op
        }
        _ => {
            // Other statement types not supported in generator yield sections
        }
    }

    Ok(())
}

impl<'a> Lowering<'a> {
    /// Compute a yield expression value for a generator.
    ///
    /// Handles simple vars, constants, unary/binary operations, and attribute
    /// access on the loop variable (e.g. `v.field` where `v` is `loop_var_id`).
    ///
    /// `loop_var_ty` is the type of the loop variable and is used to resolve
    /// attribute accesses against the class's field table.  Pass `None` when the
    /// type is not known or not needed (recursive calls).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn compute_yield_expr_for_generator(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        block: &mut mir::BasicBlock,
        mir_func: &mut mir::Function,
        var_to_mir_local: &HashMap<VarId, LocalId>,
        loop_var_local: LocalId,
        loop_var_id: VarId,
        loop_var_ty: Option<&Type>,
    ) -> Result<mir::Operand> {
        match &expr.kind {
            hir::ExprKind::Int(n) => Ok(mir::Operand::Constant(mir::Constant::Int(*n))),
            hir::ExprKind::Var(var_id) => {
                // If it's the loop variable, use the loop_var_local
                if *var_id == loop_var_id {
                    Ok(mir::Operand::Local(loop_var_local))
                } else if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                    Ok(mir::Operand::Local(mir_local))
                } else {
                    Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                        "unresolved variable in generator yield expression",
                        expr.span,
                    ))
                }
            }
            hir::ExprKind::Attribute { obj, attr } => {
                // Handle `loop_var.field` by emitting InstanceGetField.
                //
                // The generator resume protocol requires that yielded values are
                // raw i64 values that the caller can interpret as `*mut Obj` when
                // the element type is a heap-boxed type, or as raw primitive bits
                // for types that bypass boxing (Int, Bool in compact list storage).
                //
                // For float fields we must box the raw f64 bits into a FloatObj so
                // that `rt_unbox_float` can properly dereference it.  For other
                // primitive fields (Int, Bool) we return the raw i64 value directly
                // since they are stored as compact integers in the instance.
                // Class instance fields are already pointers and need no boxing.
                //
                // We only support the case where the object is the loop variable and
                // the loop variable type is a class with a known field offset.
                let obj_expr = &hir_module.exprs[*obj];
                if let hir::ExprKind::Var(var_id) = &obj_expr.kind {
                    if *var_id == loop_var_id {
                        if let Some(Type::Class { class_id, .. }) = loop_var_ty {
                            // Clone to drop the borrow on self so we can call &mut self methods.
                            let class_id = *class_id;
                            if let Some((offset, field_ty)) =
                                self.get_class_info(&class_id).and_then(|ci| {
                                    ci.field_offsets.get(attr).copied().map(|off| {
                                        let ty =
                                            ci.field_types.get(attr).cloned().unwrap_or(Type::Any);
                                        (off, ty)
                                    })
                                })
                            {
                                // Read the raw field value (i64 bits, regardless of type).
                                // InstanceGetField always returns i64; the codegen bitcasts to
                                // f64 when the destination local type is Float.
                                let raw_local =
                                    self.alloc_and_add_local(field_ty.clone(), mir_func);
                                block.instructions.push(mir::Instruction {
                                    kind: mir::InstructionKind::RuntimeCall {
                                        dest: raw_local,
                                        func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD),
                                        args: vec![
                                            mir::Operand::Local(loop_var_local),
                                            mir::Operand::Constant(mir::Constant::Int(
                                                offset as i64,
                                            )),
                                        ],
                                    },
                                    span: None,
                                });

                                // For float fields: box the f64 into a heap FloatObj so the
                                // caller's UnboxFloat call can properly dereference it.
                                // The resume protocol always passes boxed values through the
                                // iterator's *mut Obj return slot.
                                if matches!(field_ty, Type::Float) {
                                    let boxed_local =
                                        self.alloc_and_add_local(Type::HeapAny, mir_func); // boxed heap pointer
                                    block.instructions.push(mir::Instruction {
                                        kind: mir::InstructionKind::RuntimeCall {
                                            dest: boxed_local,
                                            func: mir::RuntimeFunc::Call(
                                                &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT,
                                            ),
                                            args: vec![mir::Operand::Local(raw_local)],
                                        },
                                        span: None,
                                    });
                                    return Ok(mir::Operand::Local(boxed_local));
                                }

                                return Ok(mir::Operand::Local(raw_local));
                            }
                        }
                    }
                }
                Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                    "unsupported attribute access pattern in generator yield expression",
                    expr.span,
                ))
            }
            hir::ExprKind::UnOp { op, operand } => {
                let operand_expr = &hir_module.exprs[*operand];
                let operand_val = self.compute_yield_expr_for_generator(
                    operand_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                    loop_var_ty,
                )?;

                match op {
                    hir::UnOp::Neg => {
                        // -x => 0 - x
                        let result_local = self.alloc_and_add_local(Type::Int, mir_func);
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::BinOp {
                                dest: result_local,
                                op: mir::BinOp::Sub,
                                left: mir::Operand::Constant(mir::Constant::Int(0)),
                                right: operand_val,
                            },
                            span: None,
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    hir::UnOp::Not => {
                        // Convert operand to bool via proper truthiness check,
                        // then negate. This handles heap types correctly: empty
                        // containers are falsy, non-empty containers are truthy.
                        //
                        // Use loop_var_ty when the operand is the loop variable,
                        // because get_expr_type returns Any for it (the variable
                        // hasn't been registered in var_types at inference time).
                        let operand_type = {
                            let ty = if let hir::ExprKind::Var(vid) = &operand_expr.kind {
                                if *vid == loop_var_id {
                                    loop_var_ty.cloned().unwrap_or(Type::Any)
                                } else {
                                    self.get_expr_type(operand_expr, hir_module)
                                }
                            } else {
                                self.get_expr_type(operand_expr, hir_module)
                            };
                            // Generator values are always raw i64; Any-typed
                            // operands must use Int truthiness (not IsTruthy
                            // which expects *mut Obj pointers).
                            if matches!(ty, Type::Any) {
                                Type::Int
                            } else {
                                ty
                            }
                        };
                        let bool_operand = self.convert_to_bool_in_block(
                            operand_val,
                            &operand_type,
                            mir_func,
                            block,
                        );
                        let not_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::UnOp {
                                dest: not_local,
                                op: mir::UnOp::Not,
                                operand: bool_operand,
                            },
                            span: None,
                        });
                        // Generator resume functions return i64, so widen the
                        // bool (i8) to int (i64) to avoid a Cranelift type mismatch.
                        let int_local = self.alloc_and_add_local(Type::Int, mir_func);
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::BoolToInt {
                                dest: int_local,
                                src: mir::Operand::Local(not_local),
                            },
                            span: None,
                        });
                        Ok(mir::Operand::Local(int_local))
                    }
                    _ => Ok(operand_val),
                }
            }
            hir::ExprKind::BinOp { op, left, right } => {
                // Recursively compute left and right operands
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                let left_op = self.compute_yield_expr_for_generator(
                    left_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                    loop_var_ty,
                )?;

                let right_op = self.compute_yield_expr_for_generator(
                    right_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                    loop_var_ty,
                )?;

                // Allocate result local
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                // Convert HIR BinOp to MIR BinOp
                let mir_op = hir_binop_to_mir(op, expr.span)?;

                // Emit the binary operation
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir_op,
                        left: left_op,
                        right: right_op,
                    },
                    span: None,
                });

                Ok(mir::Operand::Local(result_local))
            }
            other => Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                format!("unsupported expression in generator yield: {:?}", other),
                expr.span,
            )),
        }
    }

    /// Lower a simple expression for a generator filter condition
    /// Handles constants, vars, BinOp (arithmetic), and Compare (returns Bool)
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_simple_expr_for_generator(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        block: &mut mir::BasicBlock,
        mir_func: &mut mir::Function,
        var_to_mir_local: &HashMap<VarId, LocalId>,
        loop_var_local: LocalId,
        loop_var_id: VarId,
    ) -> Result<mir::Operand> {
        match &expr.kind {
            hir::ExprKind::Int(n) => Ok(mir::Operand::Constant(mir::Constant::Int(*n))),
            hir::ExprKind::Bool(b) => Ok(mir::Operand::Constant(mir::Constant::Bool(*b))),
            hir::ExprKind::Var(var_id) => {
                if *var_id == loop_var_id {
                    Ok(mir::Operand::Local(loop_var_local))
                } else if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                    Ok(mir::Operand::Local(mir_local))
                } else {
                    Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                        "unresolved variable in generator filter expression",
                        expr.span,
                    ))
                }
            }
            hir::ExprKind::BinOp { op, left, right } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                let left_op = self.lower_simple_expr_for_generator(
                    left_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                )?;

                let right_op = self.lower_simple_expr_for_generator(
                    right_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                )?;

                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                let mir_op = hir_binop_to_mir(op, expr.span)?;

                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir_op,
                        left: left_op,
                        right: right_op,
                    },
                    span: None,
                });

                Ok(mir::Operand::Local(result_local))
            }
            hir::ExprKind::Compare { left, op, right } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];

                let left_op = self.lower_simple_expr_for_generator(
                    left_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                )?;

                let right_op = self.lower_simple_expr_for_generator(
                    right_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                )?;

                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

                // Convert HIR CmpOp to MIR BinOp for comparison
                let mir_op = match op {
                    hir::CmpOp::Eq => mir::BinOp::Eq,
                    hir::CmpOp::NotEq => mir::BinOp::NotEq,
                    hir::CmpOp::Lt => mir::BinOp::Lt,
                    hir::CmpOp::LtE => mir::BinOp::LtE,
                    hir::CmpOp::Gt => mir::BinOp::Gt,
                    hir::CmpOp::GtE => mir::BinOp::GtE,
                    _ => mir::BinOp::Eq, // Default for In/NotIn
                };

                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir_op,
                        left: left_op,
                        right: right_op,
                    },
                    span: None,
                });

                Ok(mir::Operand::Local(result_local))
            }
            hir::ExprKind::UnOp { op, operand } => {
                let operand_expr = &hir_module.exprs[*operand];
                let operand_val = self.lower_simple_expr_for_generator(
                    operand_expr,
                    hir_module,
                    block,
                    mir_func,
                    var_to_mir_local,
                    loop_var_local,
                    loop_var_id,
                )?;

                match op {
                    hir::UnOp::Neg => {
                        let result_local = self.alloc_and_add_local(Type::Int, mir_func);
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::BinOp {
                                dest: result_local,
                                op: mir::BinOp::Sub,
                                left: mir::Operand::Constant(mir::Constant::Int(0)),
                                right: operand_val,
                            },
                            span: None,
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    hir::UnOp::Not => {
                        let operand_type = {
                            let ty = self.get_expr_type(operand_expr, hir_module);
                            // Same as yield path: generators pass values as raw
                            // i64, so Any must use Int truthiness.
                            if matches!(ty, Type::Any) {
                                Type::Int
                            } else {
                                ty
                            }
                        };
                        let bool_operand = self.convert_to_bool_in_block(
                            operand_val,
                            &operand_type,
                            mir_func,
                            block,
                        );
                        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::UnOp {
                                dest: result_local,
                                op: mir::UnOp::Not,
                                operand: bool_operand,
                            },
                            span: None,
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    _ => Ok(operand_val),
                }
            }
            other => Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                format!(
                    "unsupported expression in generator filter condition: {:?}",
                    other
                ),
                expr.span,
            )),
        }
    }

    /// Evaluate a while condition expression
    /// Returns the local containing the condition result
    pub(super) fn evaluate_while_condition(
        &mut self,
        cond_expr_id: hir::ExprId,
        hir_module: &hir::Module,
        block: &mut mir::BasicBlock,
        mir_func: &mut mir::Function,
        var_to_mir_local: &HashMap<VarId, LocalId>,
    ) -> Result<LocalId> {
        let cond_expr = &hir_module.exprs[cond_expr_id];
        let cond_result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Handle comparison condition (e.g., i < end)
        if let hir::ExprKind::Compare { left, op, right } = &cond_expr.kind {
            let left_expr = &hir_module.exprs[*left];
            let right_expr = &hir_module.exprs[*right];

            let left_operand = get_operand_for_expr(left_expr, var_to_mir_local)?;
            let right_operand = get_operand_for_expr(right_expr, var_to_mir_local)?;

            let mir_op = match op {
                hir::CmpOp::Lt => mir::BinOp::Lt,
                hir::CmpOp::LtE => mir::BinOp::LtE,
                hir::CmpOp::Gt => mir::BinOp::Gt,
                hir::CmpOp::GtE => mir::BinOp::GtE,
                hir::CmpOp::Eq => mir::BinOp::Eq,
                hir::CmpOp::NotEq => mir::BinOp::NotEq,
                _ => mir::BinOp::Lt,
            };

            block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::BinOp {
                    dest: cond_result_local,
                    op: mir_op,
                    left: left_operand,
                    right: right_operand,
                },
                span: None,
            });
        } else if matches!(cond_expr.kind, hir::ExprKind::Bool(true)) {
            // while True: — always true
            block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::Copy {
                    dest: cond_result_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(true)),
                },
                span: None,
            });
        } else if let hir::ExprKind::Var(var_id) = &cond_expr.kind {
            // while some_var: — copy the variable's boolean value
            if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::Copy {
                        dest: cond_result_local,
                        src: mir::Operand::Local(mir_local),
                    },
                    span: None,
                });
            } else {
                // Variable not in generator scope, default to true
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::Copy {
                        dest: cond_result_local,
                        src: mir::Operand::Constant(mir::Constant::Bool(true)),
                    },
                    span: None,
                });
            }
        } else {
            return Err(pyaot_diagnostics::CompilerError::codegen_error_at(
                "unsupported while-loop condition in generator \
                 (only comparisons, `while True`, and bare variables are supported)",
                cond_expr.span,
            ));
        }

        Ok(cond_result_local)
    }
}
