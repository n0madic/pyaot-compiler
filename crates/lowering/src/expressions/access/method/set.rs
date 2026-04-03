//! Set method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower set method calls.
    pub(super) fn lower_set_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        arg_types: Vec<Type>,
        elem_ty: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match method_name {
            "add" => {
                // .add(elem) - adds element to set
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let elem_arg = crate::first_arg_or_none(arg_operands);
                // Use actual argument type for boxing decision
                let elem_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_elem = self.box_primitive_if_needed(elem_arg, &elem_type, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_ADD),
                    args: vec![obj_operand, boxed_elem],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "remove" => {
                // .remove(elem) - removes element, raises KeyError if missing
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let elem_arg = crate::first_arg_or_none(arg_operands);
                let elem_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_elem = self.box_primitive_if_needed(elem_arg, &elem_type, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_REMOVE),
                    args: vec![obj_operand, boxed_elem],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "discard" => {
                // .discard(elem) - removes element if present, no error if missing
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let elem_arg = crate::first_arg_or_none(arg_operands);
                let elem_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_elem = self.box_primitive_if_needed(elem_arg, &elem_type, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_DISCARD,
                    ),
                    args: vec![obj_operand, boxed_elem],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "clear" => {
                // .clear() - removes all elements
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_CLEAR),
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "copy" => {
                // .copy() - shallow copy, preserving the source set's element type
                let result_local =
                    self.alloc_and_add_local(Type::Set(Box::new(elem_ty.clone())), mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_COPY),
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "union" => {
                // .union(other) - returns new set with elements from both sets
                let result_local =
                    self.alloc_and_add_local(Type::Set(Box::new(Type::Any)), mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_UNION),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "intersection" => {
                // .intersection(other) - returns new set with elements in both sets
                let result_local =
                    self.alloc_and_add_local(Type::Set(Box::new(Type::Any)), mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_INTERSECTION,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "difference" => {
                // .difference(other) - returns new set with elements in self but not other
                let result_local =
                    self.alloc_and_add_local(Type::Set(Box::new(Type::Any)), mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_DIFFERENCE,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "symmetric_difference" => {
                // .symmetric_difference(other) - returns new set with elements in exactly one set
                let result_local =
                    self.alloc_and_add_local(Type::Set(Box::new(Type::Any)), mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_SYMMETRIC_DIFFERENCE,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "issubset" => {
                // .issubset(other) - returns True if all elements in self are in other
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_ISSUBSET,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "issuperset" => {
                // .issuperset(other) - returns True if all elements in other are in self
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_ISSUPERSET,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "isdisjoint" => {
                // .isdisjoint(other) - returns True if no elements in common
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_ISDISJOINT,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "pop" => {
                // .pop() - removes and returns an arbitrary element
                // SetPop returns a boxed *mut Obj; unbox based on element type
                match elem_ty {
                    Type::Bool => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_SET_POP,
                            ),
                            args: vec![obj_operand],
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
                                &pyaot_core_defs::runtime_func_def::RT_SET_POP,
                            ),
                            args: vec![obj_operand],
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
                    Type::Int => {
                        let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: boxed_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_SET_POP,
                            ),
                            args: vec![obj_operand],
                        });
                        let result_local = self.alloc_and_add_local(Type::Int, mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_UNBOX_INT,
                            ),
                            args: vec![mir::Operand::Local(boxed_local)],
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    _ => {
                        // Heap types (Str, Tuple, etc.) — no unboxing needed
                        let result_local = self.alloc_and_add_local(elem_ty.clone(), mir_func);
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_SET_POP,
                            ),
                            args: vec![obj_operand],
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                }
            }
            "update" => {
                // .update(other) - adds all elements from other
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_UPDATE),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "intersection_update" => {
                // .intersection_update(other) - keeps only elements also in other
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_INTERSECTION_UPDATE,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "difference_update" => {
                // .difference_update(other) - removes elements also in other
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_DIFFERENCE_UPDATE,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "symmetric_difference_update" => {
                // .symmetric_difference_update(other) - keeps elements in exactly one set
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                let other_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_SYMMETRIC_DIFFERENCE_UPDATE,
                    ),
                    args: vec![obj_operand, other_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // Unknown set method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
