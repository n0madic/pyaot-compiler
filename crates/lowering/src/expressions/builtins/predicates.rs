//! Predicate functions lowering: all(), any()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Select the appropriate Len/Get functions and item type based on iterable type.
    /// Returns (len_func, get_func, item_type, zero_constant).
    fn predicate_iter_info(
        &self,
        iterable_type: &Type,
    ) -> (mir::RuntimeFunc, mir::RuntimeFunc, Type, mir::Constant) {
        match iterable_type {
            Type::List(elem) => {
                let elem = elem.as_ref();
                match elem {
                    Type::Bool => (
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN),
                        mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_LIST_GET_BOOL,
                        ),
                        Type::Bool,
                        mir::Constant::Bool(false),
                    ),
                    Type::Int => (
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN),
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET_INT),
                        Type::Int,
                        mir::Constant::Int(0),
                    ),
                    _ => (
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN),
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                        Type::Int,
                        mir::Constant::Int(0),
                    ),
                }
            }
            Type::Tuple(_) => (
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_LEN),
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET),
                Type::Int,
                mir::Constant::Int(0),
            ),
            _ => (
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN),
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET),
                Type::Int,
                mir::Constant::Int(0),
            ),
        }
    }

    /// Lower all(iterable) -> bool
    pub(super) fn lower_all(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::Bool(true)));
        }

        let iterable_expr = &hir_module.exprs[args[0]];
        let iterable_operand = self.lower_expr(iterable_expr, hir_module, mir_func)?;
        let iterable_type = self.get_type_of_expr_id(args[0], hir_module);

        let (len_func, get_func, item_type, zero_const) = self.predicate_iter_info(&iterable_type);

        // Create result (default True)
        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: mir::Operand::Constant(mir::Constant::Bool(true)),
        });

        // Get length
        let len_local = self.emit_runtime_call(
            len_func,
            vec![iterable_operand.clone()],
            Type::Int,
            mir_func,
        );

        // Create loop counter
        let counter_local = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        // Create loop blocks
        let loop_header = self.new_block();
        let loop_body = self.new_block();
        let loop_exit = self.new_block();

        let loop_header_id = loop_header.id;
        let loop_body_id = loop_body.id;
        let loop_exit_id = loop_exit.id;

        // Jump to loop header
        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop header: check counter < len
        self.push_block(loop_header);

        let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cmp_local,
            op: mir::BinOp::Lt,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Local(len_local),
        });

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cmp_local),
            then_block: loop_body_id,
            else_block: loop_exit_id,
        };

        // Loop body
        self.push_block(loop_body);

        // Get item using the type-appropriate getter
        let item_local = self.emit_runtime_call(
            get_func,
            vec![iterable_operand.clone(), mir::Operand::Local(counter_local)],
            item_type.clone(),
            mir_func,
        );

        // Convert to bool: compare item != zero_const (works for both i8 and i64)
        let item_bool = self.alloc_and_add_local(Type::Bool, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: item_bool,
            op: mir::BinOp::NotEq,
            left: mir::Operand::Local(item_local),
            right: mir::Operand::Constant(zero_const),
        });

        // Check if item is False - if so, early exit
        let check_false = self.new_block();
        let continue_loop = self.new_block();
        let check_false_id = check_false.id;
        let continue_loop_id = continue_loop.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(item_bool),
            then_block: continue_loop_id, // item is True, continue
            else_block: check_false_id,   // item is False, set result and exit
        };

        // False path: set result = False and jump to exit
        self.push_block(check_false);

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: mir::Operand::Constant(mir::Constant::Bool(false)),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(loop_exit_id);

        // Continue path: increment counter and loop back
        self.push_block(continue_loop);

        let temp_counter = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: temp_counter,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Local(temp_counter),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop exit
        self.push_block(loop_exit);

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower any(iterable) -> bool
    pub(super) fn lower_any(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::Bool(false)));
        }

        let iterable_expr = &hir_module.exprs[args[0]];
        let iterable_operand = self.lower_expr(iterable_expr, hir_module, mir_func)?;
        let iterable_type = self.get_type_of_expr_id(args[0], hir_module);

        let (len_func, get_func, item_type, zero_const) = self.predicate_iter_info(&iterable_type);

        // Create result (default False)
        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: mir::Operand::Constant(mir::Constant::Bool(false)),
        });

        // Get length
        let len_local = self.emit_runtime_call(
            len_func,
            vec![iterable_operand.clone()],
            Type::Int,
            mir_func,
        );

        // Create loop counter
        let counter_local = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        // Create loop blocks
        let loop_header = self.new_block();
        let loop_body = self.new_block();
        let loop_exit = self.new_block();

        let loop_header_id = loop_header.id;
        let loop_body_id = loop_body.id;
        let loop_exit_id = loop_exit.id;

        // Jump to loop header
        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop header: check counter < len
        self.push_block(loop_header);

        let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cmp_local,
            op: mir::BinOp::Lt,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Local(len_local),
        });

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cmp_local),
            then_block: loop_body_id,
            else_block: loop_exit_id,
        };

        // Loop body
        self.push_block(loop_body);

        // Get item using the type-appropriate getter
        let item_local = self.emit_runtime_call(
            get_func,
            vec![iterable_operand.clone(), mir::Operand::Local(counter_local)],
            item_type.clone(),
            mir_func,
        );

        // Convert to bool: compare item != zero_const (works for both i8 and i64)
        let item_bool = self.alloc_and_add_local(Type::Bool, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: item_bool,
            op: mir::BinOp::NotEq,
            left: mir::Operand::Local(item_local),
            right: mir::Operand::Constant(zero_const),
        });

        // Check if item is True - if so, early exit
        let check_true = self.new_block();
        let continue_loop = self.new_block();
        let check_true_id = check_true.id;
        let continue_loop_id = continue_loop.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(item_bool),
            then_block: check_true_id, // item is True, set result and exit
            else_block: continue_loop_id, // item is False, continue
        };

        // True path: set result = True and jump to exit
        self.push_block(check_true);

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: result_local,
            src: mir::Operand::Constant(mir::Constant::Bool(true)),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(loop_exit_id);

        // Continue path: increment counter and loop back
        self.push_block(continue_loop);

        let temp_counter = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: temp_counter,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Local(temp_counter),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop exit
        self.push_block(loop_exit);

        Ok(mir::Operand::Local(result_local))
    }
}
