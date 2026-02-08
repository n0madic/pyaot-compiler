//! Starred unpacking loop lowering: for first, *rest, last in items

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;
use crate::utils::{get_iterable_info, IterableKind};

impl<'a> Lowering<'a> {
    /// Starred unpacking for loop: for first, *rest, last in items
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_unpack_starred(
        &mut self,
        before_star: &[VarId],
        starred: Option<VarId>,
        after_star: &[VarId],
        iter_expr: &hir::Expr,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_type = self.get_expr_type(iter_expr, hir_module);

        let Some((kind, elem_type)) = get_iterable_info(&iter_type) else {
            // Fallback for unknown types: use iterator protocol
            return self.lower_for_unpack_starred_iterator(
                before_star,
                starred,
                after_star,
                iter_expr,
                Type::Any,
                body,
                else_block,
                hir_module,
                mir_func,
            );
        };

        // For iterators, use iterator protocol
        if kind == IterableKind::Iterator {
            return self.lower_for_unpack_starred_iterator(
                before_star,
                starred,
                after_star,
                iter_expr,
                elem_type,
                body,
                else_block,
                hir_module,
                mir_func,
            );
        }

        // Determine the types of unpacked elements from the tuple/list element type
        let target_types: Vec<Type> = match &elem_type {
            Type::Tuple(types) => types.clone(),
            Type::List(inner) => {
                // For list elements, all have the same type
                vec![(**inner).clone(); before_star.len() + after_star.len()]
            }
            _ => vec![Type::Any; before_star.len() + after_star.len()],
        };

        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        // Get length
        let len_local = self.alloc_and_add_local(Type::Int, mir_func);

        let len_func = match kind {
            IterableKind::List => mir::RuntimeFunc::ListLen,
            IterableKind::Tuple => mir::RuntimeFunc::TupleLen,
            _ => mir::RuntimeFunc::ListLen,
        };
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: len_local,
            func: len_func,
            args: vec![mir::Operand::Local(iter_local)],
        });

        // Initialize index
        let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: idx_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        // Create blocks
        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let increment_bb = self.new_block();
        let exit_bb = self.new_block();
        let else_bb = if else_block.is_empty() {
            None
        } else {
            Some(self.new_block())
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

        // Body: get tuple/list element, unpack with starred support
        self.push_block(body_bb);

        let get_func = match kind {
            IterableKind::List => mir::RuntimeFunc::ListGet,
            IterableKind::Tuple => mir::RuntimeFunc::TupleGet,
            _ => mir::RuntimeFunc::ListGet,
        };

        // Get the tuple/list element at current index
        let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: elem_local,
            func: get_func,
            args: vec![
                mir::Operand::Local(iter_local),
                mir::Operand::Local(idx_local),
            ],
        });

        // Unpack with starred support: extract before_star, starred, after_star
        self.lower_starred_unpack_from_value(
            before_star,
            starred,
            after_star,
            mir::Operand::Local(elem_local),
            &elem_type,
            &target_types,
            hir_module,
            mir_func,
        )?;

        self.push_loop(increment_id, exit_id);

        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        self.pop_loop();

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(increment_id);
        }

        // Increment: idx += 1
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

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Else block (if present)
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit
        self.push_block(exit_bb);

        Ok(())
    }

    /// Starred unpacking for iterator/generator: for first, *rest, last in gen
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_unpack_starred_iterator(
        &mut self,
        before_star: &[VarId],
        starred: Option<VarId>,
        after_star: &[VarId],
        iter_expr: &hir::Expr,
        elem_type: Type,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let iter_type = self.get_expr_type(iter_expr, hir_module);

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        // Determine element types
        let target_types: Vec<Type> = match &elem_type {
            Type::Tuple(types) => types.clone(),
            Type::List(inner) => {
                vec![(**inner).clone(); before_star.len() + after_star.len()]
            }
            _ => vec![Type::Any; before_star.len() + after_star.len()],
        };

        // Create blocks
        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let exit_bb = self.new_block();
        let else_bb = if else_block.is_empty() {
            None
        } else {
            Some(self.new_block())
        };

        let header_id = header_bb.id;
        let body_id = body_bb.id;
        let exit_id = exit_bb.id;
        let normal_exit_id = else_bb.as_ref().map(|bb| bb.id).unwrap_or(exit_id);

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Header: call next(), check exhausted
        self.push_block(header_bb);

        let next_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: next_local,
            func: mir::RuntimeFunc::IterNextNoExc,
            args: vec![mir::Operand::Local(iter_local)],
        });

        let exhausted_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: exhausted_local,
            func: mir::RuntimeFunc::GeneratorIsExhausted,
            args: vec![mir::Operand::Local(iter_local)],
        });

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: normal_exit_id,
            else_block: body_id,
        };

        // Body: unpack tuple elements with starred support
        self.push_block(body_bb);

        self.lower_starred_unpack_from_value(
            before_star,
            starred,
            after_star,
            mir::Operand::Local(next_local),
            &elem_type,
            &target_types,
            hir_module,
            mir_func,
        )?;

        self.push_loop(header_id, exit_id);

        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        self.pop_loop();

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(header_id);
        }

        // Else block (if present)
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit
        self.push_block(exit_bb);

        Ok(())
    }

    /// Helper to unpack elements from a value with starred support
    /// Extracts: before_star elements (positive indices), starred portion (slice), after_star elements (negative indices)
    #[allow(clippy::too_many_arguments)]
    fn lower_starred_unpack_from_value(
        &mut self,
        before_star: &[VarId],
        starred: Option<VarId>,
        after_star: &[VarId],
        value_operand: mir::Operand,
        value_type: &Type,
        target_types: &[Type],
        _hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let is_tuple = matches!(value_type, Type::Tuple(_));

        // Extract before_star elements with positive indices
        for (i, &target) in before_star.iter().enumerate() {
            let target_ty = target_types.get(i).cloned().unwrap_or(Type::Any);

            let func = if is_tuple {
                match &target_ty {
                    Type::Int => mir::RuntimeFunc::TupleGetInt,
                    Type::Float => mir::RuntimeFunc::TupleGetFloat,
                    Type::Bool => mir::RuntimeFunc::TupleGetBool,
                    _ => mir::RuntimeFunc::TupleGet,
                }
            } else {
                // Lists always use ListGet (no typed variants)
                mir::RuntimeFunc::ListGet
            };

            self.insert_var_type(target, target_ty.clone());
            let target_local = self.get_or_create_local(target, target_ty, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: target_local,
                func,
                args: vec![
                    value_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                ],
            });
        }

        // Extract starred portion (if any) using slicing
        if let Some(starred_var) = starred {
            let start_idx = before_star.len() as i64;
            let end_idx = if after_star.is_empty() {
                // No after_star: slice to the end
                i64::MAX
            } else {
                -(after_star.len() as i64)
            };

            // Compute the element type for the starred portion
            let starred_elem_type = match value_type {
                Type::Tuple(elem_types) => {
                    // For tuple, compute union of middle element types
                    let middle_start = before_star.len();
                    let middle_end = if elem_types.len() >= after_star.len() {
                        elem_types.len() - after_star.len()
                    } else {
                        middle_start
                    };
                    if middle_start < middle_end {
                        Type::normalize_union(elem_types[middle_start..middle_end].to_vec())
                    } else {
                        Type::Any
                    }
                }
                Type::List(inner) => (**inner).clone(),
                _ => Type::Any,
            };

            // Starred portion is always a list
            let starred_type = Type::List(Box::new(starred_elem_type));

            self.insert_var_type(starred_var, starred_type.clone());
            let starred_local = self.get_or_create_local(starred_var, starred_type, mir_func);

            if is_tuple {
                // TupleSliceToList(tuple, start, end_idx)
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: starred_local,
                    func: mir::RuntimeFunc::TupleSliceToList,
                    args: vec![
                        value_operand.clone(),
                        mir::Operand::Constant(mir::Constant::Int(start_idx)),
                        mir::Operand::Constant(mir::Constant::Int(end_idx)),
                    ],
                });
            } else {
                // ListSlice(list, start, end_idx)
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: starred_local,
                    func: mir::RuntimeFunc::ListSlice,
                    args: vec![
                        value_operand.clone(),
                        mir::Operand::Constant(mir::Constant::Int(start_idx)),
                        mir::Operand::Constant(mir::Constant::Int(end_idx)),
                    ],
                });
            }
        }

        // Extract after_star elements with negative indices
        for (i, &target) in after_star.iter().enumerate() {
            let target_idx = before_star.len() + starred.is_some() as usize + i;
            let target_ty = target_types.get(target_idx).cloned().unwrap_or(Type::Any);

            let negative_idx = -((after_star.len() - i) as i64);

            let func = if is_tuple {
                match &target_ty {
                    Type::Int => mir::RuntimeFunc::TupleGetInt,
                    Type::Float => mir::RuntimeFunc::TupleGetFloat,
                    Type::Bool => mir::RuntimeFunc::TupleGetBool,
                    _ => mir::RuntimeFunc::TupleGet,
                }
            } else {
                // Lists always use ListGet (no typed variants)
                mir::RuntimeFunc::ListGet
            };

            self.insert_var_type(target, target_ty.clone());
            let target_local = self.get_or_create_local(target, target_ty, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: target_local,
                func,
                args: vec![
                    value_operand.clone(),
                    mir::Operand::Constant(mir::Constant::Int(negative_idx)),
                ],
            });
        }

        Ok(())
    }
}
