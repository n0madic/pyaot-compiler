//! Dict method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::LocalId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Emit a runtime call and optionally unbox the result for primitive dict values.
    /// Returns the final result local (unboxed if needed).
    fn emit_dict_call_and_unbox(
        &mut self,
        result_local: LocalId,
        unbox_func: Option<mir::RuntimeFunc>,
        call_func: mir::RuntimeFunc,
        args: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) {
        if let Some(unbox_func) = unbox_func {
            let boxed_local = self.emit_runtime_call(call_func, args, Type::HeapAny, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: unbox_func,
                args: vec![mir::Operand::Local(boxed_local)],
            });
        } else {
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: call_func,
                args,
            });
        }
    }

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
        let unbox_func = Self::unbox_func_for_type(value_ty.as_ref());

        match method_name {
            "get" => {
                // .get(key) or .get(key, default)
                // Use DictGetDefault for both cases (avoids KeyError on missing keys)
                let result_local = self.alloc_and_add_local((*value_ty).clone(), mir_func);

                if arg_operands.len() >= 2 {
                    let boxed_key =
                        self.box_primitive_if_needed(arg_operands[0].clone(), &key_ty, mir_func);
                    let boxed_default =
                        self.box_primitive_if_needed(arg_operands[1].clone(), &value_ty, mir_func);
                    self.emit_dict_call_and_unbox(
                        result_local,
                        unbox_func,
                        mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DICT_GET_DEFAULT,
                        ),
                        vec![obj_operand, boxed_key, boxed_default],
                        mir_func,
                    );
                } else {
                    let key_arg = arg_operands
                        .into_iter()
                        .next()
                        .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                    let boxed_key = self.box_primitive_if_needed(key_arg, &key_ty, mir_func);
                    // Use Int(0) as null pointer for default (None is i8, but
                    // rt_dict_get_default expects i64 for the default parameter)
                    self.emit_dict_call_and_unbox(
                        result_local,
                        unbox_func,
                        mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_DICT_GET_DEFAULT,
                        ),
                        vec![
                            obj_operand,
                            boxed_key,
                            mir::Operand::Constant(mir::Constant::Int(0)),
                        ],
                        mir_func,
                    );
                }

                Ok(mir::Operand::Local(result_local))
            }
            "pop" => {
                // .pop(key) - removes and returns value
                let result_local = self.alloc_and_add_local((*value_ty).clone(), mir_func);

                let key_arg = crate::first_arg_or_none(arg_operands);
                let boxed_key = self.box_primitive_if_needed(key_arg, &key_ty, mir_func);
                self.emit_dict_call_and_unbox(
                    result_local,
                    unbox_func,
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_POP),
                    vec![obj_operand, boxed_key],
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "clear" => {
                // .clear() - removes all items
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_CLEAR),
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
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_COPY),
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
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_KEYS),
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
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_DICT_VALUES,
                    ),
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
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_ITEMS),
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "update" => {
                // .update(other) - merges another dict
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_DICT_UPDATE,
                    ),
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
                let boxed_key = self.box_primitive_if_needed(key_arg, &key_ty, mir_func);

                let default_arg = arg_operands
                    .get(1)
                    .cloned()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                let boxed_default = self.box_primitive_if_needed(default_arg, &value_ty, mir_func);

                self.emit_dict_call_and_unbox(
                    result_local,
                    unbox_func,
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET_DEFAULT),
                    vec![obj_operand, boxed_key, boxed_default],
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "popitem" => {
                // .popitem() or .popitem(last=True/False)
                // Use rt_dict_popitem_ordered which supports the `last` parameter
                let tuple_ty = Type::Tuple(vec![(*key_ty).clone(), (*value_ty).clone()]);
                let result_local = self.alloc_and_add_local(tuple_ty, mir_func);

                let last_arg = if !arg_operands.is_empty() {
                    arg_operands[0].clone()
                } else {
                    mir::Operand::Constant(mir::Constant::Int(1)) // default: last=True
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_stdlib_defs::modules::collections::ORDERED_DICT_POPITEM_FUNC.codegen,
                    ),
                    args: vec![obj_operand, last_arg],
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
                let boxed_value = self.box_primitive_if_needed(value_arg, &value_ty, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_DICT_FROM_KEYS,
                    ),
                    args: vec![keys_arg, boxed_value],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "move_to_end" => {
                // OrderedDict.move_to_end(key, last=True) — also works on regular dicts
                let result_local = self.alloc_and_add_local(Type::None, mir_func);
                let key_arg = crate::first_arg_or_none(arg_operands.clone());
                let boxed_key = self.box_primitive_if_needed(key_arg, &key_ty, mir_func);
                let last_arg = if arg_operands.len() >= 2 {
                    arg_operands[1].clone()
                } else {
                    mir::Operand::Constant(mir::Constant::Int(1)) // default: last=True
                };
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_stdlib_defs::modules::collections::ORDERED_DICT_MOVE_TO_END_FUNC
                            .codegen,
                    ),
                    args: vec![obj_operand, boxed_key, last_arg],
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
