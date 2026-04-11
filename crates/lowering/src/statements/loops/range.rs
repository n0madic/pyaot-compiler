//! Range loop lowering: for x in range(...)

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;
use crate::utils::{get_step_direction, StepDirection};

impl<'a> Lowering<'a> {
    /// Lower a for loop over range()
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_range(
        &mut self,
        target: VarId,
        args: &[hir::ExprId],
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Get the target variable (for range(), target is always int)
        // Track variable type for type inference (needed for dict key boxing, etc.)
        self.insert_var_type(target, Type::Int);
        let target_local = self.get_or_create_local(target, Type::Int, mir_func);

        // Parse range arguments: range(stop) or range(start, stop) or range(start, stop, step)
        // Also determine step direction for choosing comparison operator
        let (start_op, stop_op, step_op, step_direction) = match args.len() {
            1 => {
                // range(stop): start=0, stop=args[0], step=1
                let stop_expr = &hir_module.exprs[args[0]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    stop,
                    mir::Operand::Constant(mir::Constant::Int(1)),
                    StepDirection::Positive, // step=1 is always positive
                )
            }
            2 => {
                // range(start, stop): start=args[0], stop=args[1], step=1
                let start_expr = &hir_module.exprs[args[0]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop_expr = &hir_module.exprs[args[1]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (
                    start,
                    stop,
                    mir::Operand::Constant(mir::Constant::Int(1)),
                    StepDirection::Positive, // step=1 is always positive
                )
            }
            3 => {
                // range(start, stop, step): all provided
                // Determine step direction before lowering
                let step_expr = &hir_module.exprs[args[2]];
                let direction = get_step_direction(step_expr, hir_module);

                let start_expr = &hir_module.exprs[args[0]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop_expr = &hir_module.exprs[args[1]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                let step = self.lower_expr(step_expr, hir_module, mir_func)?;
                (start, stop, step, direction)
            }
            _ => {
                // Semantic analysis guarantees range() is called with 1–3 arguments;
                // any other count is an internal compiler error.
                panic!(
                    "internal error: range() requires 1-3 arguments, got {}",
                    args.len()
                );
            }
        };

        // Initialize loop variable: target = start
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: target_local,
            src: start_op,
        });

        // Create blocks for loop structure
        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let increment_bb = self.new_block();
        let exit_bb = self.new_block();

        let has_else = !else_block.is_empty();
        let else_bb = if has_else {
            Some(self.new_block())
        } else {
            None
        };

        let header_id = header_bb.id;
        let body_id = body_bb.id;
        let increment_id = increment_bb.id;
        let else_id = else_bb.as_ref().map(|b| b.id);
        let exit_id = exit_bb.id;
        // Normal exit (condition false) goes to else block if present, otherwise exit
        let normal_exit_id = else_id.unwrap_or(exit_id);

        // Jump to header
        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Header: check condition based on step direction
        // - Positive step: target < stop
        // - Negative step: target > stop
        // - Unknown: runtime check using (step > 0 && target < stop) || (step <= 0 && target > stop)
        self.push_block(header_bb);

        let cond_local = self.alloc_and_add_local(Type::Bool, mir_func);

        match step_direction {
            StepDirection::Positive => {
                // Simple case: target < stop
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: cond_local,
                    op: mir::BinOp::Lt,
                    left: mir::Operand::Local(target_local),
                    right: stop_op,
                });
            }
            StepDirection::Negative => {
                // Negative step: target > stop
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: cond_local,
                    op: mir::BinOp::Gt,
                    left: mir::Operand::Local(target_local),
                    right: stop_op,
                });
            }
            StepDirection::Unknown => {
                // Runtime check: (step > 0 && target < stop) || (step <= 0 && target > stop)
                self.emit_range_runtime_check(
                    cond_local,
                    target_local,
                    stop_op,
                    step_op.clone(),
                    mir_func,
                );
            }
        }

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cond_local),
            then_block: body_id,
            else_block: normal_exit_id,
        };

        // Body: execute loop body statements
        self.push_block(body_bb);

        // If target is a global variable, sync the global with the local at start of each iteration
        // This is necessary because the loop uses a local for efficiency, but code inside
        // the loop body will use GlobalGet(ValueKind::Int) to read the variable
        if self.is_global(&target) {
            let runtime_func = self.get_global_set_func(&Type::Int);
            let effective_var_id = self.get_effective_var_id(target);
            self.emit_runtime_call(
                runtime_func,
                vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_var_id)),
                    mir::Operand::Local(target_local),
                ],
                Type::None,
                mir_func,
            );
        }

        // Push loop context: continue jumps to increment, break jumps to exit
        self.push_loop(increment_id, exit_id);

        // Execute body statements
        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        // If no terminator, fall through to increment
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(increment_id);
        }

        // Pop loop context
        self.pop_loop();

        // Increment block: target = target + step, then jump to header
        self.push_block(increment_bb);

        let inc_local = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: inc_local,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(target_local),
            right: step_op,
        });

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: target_local,
            src: mir::Operand::Local(inc_local),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Else block (executed if loop completes without break)
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit block
        self.push_block(exit_bb);

        Ok(())
    }

    /// Emit runtime check for range with unknown step direction.
    /// Used by both range loops and enumerate-range loops.
    pub(crate) fn emit_range_runtime_check(
        &mut self,
        cond_local: pyaot_utils::LocalId,
        target_local: pyaot_utils::LocalId,
        stop_op: mir::Operand,
        step_op: mir::Operand,
        mir_func: &mut mir::Function,
    ) {
        // Allocate locals for intermediate results
        let step_positive_local = self.alloc_and_add_local(Type::Bool, mir_func);
        let cond_lt_local = self.alloc_and_add_local(Type::Bool, mir_func);
        let cond_gt_local = self.alloc_and_add_local(Type::Bool, mir_func);
        let pos_branch_local = self.alloc_and_add_local(Type::Bool, mir_func);
        let neg_branch_local = self.alloc_and_add_local(Type::Bool, mir_func);
        let step_not_positive_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // step_positive = step > 0
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: step_positive_local,
            op: mir::BinOp::Gt,
            left: step_op,
            right: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        // cond_lt = target < stop
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cond_lt_local,
            op: mir::BinOp::Lt,
            left: mir::Operand::Local(target_local),
            right: stop_op.clone(),
        });

        // cond_gt = target > stop
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cond_gt_local,
            op: mir::BinOp::Gt,
            left: mir::Operand::Local(target_local),
            right: stop_op,
        });

        // pos_branch = step_positive && cond_lt
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: pos_branch_local,
            op: mir::BinOp::And,
            left: mir::Operand::Local(step_positive_local),
            right: mir::Operand::Local(cond_lt_local),
        });

        // step_not_positive = !step_positive (step <= 0)
        self.emit_instruction(mir::InstructionKind::UnOp {
            dest: step_not_positive_local,
            op: mir::UnOp::Not,
            operand: mir::Operand::Local(step_positive_local),
        });

        // neg_branch = step_not_positive && cond_gt
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: neg_branch_local,
            op: mir::BinOp::And,
            left: mir::Operand::Local(step_not_positive_local),
            right: mir::Operand::Local(cond_gt_local),
        });

        // cond = pos_branch || neg_branch
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cond_local,
            op: mir::BinOp::Or,
            left: mir::Operand::Local(pos_branch_local),
            right: mir::Operand::Local(neg_branch_local),
        });
    }
}
