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
        kwargs: &[hir::KeywordArg],
        elem_ty: Box<Type>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match method_name {
            "append" => {
                // .append(value) - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                // Get the value operand
                let value_operand = arg_operands
                    .into_iter()
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));

                // Box the value if the element type requires it
                // Bool and Float elements are stored as boxed objects (ELEM_HEAP_OBJ)
                let push_operand = match &*elem_ty {
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::BoxBool,
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    Type::Float => {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::BoxFloat,
                            args: vec![value_operand],
                        });
                        mir::Operand::Local(boxed_local)
                    }
                    _ => value_operand,
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListAppend,
                    args: vec![obj_operand, push_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "pop" => {
                // .pop(index=-1) - removes and returns element at index
                let result_local = self.alloc_and_add_local((*elem_ty).clone(), mir_func);

                // Default index is -1 (last element)
                let index_arg = if arg_operands.is_empty() {
                    mir::Operand::Constant(mir::Constant::Int(-1))
                } else {
                    arg_operands
                        .into_iter()
                        .next()
                        .expect("list.pop requires at least one argument if not empty")
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListPop,
                    args: vec![obj_operand, index_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "insert" => {
                // .insert(index, value) - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let mut all_args = vec![obj_operand];
                all_args.extend(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListInsert,
                    args: all_args,
                });

                Ok(mir::Operand::Local(result_local))
            }
            "remove" => {
                // .remove(value) - mutates list, returns None (or 1/0 internally)
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let mut all_args = vec![obj_operand];
                all_args.extend(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListRemove,
                    args: all_args,
                });

                Ok(mir::Operand::Local(result_local))
            }
            "clear" => {
                // .clear() - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListClear,
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
                    func: mir::RuntimeFunc::ListIndex,
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
                    func: mir::RuntimeFunc::ListCount,
                    args: all_args,
                });

                Ok(mir::Operand::Local(result_local))
            }
            "copy" => {
                // .copy() - returns new list (shallow copy)
                let result_local = self.alloc_and_add_local(Type::List(elem_ty.clone()), mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListCopy,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "reverse" => {
                // .reverse() - mutates list in place, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListReverse,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "extend" => {
                // .extend(iterable) - mutates list, returns None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = arg_operands
                    .into_iter()
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::ListExtend,
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
                        pyaot_utils::Span::dummy(),
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
                if let Some(key_operand) =
                    self.emit_key_func_addr(sort_kwargs.key_func.as_ref(), mir_func)
                {
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
                        func: mir::RuntimeFunc::ListSortWithKey,
                        args: vec![
                            obj_operand,
                            sort_kwargs.reverse,
                            key_operand,
                            elem_tag_operand,
                        ],
                    });
                } else {
                    // No key function - use standard ListSort
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::ListSort,
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
