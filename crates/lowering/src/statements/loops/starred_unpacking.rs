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
        iter_id: hir::ExprId,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_expr = &hir_module.exprs[iter_id];
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let hir_iter_type = self.get_type_of_expr_id(iter_id, hir_module);
        let lowered_iter_type = self.operand_type(&iter_operand, mir_func);
        let iter_type = if matches!(hir_iter_type, Type::Any) || hir_iter_type.is_union() {
            lowered_iter_type
        } else {
            hir_iter_type
        };

        let Some((kind, elem_type)) = get_iterable_info(&iter_type) else {
            // Fallback for unknown types: use iterator protocol
            return self.lower_for_unpack_starred_iterator(
                before_star,
                starred,
                after_star,
                iter_id,
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
                iter_id,
                elem_type,
                body,
                else_block,
                hir_module,
                mir_func,
            );
        }

        // Determine the types of unpacked elements from the tuple/list element type.
        //
        // Layout: [before_star... | starred (List<inner>) | after_star...]
        //
        // `lower_starred_unpack_from_value` indexes into target_types as:
        //   - before_star[i]  → target_types[i]
        //   - after_star[i]   → target_types[before_star.len() + starred.is_some() as usize + i]
        //
        // So when `starred.is_some()`, position `before_star.len()` must hold the starred
        // variable's type so that after_star indices are computed correctly.
        let target_types: Vec<Type> = match &elem_type {
            Type::Tuple(types) => types.clone(),
            Type::List(inner) => {
                // For list elements, all extracted elements share the same inner type.
                // The starred variable itself collects a sub-list, so its slot holds List(inner).
                let mut types = vec![(**inner).clone(); before_star.len() + after_star.len()];
                if starred.is_some() {
                    types.insert(before_star.len(), Type::List(inner.clone()));
                }
                types
            }
            _ => {
                let mut types = vec![Type::Any; before_star.len() + after_star.len()];
                if starred.is_some() {
                    types.insert(before_star.len(), Type::List(Box::new(Type::Any)));
                }
                types
            }
        };

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        let len_func = match kind {
            IterableKind::List => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN)
            }
            IterableKind::Tuple => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_LEN)
            }
            IterableKind::Str => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STR_LEN_INT)
            }
            IterableKind::Bytes => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BYTES_LEN)
            }
            IterableKind::Dict
            | IterableKind::Set
            | IterableKind::Iterator
            | IterableKind::File => unreachable!("handled by lower_for_unpack_iterator"),
        };
        // Get length
        let len_local = self.emit_runtime_call(
            len_func,
            vec![mir::Operand::Local(iter_local)],
            Type::Int,
            mir_func,
        );

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
            IterableKind::List => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
            }
            IterableKind::Tuple => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET)
            }
            IterableKind::Str => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STR_GETCHAR)
            }
            IterableKind::Bytes => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BYTES_GET)
            }
            IterableKind::Dict
            | IterableKind::Set
            | IterableKind::Iterator
            | IterableKind::File => unreachable!("handled by lower_for_unpack_iterator"),
        };

        // Get the tuple/list element at current index
        let elem_local = self.emit_runtime_call(
            get_func,
            vec![
                mir::Operand::Local(iter_local),
                mir::Operand::Local(idx_local),
            ],
            elem_type.clone(),
            mir_func,
        );

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
        iter_id: hir::ExprId,
        elem_type: Type,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_expr = &hir_module.exprs[iter_id];
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let iter_type = self.get_type_of_expr_id(iter_id, hir_module);

        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        // Determine element types.
        //
        // Layout mirrors lower_for_unpack_starred: [before_star... | starred (List<inner>) | after_star...]
        // The starred slot must be present when `starred.is_some()` so that after_star index
        // arithmetic in lower_starred_unpack_from_value resolves correctly.
        let target_types: Vec<Type> = match &elem_type {
            Type::Tuple(types) => types.clone(),
            Type::List(inner) => {
                let mut types = vec![(**inner).clone(); before_star.len() + after_star.len()];
                if starred.is_some() {
                    types.insert(before_star.len(), Type::List(inner.clone()));
                }
                types
            }
            _ => {
                let mut types = vec![Type::Any; before_star.len() + after_star.len()];
                if starred.is_some() {
                    types.insert(before_star.len(), Type::List(Box::new(Type::Any)));
                }
                types
            }
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

        let next_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            elem_type.clone(),
            mir_func,
        );

        let exhausted_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );

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
                crate::type_dispatch::tuple_get_func(&target_ty)
            } else {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
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
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_SLICE_TO_LIST,
                    ),
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
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_SLICE),
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
                crate::type_dispatch::tuple_get_func(&target_ty)
            } else {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
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
