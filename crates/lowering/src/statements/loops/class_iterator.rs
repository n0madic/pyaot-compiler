//! Class iterator protocol loop lowering: for x in class_obj
//!
//! Generates a for-loop CFG for classes implementing __iter__/__next__,
//! using try/except StopIteration to detect exhaustion.
//!
//! CFG structure:
//! ```text
//! [pre-loop]  Call __iter__(obj) -> iter_local
//! [setup]     ExcPushFrame, TrySetjmp -> try_body | handler
//! [try_body]  CallDirect(__next__(iter)), ExcPopFrame -> body_bb
//! [handler]   ExcCheckType(StopIteration) -> match: ExcClear, goto exit/else | no match: Reraise
//! [body_bb]   assign target, execute body -> setup
//! [exit/else] optional else_block -> exit
//! [exit]      continue after loop
//! ```

use pyaot_core_defs::BuiltinExceptionKind;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a for-loop over a class implementing __iter__/__next__.
    ///
    /// Uses the exception-based iterator protocol: each iteration wraps
    /// `__next__()` in a try/except StopIteration block.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_class_iterator(
        &mut self,
        target: VarId,
        iter_expr: &hir::Expr,
        iter_type: &Type,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let (class_id, iter_func_id, next_func_id) = match iter_type {
            Type::Class { class_id, .. } => {
                let class_info = self
                    .get_class_info(class_id)
                    .expect("class info must exist for class iterator");
                let iter_func = class_info
                    .get_dunder_func("__iter__")
                    .expect("class must have __iter__ for class iterator loop");
                let next_func = class_info
                    .get_dunder_func("__next__")
                    .expect("class must have __next__ for class iterator loop");
                (*class_id, iter_func, next_func)
            }
            _ => unreachable!("lower_for_class_iterator called with non-class type"),
        };

        // Determine element type from __next__ return type
        let elem_type = self
            .get_func_return_type(&next_func_id)
            .cloned()
            .unwrap_or(Type::Any);

        // 1. Lower the iterable expression and call __iter__(obj)
        let obj_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let _ = class_id; // Used above for class_info lookup
        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::CallDirect {
            dest: iter_local,
            func: iter_func_id,
            args: vec![obj_operand],
        });

        // 2. Create target variable local
        let target_local = self.get_or_create_local(target, elem_type.clone(), mir_func);
        self.insert_var_type(target, elem_type.clone());

        // 3. Create blocks for the loop
        let setup_bb = self.new_block();
        let try_body_bb = self.new_block();
        let handler_bb = self.new_block();
        let body_bb = self.new_block();
        let exit_bb = self.new_block();

        let setup_id = setup_bb.id;
        let try_body_id = try_body_bb.id;
        let handler_id = handler_bb.id;
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

        // Jump to setup (loop header)
        self.current_block_mut().terminator = mir::Terminator::Goto(setup_id);

        // 4. Setup block: push exception frame + setjmp
        self.push_block(setup_bb);

        let frame_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::ExcPushFrame { frame_local });

        self.current_block_mut().terminator = mir::Terminator::TrySetjmp {
            frame_local,
            try_body: try_body_id,
            handler_entry: handler_id,
        };

        // 5. Try body block: call __next__(iter)
        self.push_block(try_body_bb);

        let next_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::CallDirect {
            dest: next_local,
            func: next_func_id,
            args: vec![mir::Operand::Local(iter_local)],
        });

        // Pop exception frame on success (before entering body)
        self.emit_instruction(mir::InstructionKind::ExcPopFrame);

        // Jump to body block
        self.current_block_mut().terminator = mir::Terminator::Goto(body_id);

        // 6. Handler block: check if StopIteration
        self.push_block(handler_bb);

        let check_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::ExcCheckClass {
            dest: check_local,
            class_id: BuiltinExceptionKind::StopIteration.tag(),
        });

        let reraise_bb = self.new_block();
        let stop_iter_bb = self.new_block();
        let reraise_id = reraise_bb.id;
        let stop_iter_id = stop_iter_bb.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(check_local),
            then_block: stop_iter_id,
            else_block: reraise_id,
        };

        // Stop iteration matched: clear exception and exit loop
        self.push_block(stop_iter_bb);
        self.emit_instruction(mir::InstructionKind::ExcClear);
        self.current_block_mut().terminator = mir::Terminator::Goto(normal_exit_id);

        // Not StopIteration: reraise
        self.push_block(reraise_bb);
        self.current_block_mut().terminator = mir::Terminator::Reraise;

        // 7. Body block: assign target, execute body, loop back
        self.push_block(body_bb);

        // Copy next value to target variable
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: target_local,
            src: mir::Operand::Local(next_local),
        });

        // If target is a global variable, sync the global
        if self.is_global(&target) {
            let runtime_func = self.get_global_set_func(&elem_type);
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

        // Push loop context for break/continue
        self.push_loop(setup_id, exit_id);

        // Execute body statements
        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        // Pop loop context
        self.pop_loop();

        // Loop back to setup
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(setup_id);
        }

        // 8. Else block (optional): executes on normal loop completion
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // 9. Exit block: continue after loop
        self.push_block(exit_bb);

        Ok(())
    }
}
