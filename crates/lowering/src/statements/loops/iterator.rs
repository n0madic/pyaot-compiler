//! Iterator protocol loop lowering: for x in generator

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a for-loop over an iterator/generator using the iterator protocol.
    /// Desugars `for x in gen: body` to:
    /// ```python
    /// __iter = iter(gen)  # or just gen for generators
    /// while True:
    ///     try:
    ///         x = next(__iter)
    ///     except StopIteration:
    ///         break
    ///     body
    /// ```
    /// For simplicity, we use a sentinel value approach:
    /// - next() returns 0 when exhausted (and sets exhausted flag)
    /// - We check the exhausted flag to break
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_iterator(
        &mut self,
        target: VarId,
        iter_id: hir::ExprId,
        elem_type: Type,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // 1. Lower the iterator expression (generator) and store in a temp local
        let iter_expr = &hir_module.exprs[iter_id];
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let iter_type = self.get_type_of_expr_id(iter_id, hir_module);

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        // 2. Create target variable local
        let target_local = self.get_or_create_local(target, elem_type.clone(), mir_func);
        self.insert_var_type(target, elem_type.clone());

        // 3. Create blocks for loop structure
        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let exit_bb = self.new_block();

        let header_id = header_bb.id;
        let body_id = body_bb.id;
        let exit_id = exit_bb.id;

        let has_else = !else_block.is_empty();
        let else_bb = if has_else {
            Some(self.new_block())
        } else {
            None
        };
        let else_id = else_bb.as_ref().map(|b| b.id);
        let normal_exit_id = else_id.unwrap_or(exit_id);

        // Jump to header
        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // 4. Header block: call next(), check if exhausted
        self.push_block(header_bb);

        // Call next() on the iterator (using no-exception variant for for-loops)
        let next_local = self.alloc_and_add_local(Type::Int, mir_func); // Raw value from generator

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: next_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            args: vec![mir::Operand::Local(iter_local)],
        });

        // Check if generator is exhausted
        let exhausted_local = self.alloc_and_add_local(Type::Bool, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: exhausted_local,
            func: mir::RuntimeFunc::Call(
                &pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED,
            ),
            args: vec![mir::Operand::Local(iter_local)],
        });

        // Branch: if exhausted goto exit/else, else goto body
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: normal_exit_id,
            else_block: body_id,
        };

        // 5. Body block: copy next value to target, execute body
        self.push_block(body_bb);

        // Copy the value to the target variable
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: target_local,
            src: mir::Operand::Local(next_local),
        });

        // If target is a global variable, sync the global with the local at start of each iteration
        // This is necessary because the loop uses a local for efficiency, but code inside
        // the loop body will use GlobalGet(ValueKind) to read the variable
        if self.is_global(&target) {
            let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
            let runtime_func = self.get_global_set_func(&elem_type);
            let effective_var_id = self.get_effective_var_id(target);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: runtime_func,
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_var_id)),
                    mir::Operand::Local(target_local),
                ],
            });
        }

        // Push loop context for break/continue: continue goes to header, break goes to exit
        self.push_loop(header_id, exit_id);

        // Execute body statements
        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        // Pop loop context
        self.pop_loop();

        // If no terminator, go back to header
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(header_id);
        }

        // 6. Else block (optional): executes on normal loop completion
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // 7. Exit block: continue after loop
        self.push_block(exit_bb);

        Ok(())
    }
}
