//! Iterable loop lowering: for x in list/tuple/dict/str/set/bytes

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;
use crate::utils::IterableKind;

impl<'a> Lowering<'a> {
    /// Lower a for-loop iterating over a list, tuple, dict, or string.
    /// Desugars `for x in items: body` to indexed iteration:
    /// ```python
    /// __iter = items
    /// __len = len(__iter)
    /// __idx = 0
    /// while __idx < __len:
    ///     x = __iter[__idx]
    ///     body
    ///     __idx = __idx + 1
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_iterable(
        &mut self,
        target: VarId,
        iter_expr: &hir::Expr,
        iterable_kind: IterableKind,
        elem_type: Type,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // For generators/iterators, use iterator protocol instead of indexed access
        if iterable_kind == IterableKind::Iterator {
            return self.lower_for_iterator(
                target, iter_expr, elem_type, body, else_block, hir_module, mir_func,
            );
        }

        // For files, call readlines() first, then iterate the resulting list
        if iterable_kind == IterableKind::File {
            let file_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
            let file_local = self.alloc_and_add_local(Type::File, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: file_local,
                src: file_operand,
            });

            // Call FileReadlines to get list[str]
            let lines_local = self.alloc_and_add_local(Type::List(Box::new(Type::Str)), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: lines_local,
                func: mir::RuntimeFunc::FileReadlines,
                args: vec![mir::Operand::Local(file_local)],
            });

