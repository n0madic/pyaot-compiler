//! Collection functions lowering: len(), set()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::{Type, TypeTagKind};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower len(x)
    pub(super) fn lower_len(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::Int(0)));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.seed_expr_type(args[0], hir_module);

        let result_local = self.alloc_and_add_local(Type::Int, mir_func);

        if let Some(len_func) = crate::type_dispatch::select_len_func(&arg_type) {
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(len_func),
                args: vec![arg_operand],
            });
        } else {
            match arg_type {
                Type::Class { class_id, .. } => {
                    // Check for __len__ method
                    if let Some(class_info) = self.get_class_info(&class_id) {
                        if let Some(len_func) = class_info.get_dunder_func("__len__") {
                            // Call __len__ method
                            self.emit_instruction(mir::InstructionKind::CallDirect {
                                dest: result_local,
                                func: len_func,
                                args: vec![arg_operand],
                            });
                        } else {
                            // No __len__ - raise TypeError
                            let type_name = self.intern("object of type 'instance' has no len()");
                            self.current_block_mut().terminator = mir::Terminator::Raise {
                                exc_type: 5, // TypeError
                                message: Some(mir::Operand::Constant(mir::Constant::Str(
                                    type_name,
                                ))),
                                cause: None,
                                suppress_context: false,
                            };
                            // Create unreachable block for dead code
                            let unreachable_bb = self.new_block();
                            self.push_block(unreachable_bb);
                        }
                    }
                }
                _ => {
                    // Fallback: return 0
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: result_local,
                        value: mir::Constant::Int(0),
                    });
                }
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower set() builtin - creates empty set or set from iterable
    pub(super) fn lower_set_builtin(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Bidirectional: use expected_type for empty set() calls
        let set_elem_type = if let Some(Type::Set(ref expected_elem)) = self.codegen.expected_type {
            (**expected_elem).clone()
        } else {
            Type::Any
        };
        let result_local = self.alloc_and_add_local(Type::Set(Box::new(set_elem_type)), mir_func);

        if args.is_empty() {
            // set() - create empty set
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_SET),
                args: vec![mir::Operand::Constant(mir::Constant::Int(8))],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        // set(iterable) - create set from iterable
        let iter_expr = &hir_module.exprs[args[0]];
        let iter_type = self.seed_expr_type(args[0], hir_module);

        // Determine element type
        let elem_type = match &iter_type {
            Type::List(t) => (**t).clone(),
            Type::Tuple(ts) if !ts.is_empty() => ts[0].clone(),
            Type::Set(t) => (**t).clone(),
            Type::Str => Type::Str,
            Type::Dict(k, _) => (**k).clone(),
            _ => Type::Any,
        };

        // Create the set with estimated capacity
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_SET),
            args: vec![mir::Operand::Constant(mir::Constant::Int(8))],
        });

        // Create iterator over the source
        let source_operand = self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?;

        // Get appropriate iterator source kind based on type
        let source = match &iter_type {
            Type::List(_) => mir::IterSourceKind::List,
            Type::Tuple(_) => mir::IterSourceKind::Tuple,
            Type::Str => mir::IterSourceKind::Str,
            Type::Dict(_, _) => mir::IterSourceKind::Dict,
            Type::Set(_) => mir::IterSourceKind::Set,
            _ => mir::IterSourceKind::List, // fallback
        };
        let iter_func = mir::RuntimeFunc::Call(source.iterator_def(mir::IterDirection::Forward));

        // Create iterator
        let iter_local = self.emit_runtime_call(
            iter_func,
            vec![source_operand],
            Type::Iterator(Box::new(elem_type.clone())),
            mir_func,
        );

        // Loop to add each element from iterator
        let loop_header = self.new_block();
        let loop_body = self.new_block();
        let loop_exit = self.new_block();

        let loop_header_id = loop_header.id;
        let loop_body_id = loop_body.id;
        let loop_exit_id = loop_exit.id;

        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop header: try to get next element
        self.push_block(loop_header);

        let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);

        // Push exception frame to catch StopIteration
        let exc_frame_local = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::ExcPushFrame {
            frame_local: exc_frame_local,
        });

        let try_next = self.new_block();
        let handle_stop = self.new_block();

        self.current_block_mut().terminator = mir::Terminator::TrySetjmp {
            frame_local: exc_frame_local,
            try_body: try_next.id,
            handler_entry: handle_stop.id,
        };

        // Try block: call next()
        self.push_block(try_next);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: elem_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT),
            args: vec![mir::Operand::Local(iter_local)],
        });

        self.emit_instruction(mir::InstructionKind::ExcPopFrame);

        self.current_block_mut().terminator = mir::Terminator::Goto(loop_body_id);

        // Handler: StopIteration - exit loop
        self.push_block(handle_stop);

        self.emit_instruction(mir::InstructionKind::ExcClear);
        self.current_block_mut().terminator = mir::Terminator::Goto(loop_exit_id);

        // Loop body: add element to set
        self.push_block(loop_body);

        // Box element if needed
        let boxed_elem =
            self.box_primitive_if_needed(mir::Operand::Local(elem_local), &elem_type, mir_func);

        self.emit_runtime_call_void(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_ADD),
            vec![mir::Operand::Local(result_local), boxed_elem],
            mir_func,
        );

        self.current_block_mut().terminator = mir::Terminator::Goto(loop_header_id);

        // Loop exit
        self.push_block(loop_exit);

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower list() builtin - creates empty list or list from iterable
    pub(super) fn lower_list_builtin(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Use expected_type from assignment context (e.g. `x: list[int] = list(...)`)
        // for precise element type. This enables ListGetTyped instead of generic ListGet.
        let list_elem_type = if let Some(Type::List(ref expected_elem)) = self.codegen.expected_type
        {
            (**expected_elem).clone()
        } else {
            Type::Any
        };
        let result_local = self.alloc_and_add_local(Type::List(Box::new(list_elem_type)), mir_func);

        if args.is_empty() {
            // list() - create empty list
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_LIST),
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                ],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        // list(iterable) - create list from iterable
        let iter_expr = &hir_module.exprs[args[0]];
        let hir_type = self.seed_expr_type(args[0], hir_module);

        // Check for range() call - handle specially
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            args: range_args,
            ..
        } = &iter_expr.kind
        {
            return self.lower_list_from_range(range_args, hir_module, mir_func);
        }

        let source_operand = self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?;

        // Use the lowered operand type if the HIR type is unknown (Any).
        // map/filter infer Iterator(Int) during lowering, but the HIR may still say Any.
        // We always use ELEM_HEAP_OBJ for map/filter iterators because the map callback
        // ABI returns *mut Obj (boxed), and ListGetTyped(Int) can transparently unbox both.
        let lowered_type = self.operand_type(&source_operand, mir_func);
        let iter_type = match &hir_type {
            Type::Any if matches!(lowered_type, Type::Iterator(_)) => lowered_type,
            other => other.clone(),
        };

        // Dispatch based on source type
        match &iter_type {
            Type::Tuple(_) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_FROM_TUPLE,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::Str => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_FROM_STR,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::Set(_) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_FROM_SET,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::Dict(_, _) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_FROM_DICT,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::List(_) => {
                // list(list) -> shallow copy
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_COPY),
                    args: vec![source_operand],
                });
            }
            Type::Iterator(_) => {
                // Always use ELEM_HEAP_OBJ for generic iterators (map, filter, etc.)
                // because the iterator protocol returns *mut Obj. ListGetTyped(Int/Bool)
                // transparently handles unboxing from ELEM_HEAP_OBJ storage.
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_FROM_ITER,
                    ),
                    args: vec![
                        source_operand,
                        mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                    ],
                });
            }
            _ => {
                // Fallback: try as iterator (assume heap objects)
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_FROM_ITER,
                    ),
                    args: vec![
                        source_operand,
                        mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                    ],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower list(range(start, stop, step))
    fn lower_list_from_range(
        &mut self,
        range_args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let (start, stop, step) = match range_args.len() {
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
            _ => {
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        };

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_FROM_RANGE),
            vec![start, stop, step],
            Type::List(Box::new(Type::Int)),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower tuple() builtin - creates empty tuple or tuple from iterable
    pub(super) fn lower_tuple_builtin(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Create result local with Tuple type - use vec![Any] to match typecheck
        let result_local = self.alloc_and_add_local(Type::Tuple(vec![Type::Any]), mir_func);

        if args.is_empty() {
            // tuple() - create empty tuple
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_TUPLE),
                args: vec![
                    mir::Operand::Constant(mir::Constant::Int(0)),
                    mir::Operand::Constant(mir::Constant::Int(0)), // ELEM_HEAP_OBJ
                ],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        // tuple(iterable) - create tuple from iterable
        let iter_expr = &hir_module.exprs[args[0]];
        let iter_type = self.seed_expr_type(args[0], hir_module);

        // Check for range() call - handle specially
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            args: range_args,
            ..
        } = &iter_expr.kind
        {
            return self.lower_tuple_from_range(range_args, hir_module, mir_func);
        }

        let source_operand = self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?;

        // Dispatch based on source type
        match &iter_type {
            Type::List(_) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_FROM_LIST,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::Str => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_FROM_STR,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::Set(_) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_FROM_SET,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::Dict(_, _) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_FROM_DICT,
                    ),
                    args: vec![source_operand],
                });
            }
            Type::Tuple(_) => {
                // tuple(tuple) -> same tuple (or copy)
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: source_operand,
                });
            }
            Type::Iterator(_) => {
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_FROM_ITER,
                    ),
                    args: vec![source_operand],
                });
            }
            _ => {
                // Fallback: try as iterator
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_TUPLE_FROM_ITER,
                    ),
                    args: vec![source_operand],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower tuple(range(start, stop, step))
    fn lower_tuple_from_range(
        &mut self,
        range_args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let (start, stop, step) = match range_args.len() {
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
            _ => {
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        };

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_FROM_RANGE),
            vec![start, stop, step],
            Type::Tuple(vec![Type::Int]),
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower dict() builtin - creates empty dict, dict from kwargs, or dict from iterable of pairs
    pub(super) fn lower_dict_builtin(
        &mut self,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Bidirectional: use expected_type for empty dict() calls
        let (dict_key_type, dict_val_type) =
            if let Some(Type::Dict(ref ek, ref ev)) = self.codegen.expected_type {
                ((**ek).clone(), (**ev).clone())
            } else {
                (Type::Any, Type::Any)
            };
        let result_local = self.alloc_and_add_local(
            Type::Dict(Box::new(dict_key_type), Box::new(dict_val_type)),
            mir_func,
        );

        // Start by creating an empty dict
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_DICT),
            args: vec![mir::Operand::Constant(mir::Constant::Int(8))],
        });

        // Dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Process positional argument (iterable of pairs) if present
        if !args.is_empty() {
            let iter_expr = &hir_module.exprs[args[0]];
            let iter_type = self.seed_expr_type(args[0], hir_module);
            let source_operand =
                self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?;

            // If it's a list of pairs, use DictFromPairs
            match &iter_type {
                Type::List(_) => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DICT_FROM_PAIRS,
                        ),
                        args: vec![source_operand],
                    });
                }
                Type::Dict(_, _) => {
                    // dict(other_dict) -> copy
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DICT_COPY,
                        ),
                        args: vec![source_operand],
                    });
                }
                _ => {
                    // Try treating as iterable of pairs
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DICT_FROM_PAIRS,
                        ),
                        args: vec![source_operand],
                    });
                }
            }
        }

        // Process keyword arguments: dict(a=1, b=2)
        for kwarg in kwargs {
            let value_expr = &hir_module.exprs[kwarg.value];
            let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;
            let value_type = self.seed_expr_type(kwarg.value, hir_module);

            // Create string key - use the interned string directly
            let key_local = self.emit_runtime_call(
                mir::RuntimeFunc::MakeStr,
                vec![mir::Operand::Constant(mir::Constant::Str(kwarg.name))],
                Type::Str,
                mir_func,
            );

            // Box primitive values (all dict values must be heap pointers for GC)
            let boxed_value = self.box_primitive_if_needed(value_operand, &value_type, mir_func);

            // Set the key-value pair
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                args: vec![
                    mir::Operand::Local(result_local),
                    mir::Operand::Local(key_local),
                    boxed_value,
                ],
            });
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower defaultdict(factory) builtin
    /// factory is a type constructor: int, str, list, dict, set, float, bool
    /// Encoded as factory_tag: 0=int, 1=float, 2=str, 3=bool, 4=list, 5=dict, 6=set
    pub(super) fn lower_defaultdict(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_diagnostics::CompilerError;

        // The frontend has already resolved the factory name to an integer tag.
        // args[0] is an Int literal: 0=int, 1=float, 2=str, 3=bool, 4=list, 5=dict, 6=set
        let (factory_tag, value_type) = if args.is_empty() {
            // defaultdict() with no factory — behaves like regular dict
            (-1i64, Type::Any)
        } else {
            let factory_expr = &hir_module.exprs[args[0]];
            match &factory_expr.kind {
                hir::ExprKind::Int(tag) => {
                    let vt = match *tag {
                        0 => Type::Int,
                        1 => Type::Float,
                        2 => Type::Str,
                        3 => Type::Bool,
                        4 => Type::List(Box::new(Type::Any)),
                        5 => Type::Dict(Box::new(Type::Any), Box::new(Type::Any)),
                        6 => Type::Set(Box::new(Type::Any)),
                        _ => Type::Any,
                    };
                    (*tag, vt)
                }
                _ => {
                    let factory_expr = &hir_module.exprs[args[0]];
                    return Err(CompilerError::codegen_error(
                        "defaultdict factory must be a type name (int, str, list, etc.)",
                        Some(factory_expr.span),
                    ));
                }
            }
        };

        // When no factory, use regular Dict type (standard boxing/unboxing)
        let result_type = if factory_tag < 0 {
            Type::Dict(Box::new(Type::Any), Box::new(value_type))
        } else {
            Type::DefaultDict(Box::new(Type::Any), Box::new(value_type))
        };
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_DEFAULT_DICT),
            vec![
                mir::Operand::Constant(mir::Constant::Int(8)), // capacity
                mir::Operand::Constant(mir::Constant::Int(factory_tag)),
            ],
            result_type,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower Counter(iterable?) builtin
    pub(super) fn lower_counter(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let result_local =
            self.alloc_and_add_local(Type::RuntimeObject(TypeTagKind::Counter), mir_func);

        if args.is_empty() {
            // Counter() — empty counter
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_MAKE_COUNTER_EMPTY,
                ),
                args: vec![],
            });
        } else {
            // Counter(iterable) — count elements
            let iter_expr = &hir_module.exprs[args[0]];
            let iter_type = self.seed_expr_type(args[0], hir_module);

            // Create iterator from the argument
            let iter_operand = if matches!(iter_type, Type::Iterator(_)) {
                self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?
            } else {
                // Need to convert to iterator first
                let source = self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?;
                let iter_local =
                    self.alloc_and_add_local(Type::Iterator(Box::new(Type::Any)), mir_func);
                let source_kind = match &iter_type {
                    Type::List(_) => mir::IterSourceKind::List,
                    Type::Tuple(_) => mir::IterSourceKind::Tuple,
                    Type::Dict(_, _) => mir::IterSourceKind::Dict,
                    Type::Set(_) => mir::IterSourceKind::Set,
                    Type::Str => mir::IterSourceKind::Str,
                    _ => mir::IterSourceKind::List,
                };
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: iter_local,
                    func: mir::RuntimeFunc::Call(
                        source_kind.iterator_def(mir::IterDirection::Forward),
                    ),
                    args: vec![source],
                });
                mir::Operand::Local(iter_local)
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_MAKE_COUNTER_FROM_ITER,
                ),
                args: vec![iter_operand],
            });
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower deque(iterable?, maxlen?) builtin
    pub(super) fn lower_deque(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let result_local =
            self.alloc_and_add_local(Type::RuntimeObject(TypeTagKind::Deque), mir_func);

        // Get maxlen (default -1 = unbounded)
        let maxlen = if args.len() >= 2 {
            let maxlen_expr = &hir_module.exprs[args[1]];
            self.lower_expr(maxlen_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::Int(-1))
        };

        // Check if the first arg is None (deque(maxlen=3) case where we padded with None)
        let has_iterable = if args.is_empty() {
            false
        } else {
            let first_expr = &hir_module.exprs[args[0]];
            !matches!(first_expr.kind, hir::ExprKind::None)
        };

        if !has_iterable {
            // deque() or deque(maxlen=N) — empty deque
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_DEQUE),
                args: vec![maxlen],
            });
        } else {
            // deque(iterable, maxlen?) — from iterator
            let iter_expr = &hir_module.exprs[args[0]];
            let iter_type = self.seed_expr_type(args[0], hir_module);

            let iter_operand = if matches!(iter_type, Type::Iterator(_)) {
                self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?
            } else {
                let source = self.lower_expr_expecting(iter_expr, None, hir_module, mir_func)?;
                let iter_local =
                    self.alloc_and_add_local(Type::Iterator(Box::new(Type::Any)), mir_func);
                let source_kind = match &iter_type {
                    Type::List(_) => mir::IterSourceKind::List,
                    Type::Tuple(_) => mir::IterSourceKind::Tuple,
                    Type::Dict(_, _) => mir::IterSourceKind::Dict,
                    Type::Set(_) => mir::IterSourceKind::Set,
                    Type::Str => mir::IterSourceKind::Str,
                    _ => mir::IterSourceKind::List,
                };
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: iter_local,
                    func: mir::RuntimeFunc::Call(
                        source_kind.iterator_def(mir::IterDirection::Forward),
                    ),
                    args: vec![source],
                });
                mir::Operand::Local(iter_local)
            };

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_MAKE_DEQUE_FROM_ITER,
                ),
                args: vec![iter_operand, maxlen],
            });
        }

        Ok(mir::Operand::Local(result_local))
    }
}
