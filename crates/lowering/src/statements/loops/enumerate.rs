//! Enumerate loop lowering: for i, v in enumerate(...)
//!
//! Optimized paths for enumerate over different iterable types.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;
use crate::utils::{get_iterable_info, IterableKind};

impl<'a> Lowering<'a> {
    /// Optimized enumerate loop: for i, v in enumerate(items, start=0)
    /// Zero-allocation path using indexed iteration with a counter variable
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_enumerate_optimized(
        &mut self,
        targets: &[VarId],
        enum_args: &[hir::ExprId],
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        if enum_args.is_empty() {
            return Ok(());
        }

        let counter_var = targets[0]; // i
        let elem_var = targets[1]; // v

        // Get start value (second arg to enumerate() or default 0)
        let start_operand = if enum_args.len() > 1 {
            let start_expr = &hir_module.exprs[enum_args[1]];
            self.lower_expr(start_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::Int(0))
        };

        let inner_iter_expr = &hir_module.exprs[enum_args[0]];

        // Check if the inner iterable is range() → use range-like loop
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            args: range_args,
            ..
        } = &inner_iter_expr.kind
        {
            return self.lower_for_enumerate_range(
                counter_var,
                elem_var,
                start_operand,
                range_args,
                body,
                else_block,
                hir_module,
                mir_func,
            );
        }

