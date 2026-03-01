//! Dict method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower dict method calls.
    pub(super) fn lower_dict_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        key_ty: Box<Type>,
        value_ty: Box<Type>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Determine if value type needs unboxing
        let unbox_func = match value_ty.as_ref() {
            Type::Int => Some(mir::RuntimeFunc::UnboxInt),
            Type::Float => Some(mir::RuntimeFunc::UnboxFloat),
            Type::Bool => Some(mir::RuntimeFunc::UnboxBool),
            _ => None,
        };

        match method_name {
            "get" => {
                // .get(key) or .get(key, default)
                // Dict values are stored as boxed pointers, so we need to unbox primitives
                let result_local = self.alloc_and_add_local((*value_ty).clone(), mir_func);

                if arg_operands.len() >= 2 {
                    // .get(key, default) - box key and default based on dict's types
                    let boxed_key =
                        self.box_dict_key_if_needed(arg_operands[0].clone(), &key_ty, mir_func);
                    let boxed_default =
                        self.box_dict_value_if_needed(arg_operands[1].clone(), &value_ty, mir_func);

                    if let Some(unbox_func) = unbox_func {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::DictGetDefault,
                            args: vec![obj_operand, boxed_key, boxed_default],
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: unbox_func,
                            args: vec![mir::Operand::Local(boxed_local)],
                        });
                    } else {
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::DictGetDefault,
                            args: vec![obj_operand, boxed_key, boxed_default],
                        });
                    }
                } else {
                    // .get(key) - returns None if not found
                    let key_arg = arg_operands
                        .into_iter()
                        .next()
                        .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                    let boxed_key = self.box_dict_key_if_needed(key_arg, &key_ty, mir_func);

                    if let Some(unbox_func) = unbox_func {
                        let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::DictGet,
                            args: vec![obj_operand, boxed_key],
                        });
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: unbox_func,
                            args: vec![mir::Operand::Local(boxed_local)],
                        });
                    } else {
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::DictGet,
                            args: vec![obj_operand, boxed_key],
                        });
                    }
                }

                Ok(mir::Operand::Local(result_local))
            }
            "pop" => {
                // .pop(key) - removes and returns value
                let result_local = self.alloc_and_add_local((*value_ty).clone(), mir_func);

                let key_arg = arg_operands
                    .into_iter()
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                let boxed_key = self.box_dict_key_if_needed(key_arg, &key_ty, mir_func);

                if let Some(unbox_func) = unbox_func {
                    let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: boxed_local,
                        func: mir::RuntimeFunc::DictPop,
                        args: vec![obj_operand, boxed_key],
                    });
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: unbox_func,
                        args: vec![mir::Operand::Local(boxed_local)],
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::DictPop,
                        args: vec![obj_operand, boxed_key],
                    });
                }

                Ok(mir::Operand::Local(result_local))
            }
            "clear" => {
                // .clear() - removes all items
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictClear,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "copy" => {
                // .copy() - shallow copy
                let result_local = self
                    .alloc_and_add_local(Type::Dict(key_ty.clone(), value_ty.clone()), mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictCopy,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "keys" => {
                // .keys() - returns list of keys
                let result_local = self.alloc_and_add_local(Type::List(key_ty.clone()), mir_func);
                let key_elem_tag = Self::elem_tag_for_type(&key_ty);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictKeys,
                    args: vec![
                        obj_operand,
                        mir::Operand::Constant(mir::Constant::Int(key_elem_tag)),
                    ],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "values" => {
                // .values() - returns list of values
                let result_local = self.alloc_and_add_local(Type::List(value_ty.clone()), mir_func);
                let value_elem_tag = Self::elem_tag_for_type(&value_ty);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictValues,
                    args: vec![
                        obj_operand,
                        mir::Operand::Constant(mir::Constant::Int(value_elem_tag)),
                    ],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "items" => {
                // .items() - returns list of (key, value) tuples
                let tuple_ty = Type::Tuple(vec![(*key_ty).clone(), (*value_ty).clone()]);
                let result_local =
                    self.alloc_and_add_local(Type::List(Box::new(tuple_ty)), mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictItems,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "update" => {
                // .update(other) - merges another dict
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = arg_operands
                    .into_iter()
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictUpdate,
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "setdefault" => {
                // .setdefault(key, default=None) - returns value if key exists, else sets default
                let result_local = self.alloc_and_add_local((*value_ty).clone(), mir_func);

                let key_arg = arg_operands
                    .first()
                    .cloned()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                let boxed_key = self.box_dict_key_if_needed(key_arg, &key_ty, mir_func);

                let default_arg = arg_operands
                    .get(1)
                    .cloned()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                let boxed_default = self.box_dict_value_if_needed(default_arg, &value_ty, mir_func);

                if let Some(unbox_func) = unbox_func {
                    let boxed_local = self.alloc_and_add_local(Type::Str, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: boxed_local,
                        func: mir::RuntimeFunc::DictSetDefault,
                        args: vec![obj_operand, boxed_key, boxed_default],
                    });
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: unbox_func,
                        args: vec![mir::Operand::Local(boxed_local)],
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::DictSetDefault,
                        args: vec![obj_operand, boxed_key, boxed_default],
                    });
                }

                Ok(mir::Operand::Local(result_local))
            }
            "popitem" => {
                // .popitem() - removes and returns an arbitrary (key, value) pair as tuple
                let tuple_ty = Type::Tuple(vec![(*key_ty).clone(), (*value_ty).clone()]);
                let result_local = self.alloc_and_add_local(tuple_ty, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictPopItem,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "fromkeys" => {
                // .fromkeys(keys, value=None) - creates dict from keys with same value
                let result_local = self
                    .alloc_and_add_local(Type::Dict(key_ty.clone(), value_ty.clone()), mir_func);

                let keys_arg = arg_operands
                    .first()
                    .cloned()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                let value_arg = arg_operands
                    .get(1)
                    .cloned()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));

                // Box value if needed
                let boxed_value = self.box_dict_value_if_needed(value_arg, &value_ty, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::DictFromKeys,
                    args: vec![keys_arg, boxed_value],
                });

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // Unknown dict method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
