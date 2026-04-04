//! Logical operation lowering: short-circuit and/or, ternary if expressions

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a logical operation (and/or) with short-circuit evaluation.
    ///
    /// Python semantics:
    /// - `a and b` returns `b` if `a` is truthy, else returns `a`
    /// - `a or b` returns `a` if `a` is truthy, else returns `b`
    pub(in crate::expressions) fn lower_logical_op(
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
        let left_type = self.get_type_of_expr_id(left, hir_module);
        let right_type = self.get_type_of_expr_id(right, hir_module);

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
    pub(in crate::expressions) fn lower_if_expr(
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
        let then_ty = self.get_type_of_expr_id(then_val, hir_module);
        let else_ty = self.get_type_of_expr_id(else_val, hir_module);

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
        let cond_type = self.get_type_of_expr_id(cond, hir_module);
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
}
