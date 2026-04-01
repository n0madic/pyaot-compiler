//! Chain and slice iteration lowering: chain(), islice(), enumerate()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower enumerate(iterable, start=0) - create an enumerate iterator
    /// Returns an iterator that yields (index, element) tuples
    pub(in crate::expressions::builtins) fn lower_enumerate(
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
        let elem_type = crate::type_planning::infer::extract_iterable_first_element_type(&arg_type);

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

    /// Lower itertools.chain(*iterables) - chain multiple iterators sequentially
    pub(in crate::expressions::builtins) fn lower_chain(
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
    pub(in crate::expressions::builtins) fn lower_islice(
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
}
