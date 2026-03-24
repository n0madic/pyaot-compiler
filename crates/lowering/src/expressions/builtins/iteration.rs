//! Iteration functions lowering: iter(), next(), reversed(), sorted(), zip(), chain(), islice()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::{FuncOrBuiltin, Lowering};

impl<'a> Lowering<'a> {
    /// Lower iter(x) - create an iterator from an iterable
    pub(super) fn lower_iter(
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

        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        // Handle class with __iter__ dunder
        if let Type::Class { class_id, .. } = &arg_type {
            let iter_func = self
                .get_class_info(class_id)
                .and_then(|info| info.iter_func);
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
        let elem_type = match &arg_type {
            Type::List(elem) => (**elem).clone(),
            Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
            Type::Tuple(_) => Type::Any,
            Type::Dict(key, _) => (**key).clone(),
            Type::Set(elem) => (**elem).clone(),
            Type::Str => Type::Str,
            Type::Bytes => Type::Int, // bytes yields integers
            _ => Type::Any,
        };

        // Create result local with Iterator type
        let result_local = self.alloc_and_add_local(Type::Iterator(Box::new(elem_type)), mir_func);

        // Select appropriate iterator source kind based on container type
        let source = match &arg_type {
            Type::List(_) => mir::IterSourceKind::List,
            Type::Tuple(_) => mir::IterSourceKind::Tuple,
            Type::Dict(_, _) => mir::IterSourceKind::Dict,
            Type::Set(_) => mir::IterSourceKind::Set,
            Type::Str => mir::IterSourceKind::Str,
            Type::Bytes => mir::IterSourceKind::Bytes,
            Type::Iterator(_) => mir::IterSourceKind::Generator, // Generators are their own iterators
            _ => {
                // Unknown iterable type - fallback to list iterator
                mir::IterSourceKind::List
            }
        };

        let iter_func = mir::RuntimeFunc::MakeIterator {
            source,
            direction: mir::IterDirection::Forward,
        };

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: iter_func,
            args: vec![arg_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Parse range arguments: range(stop), range(start, stop), range(start, stop, step)
    /// Returns (start, stop, step) operands
    pub(super) fn parse_range_args(
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
    pub(super) fn lower_iter_range(
        &mut self,
        range_args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let (start_operand, stop_operand, step_operand) =
            self.parse_range_args(range_args, hir_module, mir_func)?;

        // Create result local with Iterator[int] type
        let result_local = self.alloc_and_add_local(Type::Iterator(Box::new(Type::Int)), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MakeIterator {
                source: mir::IterSourceKind::Range,
                direction: mir::IterDirection::Forward,
            },
            args: vec![start_operand, stop_operand, step_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower next(iter) - get next element from iterator
    /// Raises StopIteration when the iterator is exhausted
    pub(super) fn lower_next(
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
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        // Handle class with __next__ dunder
        if let Type::Class { class_id, .. } = &arg_type {
            let next_func = self
                .get_class_info(class_id)
                .and_then(|info| info.next_func);
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

        // Determine element type from iterator type
        let elem_type = match &arg_type {
            Type::Iterator(elem) => (**elem).clone(),
            _ => Type::Any, // If not an iterator type, default to Any
        };

        // Create result local with element type
        let result_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::IterNext,
            args: vec![arg_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower reversed(x) - create a reverse iterator from a sequence
    pub(super) fn lower_reversed(
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
        let elem_type = match &arg_type {
            Type::List(elem) => (**elem).clone(),
            Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
            Type::Tuple(_) => Type::Any,
            Type::Dict(key, _) => (**key).clone(),
            Type::Str => Type::Str,
            Type::Bytes => Type::Int, // bytes yields integers
            _ => Type::Any,
        };

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

        let iter_func = mir::RuntimeFunc::MakeIterator {
            source,
            direction: mir::IterDirection::Reversed,
        };

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: iter_func,
            args: vec![arg_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower reversed(range(start, stop, step)) - create a reversed range iterator
    pub(super) fn lower_reversed_range(
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
            func: mir::RuntimeFunc::MakeIterator {
                source: mir::IterSourceKind::Range,
                direction: mir::IterDirection::Reversed,
            },
            args: vec![start_operand, stop_operand, step_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower enumerate(iterable, start=0) - create an enumerate iterator
    /// Returns an iterator that yields (index, element) tuples
    pub(super) fn lower_enumerate(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        // Determine element type from container type
        let elem_type = match &arg_type {
            Type::List(elem) => (**elem).clone(),
            Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
            Type::Str => Type::Str,
            Type::Dict(key, _) => (**key).clone(),
            Type::Set(elem) => (**elem).clone(),
            Type::Bytes => Type::Int,
            _ => Type::Any,
        };

        // Create inner iterator first
        let inner_iter = self.lower_iter(args, hir_module, mir_func)?;

        // Get start value (second arg or default 0)
        let start_operand = if args.len() > 1 {
            let start_expr = &hir_module.exprs[args[1]];
            self.lower_expr(start_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::Int(0))
        };

        // Create result local with Iterator type wrapping (Int, elem_type) tuples
        let result_local = self.alloc_and_add_local(
            Type::Iterator(Box::new(Type::Tuple(vec![Type::Int, elem_type]))),
            mir_func,
        );

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::IterEnumerate,
            args: vec![inner_iter, start_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower sorted(iterable, key=None, reverse=False) - return new sorted list from iterable
    pub(super) fn lower_sorted(
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
        let elem_type = match &arg_type {
            Type::List(elem) => (**elem).clone(),
            Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
            Type::Tuple(_) => Type::Any,
            Type::Dict(key, _) => (**key).clone(),
            Type::Set(elem) => (**elem).clone(),
            Type::Str => Type::Str,
            _ => Type::Any,
        };

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
        if let Some(key_operand) = self.emit_key_func_addr(sort_kwargs.key_func.as_ref(), mir_func)
        {
            let elem_tag_operand = mir::Operand::Constant(mir::Constant::Int(elem_tag));

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Sorted {
                    source,
                    has_key: true,
                },
                args: vec![
                    arg_operand,
                    sort_kwargs.reverse,
                    key_operand,
                    elem_tag_operand,
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
                func: mir::RuntimeFunc::Sorted {
                    source,
                    has_key: false,
                },
                args,
            });
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower sorted(range(start, stop, step), reverse=False) - create sorted list from range
    pub(super) fn lower_sorted_range(
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
            func: mir::RuntimeFunc::Sorted {
                source: mir::SortableKind::Range,
                has_key: false,
            },
            args: vec![start_operand, stop_operand, step_operand, reverse_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower zip(iter1, iter2, ...) - create a zip iterator
    /// Returns an iterator that yields tuples from all iterables
    pub(super) fn lower_zip(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // zip() with no args returns an empty iterator
            let result_local =
                self.alloc_and_add_local(Type::Iterator(Box::new(Type::Tuple(vec![]))), mir_func);
            // Create empty tuple iterator
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::MakeList,
                args: vec![mir::Operand::Constant(mir::Constant::Int(0))],
            });
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::MakeIterator {
                    source: mir::IterSourceKind::List,
                    direction: mir::IterDirection::Forward,
                },
                args: vec![mir::Operand::Local(result_local)],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        // Handle 3+ arguments separately
        if args.len() == 3 {
            // Use Zip3New for exactly 3 iterables
            let mut iter_locals = Vec::new();
            let mut elem_types = Vec::new();

            for arg_id in args {
                let (iter_local, elem_type) =
                    self.make_iter_from_expr(*arg_id, hir_module, mir_func)?;
                iter_locals.push(mir::Operand::Local(iter_local));
                elem_types.push(elem_type);
            }

            let result_local = self.alloc_local_id();
            mir_func.add_local(mir::Local {
                id: result_local,
                name: None,
                ty: Type::Iterator(Box::new(Type::Tuple(elem_types))),
                is_gc_root: true,
            });

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Zip3New,
                args: iter_locals,
            });

            return Ok(mir::Operand::Local(result_local));
        } else if args.len() > 3 {
            // Use ZipNNew for 4+ iterables
            let mut iter_locals = Vec::new();
            let mut elem_types = Vec::new();

            for arg_id in args {
                let (iter_local, elem_type) =
                    self.make_iter_from_expr(*arg_id, hir_module, mir_func)?;
                iter_locals.push(mir::Operand::Local(iter_local));
                elem_types.push(elem_type);
            }

            // Create a list of iterators
            let count = args.len() as i64;
            let iter_list_local = self.alloc_and_add_local(
                Type::List(Box::new(Type::Iterator(Box::new(Type::Any)))),
                mir_func,
            );

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: iter_list_local,
                func: mir::RuntimeFunc::MakeList,
                args: vec![mir::Operand::Constant(mir::Constant::Int(count))],
            });

            // Push each iterator to the list
            for (i, iter_op) in iter_locals.iter().enumerate() {
                let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::ListSet,
                    args: vec![
                        mir::Operand::Local(iter_list_local),
                        mir::Operand::Constant(mir::Constant::Int(i as i64)),
                        iter_op.clone(),
                    ],
                });
            }

            let result_local = self.alloc_local_id();
            mir_func.add_local(mir::Local {
                id: result_local,
                name: None,
                ty: Type::Iterator(Box::new(Type::Tuple(elem_types))),
                is_gc_root: true,
            });

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::ZipNNew,
                args: vec![
                    mir::Operand::Local(iter_list_local),
                    mir::Operand::Constant(mir::Constant::Int(count)),
                ],
            });

            return Ok(mir::Operand::Local(result_local));
        }

        // Get first iterable and create iterator
        let first_expr = &hir_module.exprs[args[0]];

        // Check if first is a range() call - needs special handling
        let first_is_range = matches!(
            &first_expr.kind,
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                ..
            }
        );

        let first_type = self.get_expr_type(first_expr, hir_module);
        let first_elem_type = if first_is_range {
            Type::Int
        } else {
            match &first_type {
                Type::List(elem) => (**elem).clone(),
                Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
                Type::Str => Type::Str,
                Type::Dict(key, _) => (**key).clone(),
                Type::Set(elem) => (**elem).clone(),
                Type::Bytes => Type::Int,
                Type::Iterator(elem) => (**elem).clone(),
                _ => Type::Any,
            }
        };

        // Create first iterator
        let first_iter_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(first_elem_type.clone())), mir_func);

        // Handle range() specially for first iterator
        if first_is_range {
            if let hir::ExprKind::BuiltinCall {
                args: range_args, ..
            } = &first_expr.kind
            {
                let (start, stop, step) =
                    self.parse_range_args(range_args, hir_module, mir_func)?;
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: first_iter_local,
                    func: mir::RuntimeFunc::MakeIterator {
                        source: mir::IterSourceKind::Range,
                        direction: mir::IterDirection::Forward,
                    },
                    args: vec![start, stop, step],
                });
            }
        } else {
            let first_operand = self.lower_expr(first_expr, hir_module, mir_func)?;
            let first_source = match &first_type {
                Type::List(_) => mir::IterSourceKind::List,
                Type::Tuple(_) => mir::IterSourceKind::Tuple,
                Type::Dict(_, _) => mir::IterSourceKind::Dict,
                Type::Set(_) => mir::IterSourceKind::Set,
                Type::Str => mir::IterSourceKind::Str,
                Type::Bytes => mir::IterSourceKind::Bytes,
                Type::Iterator(_) => mir::IterSourceKind::Generator,
                _ => mir::IterSourceKind::List,
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: first_iter_local,
                func: mir::RuntimeFunc::MakeIterator {
                    source: first_source,
                    direction: mir::IterDirection::Forward,
                },
                args: vec![first_operand],
            });
        }

        // If only one argument, just return its iterator (unusual but valid)
        if args.len() == 1 {
            return Ok(mir::Operand::Local(first_iter_local));
        }

        // Get second iterable and create iterator
        let second_expr = &hir_module.exprs[args[1]];

        // Check if second is a range() call - needs special handling
        let second_is_range = matches!(
            &second_expr.kind,
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                ..
            }
        );

        let second_type = self.get_expr_type(second_expr, hir_module);
        let second_elem_type = if second_is_range {
            Type::Int
        } else {
            match &second_type {
                Type::List(elem) => (**elem).clone(),
                Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
                Type::Str => Type::Str,
                Type::Dict(key, _) => (**key).clone(),
                Type::Set(elem) => (**elem).clone(),
                Type::Bytes => Type::Int,
                Type::Iterator(elem) => (**elem).clone(),
                _ => Type::Any,
            }
        };

        // Create second iterator
        let second_iter_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(second_elem_type.clone())), mir_func);

        // Handle range() specially for second iterator
        if second_is_range {
            if let hir::ExprKind::BuiltinCall {
                args: range_args, ..
            } = &second_expr.kind
            {
                let (start, stop, step) =
                    self.parse_range_args(range_args, hir_module, mir_func)?;
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: second_iter_local,
                    func: mir::RuntimeFunc::MakeIterator {
                        source: mir::IterSourceKind::Range,
                        direction: mir::IterDirection::Forward,
                    },
                    args: vec![start, stop, step],
                });
            }
        } else {
            let second_operand = self.lower_expr(second_expr, hir_module, mir_func)?;
            let second_source = match &second_type {
                Type::List(_) => mir::IterSourceKind::List,
                Type::Tuple(_) => mir::IterSourceKind::Tuple,
                Type::Dict(_, _) => mir::IterSourceKind::Dict,
                Type::Set(_) => mir::IterSourceKind::Set,
                Type::Str => mir::IterSourceKind::Str,
                Type::Bytes => mir::IterSourceKind::Bytes,
                Type::Iterator(_) => mir::IterSourceKind::Generator,
                _ => mir::IterSourceKind::List,
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: second_iter_local,
                func: mir::RuntimeFunc::MakeIterator {
                    source: second_source,
                    direction: mir::IterDirection::Forward,
                },
                args: vec![second_operand],
            });
        }

        // Create zip iterator
        let result_local = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: result_local,
            name: None,
            ty: Type::Iterator(Box::new(Type::Tuple(vec![
                first_elem_type,
                second_elem_type,
            ]))),
            is_gc_root: true,
        });

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::ZipNew,
            args: vec![
                mir::Operand::Local(first_iter_local),
                mir::Operand::Local(second_iter_local),
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower map(func, iterable) - create iterator that applies func to each element
    /// Supports closures with captures - captures are stored in a tuple and passed to runtime
    /// Also supports first-class builtins (len, str, int, etc.)
    pub(super) fn lower_map(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        if args.len() < 2 {
            return Err(CompilerError::codegen_error(
                "map() requires at least 2 arguments",
            ));
        }

        // Extract function or builtin from first argument
        let func_expr = &hir_module.exprs[args[0]];
        let func_or_builtin = self
            .extract_func_or_builtin(func_expr, hir_module)
            .ok_or_else(|| {
                CompilerError::codegen_error("map() first argument must be a function")
            })?;

        // Get function pointer and captures based on whether it's a user function or builtin
        let (func_ptr_operand, captures_operand, capture_count, result_elem_type) =
            match func_or_builtin {
                FuncOrBuiltin::UserFunc(func_id, captures) => {
                    // Record capture types for inline closures so lambda type inference works correctly.
                    if !captures.is_empty() && !self.has_closure_capture_types(&func_id) {
                        let mut capture_types = Vec::new();
                        for capture_id in &captures {
                            let capture_expr = &hir_module.exprs[*capture_id];
                            let capture_type = self.get_expr_type(capture_expr, hir_module);
                            capture_types.push(capture_type);
                        }
                        self.insert_closure_capture_types(func_id, capture_types);
                    }

                    // Get function address
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: func_ptr_local,
                        func: func_id,
                    });

                    // Lower captures to a tuple (if any)
                    let (cap_op, cap_count) = if captures.is_empty() {
                        (
                            mir::Operand::Constant(mir::Constant::Int(0)), // null pointer
                            mir::Operand::Constant(mir::Constant::Int(0)),
                        )
                    } else {
                        let captures_tuple =
                            self.lower_captures_to_tuple(&captures, hir_module, mir_func)?;
                        let count = captures.len() as i64;
                        (
                            captures_tuple,
                            mir::Operand::Constant(mir::Constant::Int(count)),
                        )
                    };

                    // Determine result element type from the callback function's return type
                    let result_type = self.infer_callback_return_type(func_id, hir_module);

                    (
                        mir::Operand::Local(func_ptr_local),
                        cap_op,
                        cap_count,
                        result_type,
                    )
                }
                FuncOrBuiltin::Builtin(builtin_kind) => {
                    // Get builtin function pointer from runtime table
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                        dest: func_ptr_local,
                        builtin: builtin_kind,
                    });

                    // Builtins have no captures. Set bit 7 (0x80) in capture_count
                    // to signal the runtime to box raw int elements before calling.
                    let cap_op = mir::Operand::Constant(mir::Constant::Int(0));
                    let cap_count = mir::Operand::Constant(mir::Constant::Int(0x80));

                    // Infer result type based on builtin
                    let result_type = self.infer_builtin_return_type(builtin_kind);

                    (
                        mir::Operand::Local(func_ptr_local),
                        cap_op,
                        cap_count,
                        result_type,
                    )
                }
            };

        // Create iterator from second argument
        let iter_args = &args[1..2];
        let inner_iter = self.lower_iter(iter_args, hir_module, mir_func)?;

        let result_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(result_elem_type)), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::MapNew,
            args: vec![
                func_ptr_operand,
                inner_iter,
                captures_operand,
                capture_count,
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Infer the return type of a builtin function
    fn infer_builtin_return_type(&self, builtin: mir::BuiltinFunctionKind) -> Type {
        match builtin {
            mir::BuiltinFunctionKind::Len => Type::Int,
            mir::BuiltinFunctionKind::Str => Type::Str,
            mir::BuiltinFunctionKind::Int => Type::Int,
            mir::BuiltinFunctionKind::Float => Type::Float,
            mir::BuiltinFunctionKind::Bool => Type::Bool,
            mir::BuiltinFunctionKind::Abs => Type::Any, // Could be Int or Float
            mir::BuiltinFunctionKind::Hash => Type::Int,
            mir::BuiltinFunctionKind::Ord => Type::Int,
            mir::BuiltinFunctionKind::Chr => Type::Str,
            mir::BuiltinFunctionKind::Repr => Type::Str,
            mir::BuiltinFunctionKind::Type => Type::Str,
        }
    }

    /// Lower captured expressions to a tuple
    /// Used by map/filter to store closure captures at runtime
    ///
    /// Captures are stored as raw values (i64 for int/float/bool cast as pointer)
    /// because the lambda function expects them in the same format as direct closure calls.
    /// The runtime extracts them with rt_tuple_get() which preserves the raw i64 value.
    fn lower_captures_to_tuple(
        &mut self,
        captures: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let count = captures.len();

        // Lower all capture expressions first to know their operand types
        let mut capture_operands = Vec::with_capacity(count);
        for capture_id in captures {
            let capture_expr = &hir_module.exprs[*capture_id];
            capture_operands.push(self.lower_expr(capture_expr, hir_module, mir_func)?);
        }

        // Determine elem_tag from actual operand types (more reliable than expr types).
        // Use ELEM_RAW_INT when no capture needs GC tracing (all primitives),
        // ELEM_HEAP_OBJ when any capture is a heap type (str, list, cell, etc.)
        let capture_op_types: Vec<Type> = capture_operands
            .iter()
            .map(|op| self.operand_type(op, mir_func))
            .collect();
        let any_needs_gc = capture_op_types.iter().any(Type::is_heap);
        let capture_elem_tag: i64 = if any_needs_gc { 0 } else { 1 };

        let tuple_local = self.alloc_and_add_local(Type::Tuple(vec![Type::Any; count]), mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: tuple_local,
            func: mir::RuntimeFunc::MakeTuple,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(count as i64)),
                mir::Operand::Constant(mir::Constant::Int(capture_elem_tag)),
            ],
        });

        // Set per-field heap_field_mask for mixed-type captures
        if capture_elem_tag == 0 {
            self.emit_heap_field_mask(tuple_local, &capture_op_types, mir_func);
        }

        // Set each capture into the tuple
        // Captures are stored as-is (raw i64 for primitives, pointers for heap types)
        // This matches how closures pass captures directly in lower_closure_call
        for (i, capture_operand) in capture_operands.into_iter().enumerate() {
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: tuple_local,
                func: mir::RuntimeFunc::TupleSet,
                args: vec![
                    mir::Operand::Local(tuple_local),
                    mir::Operand::Constant(mir::Constant::Int(i as i64)),
                    capture_operand,
                ],
            });
        }

        Ok(mir::Operand::Local(tuple_local))
    }

    /// Lower reduce(func, iterable, initial?) - fold iterable to single value
    /// Follows the same pattern as map/filter for callable extraction
    pub(super) fn lower_reduce(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        if args.len() < 2 || args.len() > 3 {
            return Err(CompilerError::codegen_error(
                "reduce() requires 2 or 3 arguments",
            ));
        }

        // Note: lambda param type hints for reduce are registered during the
        // precompute_closure_capture_types phase (in scan_expr_for_closures)
        // since lambdas are lowered before the module init function.

        // Extract function or builtin from first argument
        let func_expr = &hir_module.exprs[args[0]];
        let func_or_builtin = self
            .extract_func_or_builtin(func_expr, hir_module)
            .ok_or_else(|| {
                CompilerError::codegen_error("reduce() first argument must be a function")
            })?;

        // Extract func_id before consuming func_or_builtin (for result type inference)
        let reduce_func_id = match &func_or_builtin {
            FuncOrBuiltin::UserFunc(func_id, _) => Some(*func_id),
            FuncOrBuiltin::Builtin(_) => None,
        };

        // Get function pointer and captures
        let (func_ptr_operand, captures_operand, capture_count) = match func_or_builtin {
            FuncOrBuiltin::UserFunc(func_id, captures) => {
                // Record capture types for inline closures
                if !captures.is_empty() && !self.has_closure_capture_types(&func_id) {
                    let mut capture_types = Vec::new();
                    for capture_id in &captures {
                        let capture_expr = &hir_module.exprs[*capture_id];
                        let capture_type = self.get_expr_type(capture_expr, hir_module);
                        capture_types.push(capture_type);
                    }
                    self.insert_closure_capture_types(func_id, capture_types);
                }

                // Get function address
                let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::FuncAddr {
                    dest: func_ptr_local,
                    func: func_id,
                });

                // Lower captures to a tuple (if any)
                let (cap_op, cap_count) = if captures.is_empty() {
                    (
                        mir::Operand::Constant(mir::Constant::Int(0)),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                    )
                } else {
                    let captures_tuple =
                        self.lower_captures_to_tuple(&captures, hir_module, mir_func)?;
                    let count = captures.len() as i64;
                    (
                        captures_tuple,
                        mir::Operand::Constant(mir::Constant::Int(count)),
                    )
                };

                (mir::Operand::Local(func_ptr_local), cap_op, cap_count)
            }
            FuncOrBuiltin::Builtin(builtin_kind) => {
                let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                    dest: func_ptr_local,
                    builtin: builtin_kind,
                });

                (
                    mir::Operand::Local(func_ptr_local),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Constant(mir::Constant::Int(0)),
                )
            }
        };

        // Create iterator from second argument
        let iter_args = &args[1..2];
        let inner_iter = self.lower_iter(iter_args, hir_module, mir_func)?;

        // Get initial value (third argument, if provided)
        // Pass as-is (don't box) since lambda expects same type as iterable elements
        let (initial_operand, has_initial) = if args.len() > 2 {
            let initial_expr = &hir_module.exprs[args[2]];
            let initial_op = self.lower_expr(initial_expr, hir_module, mir_func)?;
            (initial_op, mir::Operand::Constant(mir::Constant::Int(1)))
        } else {
            (
                mir::Operand::Constant(mir::Constant::Int(0)),
                mir::Operand::Constant(mir::Constant::Int(0)),
            )
        };

        // Infer result type from the callback's return type
        let result_type = reduce_func_id
            .and_then(|fid| self.get_func_return_type(&fid).cloned())
            .unwrap_or(Type::Any);
        let result_local = self.alloc_and_add_local(result_type, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::ReduceNew,
            args: vec![
                func_ptr_operand,
                inner_iter,
                initial_operand,
                has_initial,
                captures_operand,
                capture_count,
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower filter(func, iterable) - create iterator that yields elements where func returns true
    /// When func is None, filter by truthiness (filter out falsy values)
    /// Supports closures with captures - captures are stored in a tuple and passed to runtime
    /// Also supports first-class builtins (bool, etc.)
    pub(super) fn lower_filter(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        if args.len() < 2 {
            return Err(CompilerError::codegen_error(
                "filter() requires at least 2 arguments",
            ));
        }

        // Check if first argument is None (truthiness filtering)
        let func_expr = &hir_module.exprs[args[0]];
        let is_none_predicate = matches!(func_expr.kind, hir::ExprKind::None);

        // Get function pointer operand and captures
        let (func_ptr_operand, captures_operand, capture_count) = if is_none_predicate {
            // Use 0 to indicate truthiness filtering, no captures
            (
                mir::Operand::Constant(mir::Constant::Int(0)),
                mir::Operand::Constant(mir::Constant::Int(0)), // null pointer
                mir::Operand::Constant(mir::Constant::Int(0)),
            )
        } else {
            // Extract function or builtin from first argument
            let func_or_builtin = self
                .extract_func_or_builtin(func_expr, hir_module)
                .ok_or_else(|| {
                    CompilerError::codegen_error(
                        "filter() first argument must be a function or None",
                    )
                })?;

            match func_or_builtin {
                FuncOrBuiltin::UserFunc(func_id, captures) => {
                    // Record capture types for inline closures
                    if !captures.is_empty() && !self.has_closure_capture_types(&func_id) {
                        let mut capture_types = Vec::new();
                        for capture_id in &captures {
                            let capture_expr = &hir_module.exprs[*capture_id];
                            let capture_type = self.get_expr_type(capture_expr, hir_module);
                            capture_types.push(capture_type);
                        }
                        self.insert_closure_capture_types(func_id, capture_types);
                    }

                    // Get function address
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::FuncAddr {
                        dest: func_ptr_local,
                        func: func_id,
                    });

                    // Lower captures to a tuple (if any)
                    let (cap_op, cap_count) = if captures.is_empty() {
                        (
                            mir::Operand::Constant(mir::Constant::Int(0)),
                            mir::Operand::Constant(mir::Constant::Int(0)),
                        )
                    } else {
                        let captures_tuple =
                            self.lower_captures_to_tuple(&captures, hir_module, mir_func)?;
                        let count = captures.len() as i64;
                        (
                            captures_tuple,
                            mir::Operand::Constant(mir::Constant::Int(count)),
                        )
                    };

                    (mir::Operand::Local(func_ptr_local), cap_op, cap_count)
                }
                FuncOrBuiltin::Builtin(builtin_kind) => {
                    // Get builtin function pointer from runtime table
                    let func_ptr_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BuiltinAddr {
                        dest: func_ptr_local,
                        builtin: builtin_kind,
                    });

                    // Builtins have no captures
                    (
                        mir::Operand::Local(func_ptr_local),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                        mir::Operand::Constant(mir::Constant::Int(0)),
                    )
                }
            }
        };

        // Create iterator from second argument
        let iter_args = &args[1..2];
        let inner_iter = self.lower_iter(iter_args, hir_module, mir_func)?;

        // Element type is same as input iterator
        let iterable_expr = &hir_module.exprs[args[1]];
        let iterable_type = self.get_expr_type(iterable_expr, hir_module);
        let elem_type = match &iterable_type {
            Type::List(elem) => (**elem).clone(),
            Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
            Type::Str => Type::Str,
            Type::Dict(key, _) => (**key).clone(),
            Type::Set(elem) => (**elem).clone(),
            _ => Type::Any,
        };

        // Determine elem_tag for truthiness filtering
        // Match how list literals store elements (see collections.rs):
        // - Int uses ELEM_RAW_INT (1) - stored as raw i64
        // - Bool uses ELEM_HEAP_OBJ (0) - stored as boxed bools
        // - All others use ELEM_HEAP_OBJ (0)
        let elem_tag: i64 = match &elem_type {
            Type::Int => 1, // ELEM_RAW_INT
            _ => 0,         // ELEM_HEAP_OBJ (Bool, Float, Str, etc. - all boxed)
        };

        let result_local = self.alloc_and_add_local(Type::Iterator(Box::new(elem_type)), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::FilterNew,
            args: vec![
                func_ptr_operand,
                inner_iter,
                mir::Operand::Constant(mir::Constant::Int(elem_tag)),
                captures_operand,
                capture_count,
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower itertools.chain(*iterables) - chain multiple iterators sequentially
    pub(super) fn lower_chain(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let num_iters = args.len() as i64;

        // Create a list to hold all iterators (elem_tag=0 for heap objects)
        let iters_list_local = self.alloc_and_add_local(Type::List(Box::new(Type::Any)), mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: iters_list_local,
            func: mir::RuntimeFunc::MakeList,
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(num_iters)),
                mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
            ],
        });

        // Create iterators for each argument and add to list
        for arg_id in args.iter() {
            let arg_expr = &hir_module.exprs[*arg_id];
            let arg_type = self.get_expr_type(arg_expr, hir_module);

            // Check if this is a range() call
            let is_range = matches!(
                &arg_expr.kind,
                hir::ExprKind::BuiltinCall {
                    builtin: hir::Builtin::Range,
                    ..
                }
            );

            let iter_local =
                self.alloc_and_add_local(Type::Iterator(Box::new(Type::Any)), mir_func);

            if is_range {
                if let hir::ExprKind::BuiltinCall {
                    args: range_args, ..
                } = &arg_expr.kind
                {
                    let (start, stop, step) =
                        self.parse_range_args(range_args, hir_module, mir_func)?;
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: iter_local,
                        func: mir::RuntimeFunc::MakeIterator {
                            source: mir::IterSourceKind::Range,
                            direction: mir::IterDirection::Forward,
                        },
                        args: vec![start, stop, step],
                    });
                }
            } else {
                let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
                let source = match &arg_type {
                    Type::List(_) => mir::IterSourceKind::List,
                    Type::Tuple(_) => mir::IterSourceKind::Tuple,
                    Type::Dict(_, _) => mir::IterSourceKind::Dict,
                    Type::Set(_) => mir::IterSourceKind::Set,
                    Type::Str => mir::IterSourceKind::Str,
                    Type::Bytes => mir::IterSourceKind::Bytes,
                    Type::Iterator(_) => mir::IterSourceKind::Generator,
                    _ => mir::IterSourceKind::List,
                };
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: iter_local,
                    func: mir::RuntimeFunc::MakeIterator {
                        source,
                        direction: mir::IterDirection::Forward,
                    },
                    args: vec![arg_operand],
                });
            }

            // Add iterator to list using push (increments list length)
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: iter_local, // unused dest
                func: mir::RuntimeFunc::ListPush,
                args: vec![
                    mir::Operand::Local(iters_list_local),
                    mir::Operand::Local(iter_local),
                ],
            });
        }

        // Create chain iterator
        let result_local = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: result_local,
            name: None,
            ty: Type::Iterator(Box::new(Type::Any)),
            is_gc_root: true,
        });

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::ChainNew,
            args: vec![
                mir::Operand::Local(iters_list_local),
                mir::Operand::Constant(mir::Constant::Int(num_iters)),
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower itertools.islice(iterable, stop) or islice(iterable, start, stop[, step])
    pub(super) fn lower_islice(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        if args.len() < 2 || args.len() > 4 {
            return Err(CompilerError::codegen_error(
                "islice() requires 2-4 arguments",
            ));
        }

        // Get the iterable and create an iterator
        let iter_expr = &hir_module.exprs[args[0]];
        let iter_type = self.get_expr_type(iter_expr, hir_module);

        let is_range = matches!(
            &iter_expr.kind,
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                ..
            }
        );

        let inner_iter_local =
            self.alloc_and_add_local(Type::Iterator(Box::new(Type::Any)), mir_func);

        if is_range {
            if let hir::ExprKind::BuiltinCall {
                args: range_args, ..
            } = &iter_expr.kind
            {
                let (start, stop, step) =
                    self.parse_range_args(range_args, hir_module, mir_func)?;
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: inner_iter_local,
                    func: mir::RuntimeFunc::MakeIterator {
                        source: mir::IterSourceKind::Range,
                        direction: mir::IterDirection::Forward,
                    },
                    args: vec![start, stop, step],
                });
            }
        } else {
            let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
            let source = match &iter_type {
                Type::List(_) => mir::IterSourceKind::List,
                Type::Tuple(_) => mir::IterSourceKind::Tuple,
                Type::Dict(_, _) => mir::IterSourceKind::Dict,
                Type::Set(_) => mir::IterSourceKind::Set,
                Type::Str => mir::IterSourceKind::Str,
                Type::Bytes => mir::IterSourceKind::Bytes,
                Type::Iterator(_) => mir::IterSourceKind::Generator,
                _ => mir::IterSourceKind::List,
            };
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: inner_iter_local,
                func: mir::RuntimeFunc::MakeIterator {
                    source,
                    direction: mir::IterDirection::Forward,
                },
                args: vec![iter_operand],
            });
        }

        // Parse start/stop/step based on argument count
        // islice(iterable, stop) -> start=0, stop=stop, step=1
        // islice(iterable, start, stop) -> start=start, stop=stop, step=1
        // islice(iterable, start, stop, step) -> start=start, stop=stop, step=step
        let (start_op, stop_op, step_op) = if args.len() == 2 {
            let stop_expr = &hir_module.exprs[args[1]];
            let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
            (
                mir::Operand::Constant(mir::Constant::Int(0)),
                stop,
                mir::Operand::Constant(mir::Constant::Int(1)),
            )
        } else if args.len() == 3 {
            let start_expr = &hir_module.exprs[args[1]];
            let stop_expr = &hir_module.exprs[args[2]];
            let start = self.lower_expr(start_expr, hir_module, mir_func)?;
            let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
            (start, stop, mir::Operand::Constant(mir::Constant::Int(1)))
        } else {
            let start_expr = &hir_module.exprs[args[1]];
            let stop_expr = &hir_module.exprs[args[2]];
            let step_expr = &hir_module.exprs[args[3]];
            let start = self.lower_expr(start_expr, hir_module, mir_func)?;
            let stop = self.lower_expr(stop_expr, hir_module, mir_func)?;
            let step = self.lower_expr(step_expr, hir_module, mir_func)?;
            (start, stop, step)
        };

        // Create islice iterator
        let result_local = self.alloc_local_id();
        mir_func.add_local(mir::Local {
            id: result_local,
            name: None,
            ty: Type::Iterator(Box::new(Type::Any)),
            is_gc_root: true,
        });

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::ISliceNew,
            args: vec![
                mir::Operand::Local(inner_iter_local),
                start_op,
                stop_op,
                step_op,
            ],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Helper function to create an iterator from an expression and return the iterator local and element type
    fn make_iter_from_expr(
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

        let expr_type = self.get_expr_type(expr, hir_module);
        let elem_type = if is_range {
            Type::Int
        } else {
            match &expr_type {
                Type::List(elem) => (**elem).clone(),
                Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
                Type::Str => Type::Str,
                Type::Dict(key, _) => (**key).clone(),
                Type::Set(elem) => (**elem).clone(),
                Type::Bytes => Type::Int,
                Type::Iterator(elem) => (**elem).clone(),
                _ => Type::Any,
            }
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
                    func: mir::RuntimeFunc::MakeIterator {
                        source: mir::IterSourceKind::Range,
                        direction: mir::IterDirection::Forward,
                    },
                    args: vec![start, stop, step],
                });
            }
        } else {
            let operand = self.lower_expr(expr, hir_module, mir_func)?;
            let source = match &expr_type {
                Type::List(_) => mir::IterSourceKind::List,
                Type::Tuple(_) => mir::IterSourceKind::Tuple,
                Type::Dict(_, _) => mir::IterSourceKind::Dict,
                Type::Set(_) => mir::IterSourceKind::Set,
                Type::Str => mir::IterSourceKind::Str,
                Type::Bytes => mir::IterSourceKind::Bytes,
                Type::Iterator(_) => mir::IterSourceKind::Generator,
                _ => mir::IterSourceKind::List,
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: iter_local,
                func: mir::RuntimeFunc::MakeIterator {
                    source,
                    direction: mir::IterDirection::Forward,
                },
                args: vec![operand],
            });
        }

        Ok((iter_local, elem_type))
    }
}
