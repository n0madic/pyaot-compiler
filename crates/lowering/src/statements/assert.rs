//! Assert statement lowering
//!
//! Handles: Assert

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower an assert statement
    pub(crate) fn lower_assert(
        &mut self,
        cond: hir::ExprId,
        msg: Option<&hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Evaluate condition
        let cond_expr = &hir_module.exprs[cond];
        let cond_type = self.get_expr_type(cond_expr, hir_module);
        let cond_operand = self.lower_expr(cond_expr, hir_module, mir_func)?;

        // Convert to bool if needed (same pattern as lower_if / lower_while)
        let final_cond_operand =
            self.emit_truthiness_conversion_if_needed(cond_operand, &cond_type, mir_func);

        // Create blocks for branching
        let fail_bb = self.new_block();
        let continue_bb = self.new_block();

        let fail_id = fail_bb.id;
        let continue_id = continue_bb.id;

        // Branch: if condition is true, continue; if false, fail
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: final_cond_operand,
            then_block: continue_id,
            else_block: fail_id,
        };

        // Fail block: call rt_assert_fail and unreachable
        self.push_block(fail_bb);

        // Create a dummy local for the runtime call result (void)
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Prepare message argument (null pointer if no message, string constant if present)
        // Note: We pass string literals directly as constants for efficiency.
        // The codegen will create a raw C string in the data section.
        if let Some(msg_expr_id) = msg {
            let msg_expr = &hir_module.exprs[*msg_expr_id];
            // Check if message is a string literal - pass it directly as constant
            if let hir::ExprKind::Str(s) = &msg_expr.kind {
                let msg_operand = mir::Operand::Constant(mir::Constant::Str(*s));
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::AssertFail,
                    args: vec![msg_operand],
                });
            } else {
                // For non-literal strings (f-strings, variables), lower the expression
                // and pass the string object to AssertFailObj
                let msg_operand = self.lower_expr(msg_expr, hir_module, mir_func)?;
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::AssertFailObj,
                    args: vec![msg_operand],
                });
            }
        } else {
            // No message - pass null pointer
            let msg_operand = mir::Operand::Constant(mir::Constant::Int(0));
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::AssertFail,
                args: vec![msg_operand],
            });
        }

        // rt_assert_fail doesn't return, so mark as unreachable
        self.current_block_mut().terminator = mir::Terminator::Unreachable;

        // Continue block: execution continues here if assertion passed
        self.push_block(continue_bb);

        Ok(())
    }
}
