//! Helper functions for generator expression lowering
//!
//! This module provides utility functions for lowering expressions within
//! generators, particularly for yield values and filter conditions.

use std::collections::HashMap;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{LocalId, VarId};

use super::YieldInfo;
use crate::context::Lowering;

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
                    Ok(mir::Operand::Constant(mir::Constant::Int(0)))
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
                                        func: mir::RuntimeFunc::InstanceGetField,
                                        args: vec![
                                            mir::Operand::Local(loop_var_local),
                                            mir::Operand::Constant(mir::Constant::Int(
                                                offset as i64,
                                            )),
                                        ],
                                    },
                                });

                                // For float fields: box the f64 into a heap FloatObj so the
                                // caller's UnboxFloat call can properly dereference it.
                                // The resume protocol always passes boxed values through the
                                // iterator's *mut Obj return slot.
                                if matches!(field_ty, Type::Float) {
                                    let boxed_local = self.alloc_and_add_local(Type::Str, mir_func); // Str = i64 ptr
                                    block.instructions.push(mir::Instruction {
                                        kind: mir::InstructionKind::RuntimeCall {
                                            dest: boxed_local,
                                            func: mir::RuntimeFunc::BoxFloat,
                                            args: vec![mir::Operand::Local(raw_local)],
                                        },
                                    });
                                    return Ok(mir::Operand::Local(boxed_local));
                                }

                                return Ok(mir::Operand::Local(raw_local));
                            }
                        }
                    }
                }
                // Unsupported attribute access pattern — fall back to None
                Ok(mir::Operand::Constant(mir::Constant::Int(0)))
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
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    hir::UnOp::Not => {
                        // not x => x == 0
                        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        block.instructions.push(mir::Instruction {
                            kind: mir::InstructionKind::BinOp {
                                dest: result_local,
                                op: mir::BinOp::Eq,
                                left: operand_val,
                                right: mir::Operand::Constant(mir::Constant::Int(0)),
                            },
                        });
                        Ok(mir::Operand::Local(result_local))
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
                let mir_op = match op {
                    hir::BinOp::Add => mir::BinOp::Add,
                    hir::BinOp::Sub => mir::BinOp::Sub,
                    hir::BinOp::Mul => mir::BinOp::Mul,
                    hir::BinOp::Div => mir::BinOp::Div,
                    hir::BinOp::Mod => mir::BinOp::Mod,
                    hir::BinOp::FloorDiv => mir::BinOp::FloorDiv,
                    _ => mir::BinOp::Add, // Default fallback
                };

                // Emit the binary operation
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir_op,
                        left: left_op,
                        right: right_op,
                    },
                });

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // For other expressions, return 0 as fallback
                Ok(mir::Operand::Constant(mir::Constant::Int(0)))
            }
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
                    Ok(mir::Operand::Constant(mir::Constant::Int(0)))
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

                let mir_op = match op {
                    hir::BinOp::Add => mir::BinOp::Add,
                    hir::BinOp::Sub => mir::BinOp::Sub,
                    hir::BinOp::Mul => mir::BinOp::Mul,
                    hir::BinOp::Div => mir::BinOp::Div,
                    hir::BinOp::Mod => mir::BinOp::Mod,
                    hir::BinOp::FloorDiv => mir::BinOp::FloorDiv,
                    _ => mir::BinOp::Add,
                };

                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir_op,
                        left: left_op,
                        right: right_op,
                    },
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
                });

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // For other expressions, return false as fallback
                Ok(mir::Operand::Constant(mir::Constant::Bool(false)))
            }
        }
    }

    /// Helper to get an operand for an expression
    pub(super) fn get_operand_for_expr(
        &self,
        expr: &hir::Expr,
        var_to_mir_local: &HashMap<VarId, LocalId>,
    ) -> mir::Operand {
        match &expr.kind {
            hir::ExprKind::Int(n) => mir::Operand::Constant(mir::Constant::Int(*n)),
            hir::ExprKind::Var(var_id) => {
                if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                    mir::Operand::Local(mir_local)
                } else {
                    mir::Operand::Constant(mir::Constant::Int(0))
                }
            }
            _ => mir::Operand::Constant(mir::Constant::Int(0)),
        }
    }

    /// Collect yield information from the function body (in order)
    /// Returns YieldInfo for each yield, including assignment targets
    pub(super) fn collect_yield_info(
        &self,
        body: &[hir::StmtId],
        hir_module: &hir::Module,
    ) -> Vec<YieldInfo> {
        let mut yields = Vec::new();
        for stmt_id in body {
            self.collect_yields_from_stmt_with_target(*stmt_id, hir_module, &mut yields);
        }
        yields
    }

    fn collect_yields_from_stmt_with_target(
        &self,
        stmt_id: hir::StmtId,
        hir_module: &hir::Module,
        yields: &mut Vec<YieldInfo>,
    ) {
        let stmt = &hir_module.stmts[stmt_id];
        match &stmt.kind {
            hir::StmtKind::Expr(expr_id) => {
                self.collect_yields_from_expr_with_target(*expr_id, None, hir_module, yields);
            }
            hir::StmtKind::Assign { target, value, .. } => {
                // Check if the value is a yield expression
                let value_expr = &hir_module.exprs[*value];
                if matches!(value_expr.kind, hir::ExprKind::Yield(_)) {
                    // This is `target = yield value` - record the assignment target
                    self.collect_yields_from_expr_with_target(
                        *value,
                        Some(*target),
                        hir_module,
                        yields,
                    );
                } else {
                    self.collect_yields_from_expr_with_target(*value, None, hir_module, yields);
                }
            }
            hir::StmtKind::Return(Some(expr_id)) => {
                self.collect_yields_from_expr_with_target(*expr_id, None, hir_module, yields);
            }
            hir::StmtKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
                for s in then_block {
                    self.collect_yields_from_stmt_with_target(*s, hir_module, yields);
                }
                for s in else_block {
                    self.collect_yields_from_stmt_with_target(*s, hir_module, yields);
                }
            }
            hir::StmtKind::While {
                cond,
                body,
                else_block,
            } => {
                self.collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
                for s in body {
                    self.collect_yields_from_stmt_with_target(*s, hir_module, yields);
                }
                for s in else_block {
                    self.collect_yields_from_stmt_with_target(*s, hir_module, yields);
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
                self.collect_yields_from_expr_with_target(*iter, None, hir_module, yields);
                for s in body {
                    self.collect_yields_from_stmt_with_target(*s, hir_module, yields);
                }
                for s in else_block {
                    self.collect_yields_from_stmt_with_target(*s, hir_module, yields);
                }
            }
            _ => {}
        }
    }

    fn collect_yields_from_expr_with_target(
        &self,
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
                self.collect_yields_from_expr_with_target(*left, None, hir_module, yields);
                self.collect_yields_from_expr_with_target(*right, None, hir_module, yields);
            }
            hir::ExprKind::UnOp { operand, .. } => {
                self.collect_yields_from_expr_with_target(*operand, None, hir_module, yields);
            }
            hir::ExprKind::Call { func, args, .. } => {
                self.collect_yields_from_expr_with_target(*func, None, hir_module, yields);
                for a in args {
                    let arg_id = match a {
                        hir::CallArg::Regular(id) => id,
                        hir::CallArg::Starred(id) => id,
                    };
                    self.collect_yields_from_expr_with_target(*arg_id, None, hir_module, yields);
                }
            }
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                self.collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
                self.collect_yields_from_expr_with_target(*then_val, None, hir_module, yields);
                self.collect_yields_from_expr_with_target(*else_val, None, hir_module, yields);
            }
            _ => {}
        }
    }

    /// Lower a simple statement in generator context
    /// Handles: assignments with simple expressions, expression statements
    pub(super) fn lower_simple_stmt_for_generator(
        &mut self,
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
                            });
                        }
                        hir::ExprKind::Var(src_var) => {
                            if let Some(&src_mir_local) = var_to_mir_local.get(src_var) {
                                block.instructions.push(mir::Instruction {
                                    kind: mir::InstructionKind::Copy {
                                        dest: dest_mir_local,
                                        src: mir::Operand::Local(src_mir_local),
                                    },
                                });
                            }
                        }
                        hir::ExprKind::BinOp { left, op, right } => {
                            let left_expr = &hir_module.exprs[*left];
                            let right_expr = &hir_module.exprs[*right];
                            let left_op = self.get_operand_for_expr(left_expr, var_to_mir_local);
                            let right_op = self.get_operand_for_expr(right_expr, var_to_mir_local);
                            let mir_op = match op {
                                hir::BinOp::Add => mir::BinOp::Add,
                                hir::BinOp::Sub => mir::BinOp::Sub,
                                hir::BinOp::Mul => mir::BinOp::Mul,
                                hir::BinOp::Div => mir::BinOp::Div,
                                hir::BinOp::Mod => mir::BinOp::Mod,
                                hir::BinOp::FloorDiv => mir::BinOp::FloorDiv,
                                _ => mir::BinOp::Add,
                            };
                            block.instructions.push(mir::Instruction {
                                kind: mir::InstructionKind::BinOp {
                                    dest: dest_mir_local,
                                    op: mir_op,
                                    left: left_op,
                                    right: right_op,
                                },
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

            let left_operand = self.get_operand_for_expr(left_expr, var_to_mir_local);
            let right_operand = self.get_operand_for_expr(right_expr, var_to_mir_local);

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
            });
        } else if matches!(cond_expr.kind, hir::ExprKind::Bool(true)) {
            // while True: — always true
            block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::Copy {
                    dest: cond_result_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(true)),
                },
            });
        } else if let hir::ExprKind::Var(var_id) = &cond_expr.kind {
            // while some_var: — copy the variable's boolean value
            if let Some(&mir_local) = var_to_mir_local.get(var_id) {
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::Copy {
                        dest: cond_result_local,
                        src: mir::Operand::Local(mir_local),
                    },
                });
            } else {
                // Variable not in generator scope, default to true
                block.instructions.push(mir::Instruction {
                    kind: mir::InstructionKind::Copy {
                        dest: cond_result_local,
                        src: mir::Operand::Constant(mir::Constant::Bool(true)),
                    },
                });
            }
        } else {
            // TODO: handle other expression kinds in generator while conditions
            block.instructions.push(mir::Instruction {
                kind: mir::InstructionKind::Copy {
                    dest: cond_result_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(true)),
                },
            });
        }

        Ok(cond_result_local)
    }
}
