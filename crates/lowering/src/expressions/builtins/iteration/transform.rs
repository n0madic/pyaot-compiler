//! Transformation iteration lowering: reversed(), sorted()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower reversed(x) - create a reverse iterator from a sequence
    pub(in crate::expressions::builtins) fn lower_reversed(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // reversed() with no args is invalid
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
            return self.lower_reversed_range(range_args, hir_module, mir_func);
        }

        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        // Determine element type from container type
        let elem_type = crate::type_planning::infer::extract_iterable_first_element_type(&arg_type);

        // Create result local with Iterator type
        let result_local = self.alloc_and_add_local(Type::Iterator(Box::new(elem_type)), mir_func);

        // Select appropriate iterator source kind based on container type
        let source = match &arg_type {
            Type::List(_) => mir::IterSourceKind::List,
            Type::Tuple(_) => mir::IterSourceKind::Tuple,
            Type::Dict(_, _) => mir::IterSourceKind::Dict,
            Type::Str => mir::IterSourceKind::Str,
            Type::Bytes => mir::IterSourceKind::Bytes,
            _ => {
                // Unknown sequence type - fallback to list iterator
                mir::IterSourceKind::List
            }
        };

        let iter_func = mir::RuntimeFunc::Call(source.iterator_def(mir::IterDirection::Reversed));

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: iter_func,
            args: vec![arg_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower reversed(range(start, stop, step)) - create a reversed range iterator
    pub(in crate::expressions::builtins) fn lower_reversed_range(
        &mut self,
        range_args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Parse range arguments: range(stop), range(start, stop), range(start, stop, step)
        let (start_operand, stop_operand, step_operand) = match range_args.len() {
            1 => {
                // range(stop) -> start=0, step=1
                let stop_expr = &hir_module.exprs[range_args[0]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    stop,
                    mir::Operand::Constant(mir::Constant::Int(1)),
                )
            }
            2 => {
                // range(start, stop) -> step=1
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (start, stop, mir::Operand::Constant(mir::Constant::Int(1)))
            }
            3 => {
                // range(start, stop, step)
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let step_expr = &hir_module.exprs[range_args[2]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                let step = self.lower_expr(step_expr, hir_module, mir_func)?;
                (start, stop, step)
            }
            _ => {
                // Invalid range call - return None
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        };

        // Create result local with Iterator[int] type
        let result_local = self.alloc_and_add_local(Type::Iterator(Box::new(Type::Int)), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(
                mir::IterSourceKind::Range.iterator_def(mir::IterDirection::Reversed),
            ),
            args: vec![start_operand, stop_operand, step_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower sorted(iterable, key=None, reverse=False) - return new sorted list from iterable
    pub(in crate::expressions::builtins) fn lower_sorted(
        &mut self,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // sorted() with no args is invalid
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        // Use shared helper to extract sort kwargs
        let sort_kwargs = self.extract_sort_kwargs(kwargs, hir_module, mir_func)?;

        let arg_expr = &hir_module.exprs[args[0]];

        // Check if the argument is a range() call - needs special handling
        // Note: range with key is not supported, fall through to normal sorted
        if sort_kwargs.key_func.is_none() {
            if let hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                args: range_args,
                ..
            } = &arg_expr.kind
            {
                return self.lower_sorted_range(
                    range_args,
                    sort_kwargs.reverse,
                    hir_module,
                    mir_func,
                );
            }
        }

        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        // Determine element type from container type
        let elem_type = crate::type_planning::infer::extract_iterable_first_element_type(&arg_type);

        // Determine elem_tag for boxing raw elements before calling key function.
        // Only builtin wrappers need boxing - user functions work with raw values.
        // Compute before elem_type is moved.
        let elem_tag = sort_kwargs
            .key_func
            .as_ref()
            .map(|kf| Self::elem_tag_for_key_func(kf, &elem_type))
            .unwrap_or(0);

        // Compute elem_tag for Set/Dict sorted result before elem_type is moved
        let result_elem_tag = Self::elem_tag_for_type(&elem_type);

        // Create result local with List type (sorted always returns a list)
        let result_local = self.alloc_and_add_local(Type::List(Box::new(elem_type)), mir_func);

        // Select appropriate sortable kind based on container type
        let source = match &arg_type {
            Type::List(_) => mir::SortableKind::List,
            Type::Tuple(_) => mir::SortableKind::Tuple,
            Type::Dict(_, _) => mir::SortableKind::Dict,
            Type::Str => mir::SortableKind::Str,
            Type::Set(_) => mir::SortableKind::Set,
            _ => mir::SortableKind::List,
        };

        // If key function is provided, use the with_key variant
        if let Some(resolved) =
            self.emit_key_func_with_captures(sort_kwargs.key_func.as_ref(), hir_module, mir_func)?
        {
            let elem_tag_operand = mir::Operand::Constant(mir::Constant::Int(elem_tag));

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(source.sorted_def(true)),
                args: vec![
                    arg_operand,
                    sort_kwargs.reverse,
                    resolved.func_addr,
                    elem_tag_operand,
                    resolved.captures,
                    resolved.capture_count,
                ],
            });
        } else {
            // No key function - use standard sorted
            // Set/Dict sorted need elem_tag to produce correctly-typed result lists
            let args = if matches!(source, mir::SortableKind::Set | mir::SortableKind::Dict) {
                vec![
                    arg_operand,
                    sort_kwargs.reverse,
                    mir::Operand::Constant(mir::Constant::Int(result_elem_tag)),
                ]
            } else {
                vec![arg_operand, sort_kwargs.reverse]
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(source.sorted_def(false)),
                args,
            });
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower sorted(range(start, stop, step), reverse=False) - create sorted list from range
    pub(in crate::expressions::builtins) fn lower_sorted_range(
        &mut self,
        range_args: &[hir::ExprId],
        reverse_operand: mir::Operand,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Parse range arguments: range(stop), range(start, stop), range(start, stop, step)
        let (start_operand, stop_operand, step_operand) = match range_args.len() {
            1 => {
                // range(stop) -> start=0, step=1
                let stop_expr = &hir_module.exprs[range_args[0]];
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    stop,
                    mir::Operand::Constant(mir::Constant::Int(1)),
                )
            }
            2 => {
                // range(start, stop) -> step=1
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                (start, stop, mir::Operand::Constant(mir::Constant::Int(1)))
            }
            3 => {
                // range(start, stop, step)
                let start_expr = &hir_module.exprs[range_args[0]];
                let stop_expr = &hir_module.exprs[range_args[1]];
                let step_expr = &hir_module.exprs[range_args[2]];
                let start = self.lower_expr(start_expr, hir_module, mir_func)?;
                let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
                let step = self.lower_expr(step_expr, hir_module, mir_func)?;
                (start, stop, step)
            }
            _ => {
                // Invalid range call - return None
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        };

        // Create result local with List[int] type
        let result_local = self.alloc_and_add_local(Type::List(Box::new(Type::Int)), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(mir::SortableKind::Range.sorted_def(false)),
            args: vec![start_operand, stop_operand, step_operand, reverse_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