            // Create a synthetic expression-like wrapper so we can reuse list iteration.
            // We directly emit the indexed loop over lines_local here instead of recursing,
            // since we already have the lowered operand.
            let iter_local = lines_local;
            let len_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: len_local,
                func: mir::RuntimeFunc::ListLen,
                args: vec![mir::Operand::Local(iter_local)],
            });

            let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: idx_local,
                src: mir::Operand::Constant(mir::Constant::Int(0)),
            });

            let header_bb = self.new_block();
            let body_bb = self.new_block();
            let increment_bb = self.new_block();
            let exit_bb = self.new_block();
            let has_else = !else_block.is_empty();
            let else_bb_opt = if has_else {
                Some(self.new_block())
            } else {
                None
            };

            let header_id = header_bb.id;
            let body_id = body_bb.id;
            let increment_id = increment_bb.id;
            let exit_id = exit_bb.id;
            let else_id = else_bb_opt.as_ref().map(|b| b.id);
            let normal_exit_id = else_id.unwrap_or(exit_id);

            self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

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

            self.push_block(body_bb);
            self.insert_var_type(target, Type::Str);
            let target_local = self.get_or_create_local(target, Type::Str, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: target_local,
                func: mir::RuntimeFunc::ListGet,
                args: vec![
                    mir::Operand::Local(iter_local),
                    mir::Operand::Local(idx_local),
                ],
            });

            if self.is_global(&target) {
                let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
                let runtime_func = self.get_global_set_func(&Type::Str);
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

            self.push_loop(increment_id, exit_id);
            for stmt_id in body {
                let stmt = &hir_module.stmts[*stmt_id];
                self.lower_stmt(stmt, hir_module, mir_func)?;
            }
            self.pop_loop();

            if !self.current_block_has_terminator() {
                self.current_block_mut().terminator = mir::Terminator::Goto(increment_id);
            }

            self.push_block(increment_bb);
            let inc_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: inc_local,
                op: mir::BinOp::Add,
                left: mir::Operand::Local(idx_local),
                right: mir::Operand::Constant(mir::Constant::Int(1)),
            });
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: idx_local,
                src: mir::Operand::Local(inc_local),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

            if let Some(else_bb) = else_bb_opt {
                self.push_block(else_bb);
                self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
            }

            self.push_block(exit_bb);
            return Ok(());
        }

        // 1. Lower the iterator expression and store in a temp local
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let iter_type = self.get_expr_type(iter_expr, hir_module);

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        // 2. Get the length of the iterable
        let len_local = self.alloc_and_add_local(Type::Int, mir_func);

        // For dict and set iteration, we need to convert to list first, then iterate by index
        let (actual_iter_local, converted_list_local) = if iterable_kind == IterableKind::Dict {
            // Get dict keys as a list
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
            // Get length from the keys list
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: len_local,
                func: mir::RuntimeFunc::ListLen,
                args: vec![mir::Operand::Local(keys_local)],
            });
            (keys_local, Some(keys_local))
        } else if iterable_kind == IterableKind::Set {
            // Convert set to list for iteration
            let list_local =
                self.alloc_and_add_local(Type::List(Box::new(elem_type.clone())), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: list_local,
                func: mir::RuntimeFunc::SetToList,
                args: vec![mir::Operand::Local(iter_local)],
            });
            // Get length from the converted list
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: len_local,
                func: mir::RuntimeFunc::ListLen,
                args: vec![mir::Operand::Local(list_local)],
            });
            (list_local, Some(list_local))
        } else {
            // Get length directly from the iterable
            let len_func = match iterable_kind {
                IterableKind::List => mir::RuntimeFunc::ListLen,
                IterableKind::Tuple => mir::RuntimeFunc::TupleLen,
                IterableKind::Str => mir::RuntimeFunc::StrLenInt,
                IterableKind::Bytes => mir::RuntimeFunc::BytesLen,
                IterableKind::Dict
                | IterableKind::Set
                | IterableKind::Iterator
                | IterableKind::File => unreachable!(),
            };
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: len_local,
                func: len_func,
                args: vec![mir::Operand::Local(iter_local)],
            });
            (iter_local, None)
        };

        // 3. Initialize index to 0
        let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: idx_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        // 4. Create blocks for loop structure (same pattern as range())
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
        let exit_id = exit_bb.id;

        let else_id = else_bb.as_ref().map(|b| b.id);
        let normal_exit_id = else_id.unwrap_or(exit_id);

        // Jump to header
        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // 5. Header block: check idx < len
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

        // 6. Body block: extract element and execute body statements
        self.push_block(body_bb);

        // Track variable type for the target
        self.insert_var_type(target, elem_type.clone());
        let target_local = self.get_or_create_local(target, elem_type.clone(), mir_func);

        // Get element at current index
        // Use specialized get functions for primitive types to avoid type mismatches
        // Note: Lists store Int as raw i64 but Bool/Float are boxed (see collections.rs)
        let get_func = match iterable_kind {
            IterableKind::List => {
                // Use specialized list get for Int (raw i64), otherwise generic
                // Bool and Float are boxed in lists, so use ListGet
                match &elem_type {
                    Type::Int => mir::RuntimeFunc::ListGetTyped(mir::GetElementKind::Int),
                    _ => mir::RuntimeFunc::ListGet, // Bool, Float, etc. are boxed
                }
            }
            IterableKind::Dict | IterableKind::Set => {
                // After DictKeys/SetToList conversion, the result list's elem_tag
                // depends on the element type: ELEM_RAW_INT for Int, ELEM_HEAP_OBJ
                // for everything else. Use specialized get functions accordingly.
                match &elem_type {
                    Type::Int => mir::RuntimeFunc::ListGetTyped(mir::GetElementKind::Int),
                    Type::Float => mir::RuntimeFunc::ListGetTyped(mir::GetElementKind::Float),
                    Type::Bool => mir::RuntimeFunc::ListGetTyped(mir::GetElementKind::Bool),
                    _ => mir::RuntimeFunc::ListGet,
                }
            }
            IterableKind::Tuple => Self::tuple_get_func(&elem_type),
            IterableKind::Str => mir::RuntimeFunc::StrGetChar,
            IterableKind::Bytes => mir::RuntimeFunc::BytesGet,
            IterableKind::Iterator | IterableKind::File => {
                unreachable!("Iterator and File handled separately")
            }
        };

        // Use converted list for dict/set, otherwise use the original iter
        let iter_to_index = if converted_list_local.is_some() {
            actual_iter_local
        } else {
            iter_local
        };

        // Specialized get functions (ListGetTyped) handle
        // both ELEM_RAW_INT and ELEM_HEAP_OBJ transparently, so no manual unboxing needed.
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: target_local,
            func: get_func,
            args: vec![
                mir::Operand::Local(iter_to_index),
                mir::Operand::Local(idx_local),
            ],
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

        // Push loop context for break/continue: continue jumps to increment, break jumps to exit
        self.push_loop(increment_id, exit_id);

        // Execute body statements
        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        // Pop loop context
        self.pop_loop();

        // If no terminator (no break/return), fall through to increment
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(increment_id);
        }

        // 7. Increment block: idx = idx + 1, then jump to header
        self.push_block(increment_bb);

        let inc_local = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: inc_local,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(idx_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });

        self.emit_instruction(mir::InstructionKind::Copy {
            dest: idx_local,
            src: mir::Operand::Local(inc_local),
        });

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // 8. Else block: execute if loop completed normally (not via break)
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // 9. Exit block: continue after loop
        self.push_block(exit_bb);

        Ok(())
    }
}
