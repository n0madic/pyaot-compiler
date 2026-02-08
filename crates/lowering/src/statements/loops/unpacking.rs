//! Tuple unpacking loop lowering: for a, b in list_of_tuples

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;
use crate::utils::{get_iterable_info, IterableKind};

impl<'a> Lowering<'a> {
    /// General tuple unpacking for loop: for a, b in list_of_tuples
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_unpack_general(
        &mut self,
        targets: &[VarId],
        iter_expr: &hir::Expr,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Special case: zip() builtin - compute element types directly from arguments
        // to handle cases like zip(range(3), ...) where range has no explicit type
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Zip,
            args: zip_args,
            ..
        } = &iter_expr.kind
        {
            let elem_types = self.compute_zip_element_types(zip_args, hir_module);
            return self.lower_for_unpack_iterator(
                targets,
                iter_expr,
                Type::Tuple(elem_types),
                body,
                else_block,
                hir_module,
                mir_func,
            );
        }

        let iter_type = self.get_expr_type(iter_expr, hir_module);

        let Some((kind, elem_type)) = get_iterable_info(&iter_type) else {
            // Fallback for unknown types: use iterator protocol
            return self.lower_for_unpack_iterator(
                targets,
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
            return self.lower_for_unpack_iterator(
                targets, iter_expr, elem_type, body, else_block, hir_module, mir_func,
            );
        }

        // Determine the types of unpacked elements from the tuple element type
        let target_types: Vec<Type> = match &elem_type {
            Type::Tuple(types) => types.clone(),
            _ => vec![Type::Any; targets.len()],
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

        // Create target locals
        let mut target_locals = Vec::new();
        for (i, &target) in targets.iter().enumerate() {
            let ty = target_types.get(i).cloned().unwrap_or(Type::Any);
            self.insert_var_type(target, ty.clone());
            let local = self.get_or_create_local(target, ty, mir_func);
            target_locals.push(local);
        }

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

        // Body: get tuple element, unpack fields
        self.push_block(body_bb);

        let get_func = match kind {
            IterableKind::List => mir::RuntimeFunc::ListGet,
            IterableKind::Tuple => mir::RuntimeFunc::TupleGet,
            _ => mir::RuntimeFunc::ListGet,
        };

        // Get the tuple element at current index
        let tuple_elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: tuple_elem_local,
            func: get_func,
            args: vec![
                mir::Operand::Local(iter_local),
                mir::Operand::Local(idx_local),
            ],
        });

        // Unpack tuple fields into target locals
        for (i, &target_local) in target_locals.iter().enumerate() {
            let target_ty = target_types.get(i).cloned().unwrap_or(Type::Any);

            // Use typed TupleGet for primitive types to handle unboxing
            let func = match &target_ty {
                Type::Int => mir::RuntimeFunc::TupleGetInt,
                Type::Float => mir::RuntimeFunc::TupleGetFloat,
                Type::Bool => mir::RuntimeFunc::TupleGetBool,
                _ => mir::RuntimeFunc::TupleGet, // Heap types: str, list, etc.
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: target_local,
                func,
                args: vec![
                    mir::Operand::Local(tuple_elem_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
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

        // Handle else block
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit
        self.push_block(exit_bb);

        Ok(())
    }

    /// General tuple unpacking for iterator/generator: for a, b in gen
    /// Uses iterator protocol with tuple unpacking at each step
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_for_unpack_iterator(
        &mut self,
        targets: &[VarId],
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
            _ => vec![Type::Any; targets.len()],
        };

        // Create target locals
        let mut target_locals = Vec::new();
        for (i, &target) in targets.iter().enumerate() {
            let ty = target_types.get(i).cloned().unwrap_or(Type::Any);
            self.insert_var_type(target, ty.clone());
            let local = self.get_or_create_local(target, ty, mir_func);
            target_locals.push(local);
        }

        // Create blocks
        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let exit_bb = self.new_block();
        let else_bb = if !else_block.is_empty() {
            Some(self.new_block())
        } else {
            None
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

        // Body: unpack tuple elements
        self.push_block(body_bb);

        for (i, &target_local) in target_locals.iter().enumerate() {
            let target_ty = target_types.get(i).cloned().unwrap_or(Type::Any);

            // Use typed TupleGet for primitive types to handle unboxing
            let func = match &target_ty {
                Type::Int => mir::RuntimeFunc::TupleGetInt,
                Type::Float => mir::RuntimeFunc::TupleGetFloat,
                Type::Bool => mir::RuntimeFunc::TupleGetBool,
                _ => mir::RuntimeFunc::TupleGet, // Heap types: str, list, etc.
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: target_local,
                func,
                args: vec![
                    mir::Operand::Local(next_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                ],
            });
        }

        self.push_loop(header_id, exit_id);

        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        self.pop_loop();

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(header_id);
        }

        // Handle else block
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }

        // Exit
        self.push_block(exit_bb);

        Ok(())
    }

    /// Compute element types for zip() arguments, handling special cases like range()
    fn compute_zip_element_types(
        &self,
        zip_args: &[hir::ExprId],
        hir_module: &hir::Module,
    ) -> Vec<Type> {
        let mut elem_types = Vec::new();
        for arg_id in zip_args {
            let arg_expr = &hir_module.exprs[*arg_id];
            // Special case: range() returns Int elements
            if let hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                ..
            } = &arg_expr.kind
            {
                elem_types.push(Type::Int);
                continue;
            }
            let arg_type = self.get_expr_type(arg_expr, hir_module);
            let elem_type = match &arg_type {
                Type::List(elem) => (**elem).clone(),
                Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
                Type::Str => Type::Str,
                Type::Dict(key, _) => (**key).clone(),
                Type::Set(elem) => (**elem).clone(),
                Type::Bytes => Type::Int,
                Type::Iterator(elem) => (**elem).clone(),
                _ => Type::Any,
            };
            elem_types.push(elem_type);
        }
        elem_types
    }
}
