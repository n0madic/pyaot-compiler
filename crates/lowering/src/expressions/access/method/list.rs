//! List method lowering
#![allow(clippy::too_many_arguments)]

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower list method calls.
    pub(super) fn lower_list_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        arg_types: Vec<Type>,
        kwargs: &[hir::KeywordArg],
        elem_ty: Box<Type>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match method_name {
            "append" => {
                // .append(value) - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let value_operand = crate::first_arg_or_none(arg_operands);

                // When elem_ty is Any (e.g., `li = []` without annotation), the list was
                // created with ELEM_HEAP_OBJ. If the actual value is Int, we need to
                // update the list's elem_tag to ELEM_RAW_INT before storing the raw value.
                let actual_value_ty = arg_types.first();
                if *elem_ty == Type::Any {
                    if let Some(Type::Int) = actual_value_ty {
                        let dummy = self.alloc_and_add_local(Type::None, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: dummy,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_SET_ELEM_TAG,
                            ),
                            args: vec![
                                obj_operand.clone(),
                                mir::Operand::Constant(mir::Constant::Int(1)), // ELEM_RAW_INT
                            ],
                        });
                    }
                }

                // Box the value if the element type requires it
                // Bool and Float elements are stored as boxed objects (ELEM_HEAP_OBJ)
                let push_operand = match &*elem_ty {
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_BOOL,
                            ),
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT,
                            ),
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    _ => value_operand,
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_APPEND,
                    ),
                    args: vec![obj_operand, push_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "pop" => {
                // .pop(index=-1) - removes and returns element at index
                // Default index is -1 (last element)
                let index_arg = if arg_operands.is_empty() {
                    mir::Operand::Constant(mir::Constant::Int(-1))
                } else {
                    arg_operands
                        .into_iter()
                        .next()
                        .expect("list.pop requires at least one argument if not empty")
                };

                // ListPop returns *mut Obj for Bool/Float (boxed), need to unbox
                match &*elem_ty {
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_POP,
                            ),
                            args: vec![obj_operand, index_arg],
                        });
                        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_BOOL,
                            ),
                            args: vec![mir::Operand::Local(boxed_local)],
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    Type::Float => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_POP,
                            ),
                            args: vec![obj_operand, index_arg],
                        });
                        let result_local = self.alloc_and_add_local(Type::Float, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                            ),
                            args: vec![mir::Operand::Local(boxed_local)],
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    _ => {
                        let result_local = self.alloc_and_add_local((*elem_ty).clone(), mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_LIST_POP,
                            ),
                            args: vec![obj_operand, index_arg],
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                }
            }
            "insert" => {
                // .insert(index, value) - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                // arg_operands[0] = index, arg_operands[1] = value
                // Box the value if the element type requires it (Bool/Float stored as boxed)
                let mut args_iter = arg_operands.into_iter();
                let index_operand = args_iter
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::Int(0)));
                let value_operand = args_iter
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));

                let boxed_value = match &*elem_ty {
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_BOOL,
                            ),
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT,
                            ),
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    _ => value_operand,
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_INSERT,
                    ),
                    args: vec![obj_operand, index_operand, boxed_value],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "remove" => {
                // .remove(value) - mutates list, returns None (or 1/0 internally)
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                // Box the search value if the element type requires it (Bool/Float)
                let value_operand = crate::first_arg_or_none(arg_operands);

                let boxed_value = match &*elem_ty {
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_BOOL,
                            ),
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT,
                            ),
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    _ => value_operand,
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_REMOVE,
                    ),
                    args: vec![obj_operand, boxed_value],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "clear" => {
                // .clear() - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_CLEAR),
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "index" => {
                // .index(value) - returns int index or -1 if not found
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                let mut all_args = vec![obj_operand];
                all_args.extend(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_INDEX),
                    args: all_args,
                });

                Ok(mir::Operand::Local(result_local))
            }
            "count" => {
                // .count(value) - returns int count
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                let mut all_args = vec![obj_operand];
                all_args.extend(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_COUNT),
                    args: all_args,
                });

                Ok(mir::Operand::Local(result_local))
            }
            "copy" => {
                // .copy() - returns new list (shallow copy)
                let result_local = self.alloc_and_add_local(Type::List(elem_ty.clone()), mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_COPY),
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "reverse" => {
                // .reverse() - mutates list in place, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_REVERSE,
                    ),
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "extend" => {
                // .extend(iterable) - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_LIST_EXTEND,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "sort" => {
                // CPython signature: list.sort(*, key=None, reverse=False)
                // All arguments are keyword-only; positional args are not allowed
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                // Reject positional arguments (CPython behavior)
                if !arg_operands.is_empty() {
                    return Err(CompilerError::type_error(
                        "list.sort() takes no positional arguments",
                        self.call_span(),
                    ));
                }

                // Validate unknown kwargs
                for kw in kwargs {
                    let kw_name = self.resolve(kw.name);
                    if kw_name != "key" && kw_name != "reverse" {
                        return Err(CompilerError::type_error(
                            format!(
                                "list.sort() got an unexpected keyword argument '{}'",
                                kw_name
                            ),
                            kw.span,
                        ));
                    }
                }

                // Use shared helper to extract sort kwargs
                let sort_kwargs = self.extract_sort_kwargs(kwargs, hir_module, mir_func)?;

                // If key function is provided, use ListSortWithKey
                if let Some(resolved) = self.emit_key_func_with_captures(
                    sort_kwargs.key_func.as_ref(),
                    hir_module,
                    mir_func,
                )? {
                    // Determine elem_tag for boxing raw elements before calling key function.
                    // Only builtin wrappers need boxing - user functions work with raw values.
                    let elem_tag = sort_kwargs
                        .key_func
                        .as_ref()
                        .map(|kf| Self::elem_tag_for_key_func(kf, &elem_ty))
                        .unwrap_or(0);
                    let elem_tag_operand = mir::Operand::Constant(mir::Constant::Int(elem_tag));

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_LIST_SORT_WITH_KEY,
                        ),
                        args: vec![
                            obj_operand,
                            sort_kwargs.reverse,
                            resolved.func_addr,
                            elem_tag_operand,
                            resolved.captures,
                            resolved.capture_count,
                        ],
                    });
                } else {
                    // No key function - use standard ListSort
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_LIST_SORT,
                        ),
                        args: vec![obj_operand, sort_kwargs.reverse],
                    });
                }

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // Unknown list method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