        // General iterable path: use indexed iteration with counter
        let iter_type = self.get_expr_type(inner_iter_expr, hir_module);
        if let Some((kind, elem_type)) = get_iterable_info(&iter_type) {
            self.lower_for_enumerate_iterable(
                counter_var,
                elem_var,
                start_operand,
                inner_iter_expr,
                kind,
                elem_type,
                body,
                else_block,
                hir_module,
                mir_func,
            )?;
        } else {
            // Fallback for unknown types: use iterator protocol with enumerate runtime
            self.lower_for_enumerate_iterator(
                counter_var,
                elem_var,
                start_operand,
                inner_iter_expr,
                Type::Any,
                body,
                else_block,
                hir_module,
                mir_func,
            )?;
        }
        Ok(())
    }

    /// Enumerate over a range: for i, v in enumerate(range(...), start)
    #[allow(clippy::too_many_arguments)]
    fn lower_for_enumerate_range(
        &mut self,
        counter_var: VarId,
        elem_var: VarId,
        start_operand: mir::Operand,
        range_args: &[hir::ExprId],
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Parse range arguments
        let (range_start, range_stop, range_step) = match range_args.len() {
            1 => {
                let stop_expr = &hir_module.exprs[range_args[0]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    stop,
                    mir::Operand::Constant(mir::Constant::Int(1)),
                )
            }
            2 => {
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (start, stop, mir::Operand::Constant(mir::Constant::Int(1)))
            }
            3 => {
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let step_expr = &hir_module.exprs[range_args[2]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                let step = self.lower_expr(step_expr, hir_module, mir_func)?;
                (start, stop, step)
            }
            _ => return Ok(()),
        };

        // Set up counter variable
        self.insert_var_type(counter_var, Type::Int);
        let counter_local = self.get_or_create_local(counter_var, Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: start_operand,
        });

        // Set up elem variable (range yields ints)
        self.insert_var_type(elem_var, Type::Int);
        let elem_local = self.get_or_create_local(elem_var, Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: elem_local,
            src: range_start,
        });

        // Create blocks for loop structure
        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let increment_bb = self.new_block();
        let exit_bb = self.new_block();
        let else_bb = if !else_block.is_empty() {
            Some(self.new_block())
        } else {
            None
        };

        let header_id = header_bb.id;
        let body_id = body_bb.id;
        let increment_id = increment_bb.id;
        let exit_id = exit_bb.id;
        let normal_exit_id = else_bb.as_ref().map(|bb| bb.id).unwrap_or(exit_id);

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Header: check loop condition based on step direction
        self.push_block(header_bb);

        let cond_local = self.alloc_and_add_local(Type::Bool, mir_func);

        // Determine step direction for correct comparison operator
        let cmp_op = if range_args.len() >= 3 {
            let step_expr = &hir_module.exprs[range_args[2]];
            match crate::utils::get_step_direction(step_expr, hir_module) {
                crate::utils::StepDirection::Positive => mir::BinOp::Lt,
                crate::utils::StepDirection::Negative => mir::BinOp::Gt,
                // TODO: StepDirection::Unknown means the step sign cannot be determined
                // at compile time (e.g. a variable step).  Defaulting to Lt (positive
                // step direction) produces correct code only for positive steps; a
                // negative runtime step will cause an infinite loop.  A full fix
                // requires emitting a runtime check similar to emit_range_runtime_check
                // used in the non-enumerate range loop path.
                crate::utils::StepDirection::Unknown => mir::BinOp::Lt,
            }
        } else {
            mir::BinOp::Lt // Default: positive step
        };

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cond_local,
            op: cmp_op,
            left: mir::Operand::Local(elem_local),
            right: range_stop,
        });

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cond_local),
            then_block: body_id,
            else_block: normal_exit_id,
        };

        // Body
        self.push_block(body_bb);

        self.push_loop(increment_id, exit_id);

        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        self.pop_loop();

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(increment_id);
        }

        // Increment: elem += step, counter += 1
        self.push_block(increment_bb);

        let inc_elem = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: inc_elem,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(elem_local),
            right: range_step,
        });
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: elem_local,
            src: mir::Operand::Local(inc_elem),
        });

        let inc_counter = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: inc_counter,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Local(inc_counter),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Else block
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit
        self.push_block(exit_bb);

        Ok(())
    }

    /// Enumerate over an iterable: for i, v in enumerate(items, start)
    /// Uses indexed iteration with a separate counter variable
    #[allow(clippy::too_many_arguments)]
    fn lower_for_enumerate_iterable(
        &mut self,
        counter_var: VarId,
        elem_var: VarId,
        start_operand: mir::Operand,
        iter_expr: &hir::Expr,
        iterable_kind: IterableKind,
        elem_type: Type,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // For iterators, use the iterator protocol with enumerate runtime
        if iterable_kind == IterableKind::Iterator {
            return self.lower_for_enumerate_iterator(
                counter_var,
                elem_var,
                start_operand,
                iter_expr,
                elem_type,
                body,
                else_block,
                hir_module,
                mir_func,
            );
        }

        // Lower the iterator expression
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let iter_type = self.get_expr_type(iter_expr, hir_module);

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        // Get length
        let len_local = self.alloc_and_add_local(Type::Int, mir_func);

        // Handle dict/set conversion to list for iteration
        let (actual_iter_local, _converted) = if iterable_kind == IterableKind::Dict {
            let keys_local =
                self.alloc_and_add_local(Type::List(Box::new(elem_type.clone())), mir_func);
            let key_elem_tag = Self::elem_tag_for_type(&elem_type);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: keys_local,
                func: mir::RuntimeFunc::DictKeys,
                args: vec![
                    mir::Operand::Local(iter_local),
                    mir::Operand::Constant(mir::Constant::Int(key_elem_tag)),
                ],
            });
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: len_local,
                func: mir::RuntimeFunc::ListLen,
                args: vec![mir::Operand::Local(keys_local)],
            });
            (keys_local, true)
        } else if iterable_kind == IterableKind::Set {
            let list_local =
                self.alloc_and_add_local(Type::List(Box::new(elem_type.clone())), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: list_local,
                func: mir::RuntimeFunc::SetToList,
                args: vec![mir::Operand::Local(iter_local)],
            });
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: len_local,
                func: mir::RuntimeFunc::ListLen,
                args: vec![mir::Operand::Local(list_local)],
            });
            (list_local, true)
        } else {
            let len_func = match iterable_kind {
                IterableKind::List => mir::RuntimeFunc::ListLen,
                IterableKind::Tuple => mir::RuntimeFunc::TupleLen,
                IterableKind::Str => mir::RuntimeFunc::StrLenInt,
                IterableKind::Bytes => mir::RuntimeFunc::BytesLen,
                _ => unreachable!(),
            };
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: len_local,
                func: len_func,
                args: vec![mir::Operand::Local(iter_local)],
            });
            (iter_local, false)
        };

        // Initialize index and counter
        let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: idx_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        self.insert_var_type(counter_var, Type::Int);
        let counter_local = self.get_or_create_local(counter_var, Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: start_operand,
        });

        self.insert_var_type(elem_var, elem_type.clone());
        let elem_local = self.get_or_create_local(elem_var, elem_type.clone(), mir_func);

        // Create blocks
        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let increment_bb = self.new_block();
        let exit_bb = self.new_block();
        let else_bb = if !else_block.is_empty() {
            Some(self.new_block())
        } else {
            None
        };

        let header_id = header_bb.id;
        let body_id = body_bb.id;
        let increment_id = increment_bb.id;
        let exit_id = exit_bb.id;
        let normal_exit_id = else_bb.as_ref().map(|bb| bb.id).unwrap_or(exit_id);

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Header: check idx < len
        self.push_block(header_bb);

        let cond_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cond_local,
            op: mir::BinOp::Lt,
            left: mir::Operand::Local(idx_local),
            right: mir::Operand::Local(len_local),
        });
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cond_local),
            then_block: body_id,
            else_block: normal_exit_id,
        };

        // Body: get element, set counter, execute body
        self.push_block(body_bb);

        let get_func = match iterable_kind {
            IterableKind::List | IterableKind::Dict | IterableKind::Set => {
                mir::RuntimeFunc::ListGet
            }
            IterableKind::Tuple => mir::RuntimeFunc::TupleGet,
            IterableKind::Str => mir::RuntimeFunc::StrGetChar,
            IterableKind::Bytes => mir::RuntimeFunc::BytesGet,
            IterableKind::Iterator | IterableKind::File => unreachable!(),
        };

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: elem_local,
            func: get_func,
            args: vec![
                mir::Operand::Local(actual_iter_local),
                mir::Operand::Local(idx_local),
            ],
        });

        self.push_loop(increment_id, exit_id);

        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        self.pop_loop();

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(increment_id);
        }

        // Increment: idx += 1, counter += 1
        self.push_block(increment_bb);

        let inc_idx = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: inc_idx,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(idx_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: idx_local,
            src: mir::Operand::Local(inc_idx),
        });

        let inc_counter = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: inc_counter,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(counter_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: counter_local,
            src: mir::Operand::Local(inc_counter),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Else block
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit
        self.push_block(exit_bb);

        Ok(())
    }

    /// Enumerate over an iterator/generator: for i, v in enumerate(gen, start)
    /// Uses iterator protocol with rt_iter_enumerate runtime
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_enumerate_iterator(
        &mut self,
        counter_var: VarId,
        elem_var: VarId,
        start_operand: mir::Operand,
        iter_expr: &hir::Expr,
        elem_type: Type,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Create iterator from the expression
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let iter_type = self.get_expr_type(iter_expr, hir_module);

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        // Create enumerate iterator wrapping the inner iterator
        let enum_iter_local = self.alloc_and_add_local(
            Type::Iterator(Box::new(Type::Tuple(vec![Type::Int, elem_type.clone()]))),
            mir_func,
        );
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: enum_iter_local,
            func: mir::RuntimeFunc::IterEnumerate,
            args: vec![mir::Operand::Local(iter_local), start_operand],
        });

        // Set up target variables
        self.insert_var_type(counter_var, Type::Int);
        let counter_local = self.get_or_create_local(counter_var, Type::Int, mir_func);
        self.insert_var_type(elem_var, elem_type.clone());
        let elem_local = self.get_or_create_local(elem_var, elem_type.clone(), mir_func);

        // Create blocks
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

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Header: call next(), check exhausted
        self.push_block(header_bb);

        // next() returns a tuple (counter, elem) as a pointer
        let next_local =
            self.alloc_and_add_local(Type::Tuple(vec![Type::Int, elem_type.clone()]), mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: next_local,
            func: mir::RuntimeFunc::IterNextNoExc,
            args: vec![mir::Operand::Local(enum_iter_local)],
        });

        let exhausted_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: exhausted_local,
            func: mir::RuntimeFunc::GeneratorIsExhausted,
            args: vec![mir::Operand::Local(enum_iter_local)],
        });

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: normal_exit_id,
            else_block: body_id,
        };

        // Body: unpack tuple into counter and elem variables
        self.push_block(body_bb);

        // Unpack counter (index 0) - unbox int from tuple element
        let boxed_counter = self.alloc_and_add_local(Type::Any, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: boxed_counter,
            func: mir::RuntimeFunc::TupleGet,
            args: vec![
                mir::Operand::Local(next_local),
                mir::Operand::Constant(mir::Constant::Int(0)),
            ],
        });
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: counter_local,
            func: mir::RuntimeFunc::UnboxInt,
            args: vec![mir::Operand::Local(boxed_counter)],
        });

        // Unpack elem (index 1)
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: elem_local,
            func: mir::RuntimeFunc::TupleGet,
            args: vec![
                mir::Operand::Local(next_local),
                mir::Operand::Constant(mir::Constant::Int(1)),
            ],
        });

        self.push_loop(header_id, exit_id);

        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        self.pop_loop();

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(header_id);
        }

        // Else block (optional): executes on normal loop completion
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit
        self.push_block(exit_bb);

        Ok(())
    }
}
