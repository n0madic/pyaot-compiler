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
                // After §F.7c: list slots store uniform tagged Values; box every primitive.
                let value_operand = crate::first_arg_or_none(arg_operands);
                let actual_value_ty = arg_types.first().cloned().unwrap_or(Type::Any);
                let box_type = if matches!(*elem_ty, Type::Union(_) | Type::Any) {
                    actual_value_ty
                } else {
                    (*elem_ty).clone()
                };
                let push_operand = self.box_primitive_if_needed(value_operand, &box_type, mir_func);

                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_APPEND),
                    vec![obj_operand, push_operand],
                    Type::None,
                    mir_func,
                );

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
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_POP),
                            vec![obj_operand, index_arg],
                            Type::HeapAny,
                            mir_func,
                        );
                        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnwrapValueBool {
                            dest: result_local,
                            src: mir::Operand::Local(boxed_local),
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    Type::Float => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_POP),
                            vec![obj_operand, index_arg],
                            Type::HeapAny,
                            mir_func,
                        );
                        let result_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT,
                            ),
                            vec![mir::Operand::Local(boxed_local)],
                            Type::Float,
                            mir_func,
                        );
                        Ok(mir::Operand::Local(result_local))
                    }
                    Type::Int => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_POP),
                            vec![obj_operand, index_arg],
                            Type::HeapAny,
                            mir_func,
                        );
                        let result_local = self.alloc_and_add_local(Type::Int, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnwrapValueInt {
                            dest: result_local,
                            src: mir::Operand::Local(boxed_local),
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    _ => {
                        let result_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_POP),
                            vec![obj_operand, index_arg],
                            (*elem_ty).clone(),
                            mir_func,
                        );
                        Ok(mir::Operand::Local(result_local))
                    }
                }
            }
            "insert" => {
                // .insert(index, value) - mutates list, returns None
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
                    Type::Int => {
                        let tagged_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::ValueFromInt {
                            dest: tagged_local,
                            src: value_operand,
                        });
                        mir::Operand::Local(tagged_local)
                    }
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::ValueFromBool {
                            dest: boxed_local,
                            src: value_operand,
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT,
                            ),
                            vec![value_operand],
                            Type::HeapAny,
                            mir_func,
                        );
                        mir::Operand::Local(boxed_local)
                    }
                    _ => value_operand,
                };

                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_INSERT),
                    vec![obj_operand, index_operand, boxed_value],
                    Type::None,
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "remove" => {
                // .remove(value): box search value (slots are tagged Values post-§F.7c).
                let value_operand = crate::first_arg_or_none(arg_operands);
                let actual_ty = arg_types.first().cloned().unwrap_or(Type::Any);
                let box_type = if matches!(*elem_ty, Type::Union(_) | Type::Any) {
                    actual_ty
                } else {
                    (*elem_ty).clone()
                };
                let boxed_value = self.box_primitive_if_needed(value_operand, &box_type, mir_func);

                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_REMOVE),
                    vec![obj_operand, boxed_value],
                    Type::None,
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "clear" => {
                // .clear() - mutates list, returns None
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_CLEAR),
                    vec![obj_operand],
                    Type::None,
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "index" => {
                // .index(value): box the search value because slots are tagged Values.
                let value_operand = crate::first_arg_or_none(arg_operands);
                let actual_ty = arg_types.first().cloned().unwrap_or(Type::Any);
                let box_type = if matches!(*elem_ty, Type::Union(_) | Type::Any) {
                    actual_ty
                } else {
                    (*elem_ty).clone()
                };
                let boxed_value = self.box_primitive_if_needed(value_operand, &box_type, mir_func);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_INDEX),
                    vec![obj_operand, boxed_value],
                    Type::Int,
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "count" => {
                // .count(value): box the search value (slots are tagged Values).
                let value_operand = crate::first_arg_or_none(arg_operands);
                let actual_ty = arg_types.first().cloned().unwrap_or(Type::Any);
                let box_type = if matches!(*elem_ty, Type::Union(_) | Type::Any) {
                    actual_ty
                } else {
                    (*elem_ty).clone()
                };
                let boxed_value = self.box_primitive_if_needed(value_operand, &box_type, mir_func);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_COUNT),
                    vec![obj_operand, boxed_value],
                    Type::Int,
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "copy" => {
                // .copy() - returns new list (shallow copy)
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_COPY),
                    vec![obj_operand],
                    Type::List(elem_ty.clone()),
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "reverse" => {
                // .reverse() - mutates list in place, returns None
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_REVERSE),
                    vec![obj_operand],
                    Type::None,
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "extend" => {
                // .extend(iterable) - mutates list, returns None
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_EXTEND),
                    vec![obj_operand, other_arg],
                    Type::None,
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "sort" => {
                // CPython signature: list.sort(*, key=None, reverse=False)
                // All arguments are keyword-only; positional args are not allowed

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
                let result_local = if let Some(resolved) = self.emit_key_func_with_captures(
                    sort_kwargs.key_func.as_ref(),
                    hir_module,
                    mir_func,
                )? {
                    // After §F.7c: list slots are tagged Values; runtime no
                    // longer needs an elem_tag hint.
                    self.emit_runtime_call(
                        mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_LIST_SORT_WITH_KEY,
                        ),
                        vec![
                            obj_operand,
                            sort_kwargs.reverse,
                            resolved.func_addr,
                            resolved.captures,
                            resolved.capture_count,
                            resolved.key_return_tag,
                        ],
                        Type::None,
                        mir_func,
                    )
                } else {
                    // No key function - use standard ListSort
                    self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_SORT),
                        vec![obj_operand, sort_kwargs.reverse],
                        Type::None,
                        mir_func,
                    )
                };

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // Unknown list method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
