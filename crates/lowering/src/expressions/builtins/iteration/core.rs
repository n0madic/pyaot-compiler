//! Core iteration lowering: iter(), next(), range iterator creation

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower iter(x) - create an iterator from an iterable
    pub(in crate::expressions::builtins) fn lower_iter(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // iter() with no args is invalid
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let arg_expr = &hir_module.exprs[args[0]];

        // Check if the argument is a range() call - needs special handling
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            args: range_args,
            ..
        } = &arg_expr.kind
        {
            return self.lower_iter_range(range_args, hir_module, mir_func);
        }

        let arg_operand = self.lower_expr_expecting(arg_expr, None, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        // Handle class with __iter__ dunder
        if let Type::Class { class_id, .. } = &arg_type {
            let iter_func = self
                .get_class_info(class_id)
                .and_then(|info| info.get_dunder_func("__iter__"));
            if let Some(func_id) = iter_func {
                let result_local = self.alloc_and_add_local(arg_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: func_id,
                    args: vec![arg_operand],
                });
                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Determine element type from container type
        let elem_type = crate::type_planning::infer::extract_iterable_first_element_type(&arg_type);

        // Select appropriate iterator source kind based on container type
        let source = crate::type_dispatch::type_to_iter_source(&arg_type);

        let iter_func = mir::RuntimeFunc::Call(source.iterator_def(mir::IterDirection::Forward));

        // Create result local with Iterator type
        let result_local = self.emit_runtime_call(
            iter_func,
            vec![arg_operand],
            Type::Iterator(Box::new(elem_type)),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Parse range arguments: range(stop), range(start, stop), range(start, stop, step)
    /// Returns (start, stop, step) operands
    pub(in crate::expressions::builtins) fn parse_range_args(
        &mut self,
        range_args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<(mir::Operand, mir::Operand, mir::Operand)> {
        match range_args.len() {
            1 => {
                // range(stop) -> start=0, step=1
                let stop_expr = &hir_module.exprs[range_args[0]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                Ok((
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    stop,
                    mir::Operand::Constant(mir::Constant::Int(1)),
                ))
            }
            2 => {
                // range(start, stop) -> step=1
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                Ok((start, stop, mir::Operand::Constant(mir::Constant::Int(1))))
            }
            3 => {
                // range(start, stop, step)
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let step_expr = &hir_module.exprs[range_args[2]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                let step = self.lower_expr(step_expr, hir_module, mir_func)?;
                Ok((start, stop, step))
            }
            _ => {
                // Invalid range call - return default values
                Ok((
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Constant(mir::Constant::Int(1)),
                ))
            }
        }
    }

    /// Lower iter(range(start, stop, step)) - create a range iterator
    pub(in crate::expressions::builtins) fn lower_iter_range(
        &mut self,
        range_args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let (start_operand, stop_operand, step_operand) =
            self.parse_range_args(range_args, hir_module, mir_func)?;

        // Create result local with Iterator[int] type
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(
                mir::IterSourceKind::Range.iterator_def(mir::IterDirection::Forward),
            ),
            vec![start_operand, stop_operand, step_operand],
            Type::Iterator(Box::new(Type::Int)),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower next(iter) - get next element from iterator
    /// Raises StopIteration when the iterator is exhausted
    pub(in crate::expressions::builtins) fn lower_next(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // next() with no args is invalid
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr_expecting(arg_expr, None, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        // Handle class with __next__ dunder
        if let Type::Class { class_id, .. } = &arg_type {
            let next_func = self
                .get_class_info(class_id)
                .and_then(|info| info.get_dunder_func("__next__"));
            if let Some(func_id) = next_func {
                let return_ty = self
                    .get_func_return_type(&func_id)
                    .cloned()
                    .unwrap_or(Type::Any);
                let result_local = self.alloc_and_add_local(return_ty, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: func_id,
                    args: vec![arg_operand],
                });
                return Ok(mir::Operand::Local(result_local));
            }
        }

        // After §F.7c BigBang: list/dict/tuple/set iterators return tagged
        // Value bits. Bind to HeapAny first, then unwrap Int/Bool for typed callers.
        let elem_type = match &arg_type {
            Type::Iterator(elem) => (**elem).clone(),
            _ => Type::Any,
        };

        let raw_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT),
            vec![arg_operand],
            Type::HeapAny,
            mir_func,
        );

        let result_local = match &elem_type {
            Type::Int => {
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::UnwrapValueInt {
                    dest,
                    src: mir::Operand::Local(raw_local),
                });
                dest
            }
            Type::Bool => {
                let dest = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::UnwrapValueBool {
                    dest,
                    src: mir::Operand::Local(raw_local),
                });
                dest
            }
            _ => {
                // Re-tag the local with the proper element type so downstream
                // type-aware lowering sees the right type.
                let dest = self.alloc_and_add_local(elem_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest,
                    src: mir::Operand::Local(raw_local),
                });
                dest
            }
        };

        Ok(mir::Operand::Local(result_local))
    }

    /// Helper function to create an iterator from an expression and return the iterator local and element type
    pub(in crate::expressions::builtins) fn make_iter_from_expr(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<(pyaot_utils::LocalId, Type)> {
        let expr = &hir_module.exprs[expr_id];

        // Check if the argument is a range() call - needs special handling
        let is_range = matches!(
            &expr.kind,
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                ..
            }
        );

        let expr_type = self.seed_expr_type(expr_id, hir_module);
        let elem_type = if is_range {
            Type::Int
        } else {
            crate::type_planning::infer::extract_iterable_first_element_type(&expr_type)
        };

        // Create iterator
        let iter_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(elem_type.clone())), mir_func);

        // Handle range() specially
        if is_range {
            if let hir::ExprKind::BuiltinCall {
                args: range_args, ..
            } = &expr.kind
            {
                let (start, stop, step) =
                    self.parse_range_args(range_args, hir_module, mir_func)?;
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: iter_local,
                    func: mir::RuntimeFunc::Call(
                        mir::IterSourceKind::Range.iterator_def(mir::IterDirection::Forward),
                    ),
                    args: vec![start, stop, step],
                });
            }
        } else {
            let operand = self.lower_expr_expecting(expr, None, hir_module, mir_func)?;
            let source = crate::type_dispatch::type_to_iter_source(&expr_type);

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: iter_local,
                func: mir::RuntimeFunc::Call(source.iterator_def(mir::IterDirection::Forward)),
                args: vec![operand],
            });
        }

        Ok((iter_local, elem_type))
    }
}
