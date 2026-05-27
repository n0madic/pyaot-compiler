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
                // .add(elem) - adds element to set; returns None in Python
                let elem_arg = crate::first_arg_or_none(arg_operands);
                // Use actual argument type for boxing decision
                let elem_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_elem = self.emit_value_slot(elem_arg, &elem_type, mir_func);

                self.emit_void_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_ADD),
                    vec![obj_operand, boxed_elem],
                    mir_func,
                );

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "remove" => {
                // .remove(elem) - removes element, raises KeyError if missing; returns None
                let elem_arg = crate::first_arg_or_none(arg_operands);
                let elem_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_elem = self.emit_value_slot(elem_arg, &elem_type, mir_func);

                self.emit_void_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_REMOVE),
                    vec![obj_operand, boxed_elem],
                    mir_func,
                );

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "discard" => {
                // .discard(elem) - removes element if present, no error if missing; returns None
                let elem_arg = crate::first_arg_or_none(arg_operands);
                let elem_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_elem = self.emit_value_slot(elem_arg, &elem_type, mir_func);

                self.emit_void_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_DISCARD),
                    vec![obj_operand, boxed_elem],
                    mir_func,
                );

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "clear" => {
                // .clear() - removes all elements; returns None
                self.emit_void_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_CLEAR),
                    vec![obj_operand],
                    mir_func,
                );

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "copy" => {
                // .copy() - shallow copy, preserving the source set's element type
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_COPY),
                    vec![obj_operand],
                    Type::set_of(elem_ty.clone()),
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "union" => {
                // .union(other) - returns new set with elements from both sets
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_UNION),
                    vec![obj_operand, other_arg],
                    Type::set_of(Type::Any),
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "intersection" => {
                // .intersection(other) - returns new set with elements in both sets
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_INTERSECTION),
                    vec![obj_operand, other_arg],
                    Type::set_of(Type::Any),
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "difference" => {
                // .difference(other) - returns new set with elements in self but not other
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_DIFFERENCE),
                    vec![obj_operand, other_arg],
                    Type::set_of(Type::Any),
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "symmetric_difference" => {
                // .symmetric_difference(other) - returns new set with elements in exactly one set
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_SYMMETRIC_DIFFERENCE,
                    ),
                    vec![obj_operand, other_arg],
                    Type::set_of(Type::Any),
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "issubset" => {
                // .issubset(other) - returns True if all elements in self are in other
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_ISSUBSET),
                    vec![obj_operand, other_arg],
                    Type::Bool,
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "issuperset" => {
                // .issuperset(other) - returns True if all elements in other are in self
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_ISSUPERSET),
                    vec![obj_operand, other_arg],
                    Type::Bool,
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "isdisjoint" => {
                // .isdisjoint(other) - returns True if no elements in common
                let other_arg = crate::first_arg_or_none(arg_operands);
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_ISDISJOINT),
                    vec![obj_operand, other_arg],
                    Type::Bool,
                    mir_func,
                );
                Ok(mir::Operand::Local(result_local))
            }
            "pop" => {
                // .pop() - removes and returns an arbitrary element
                // SetPop returns a boxed *mut Obj; unbox based on element type
                match elem_ty {
                    Type::Bool => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_POP),
                            vec![obj_operand],
                            Type::Any,
                            mir_func,
                        );
                        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnboxValue {
                            dest: result_local,
                            src: mir::Operand::Local(boxed_local),
                            dest_type: Type::Bool,
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    Type::Float => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_POP),
                            vec![obj_operand],
                            Type::Any,
                            mir_func,
                        );
                        let result_local = self.alloc_and_add_local(Type::Float, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnboxValue {
                            dest: result_local,
                            src: mir::Operand::Local(boxed_local),
                            dest_type: Type::Float,
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    Type::Int => {
                        let boxed_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_POP),
                            vec![obj_operand],
                            Type::Any,
                            mir_func,
                        );
                        let result_local = self.alloc_and_add_local(Type::Int, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnboxValue {
                            dest: result_local,
                            src: mir::Operand::Local(boxed_local),
                            dest_type: Type::Int,
                        });
                        Ok(mir::Operand::Local(result_local))
                    }
                    _ => {
                        // Heap types (Str, Tuple, etc.) — no unboxing needed
                        let result_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_POP),
                            vec![obj_operand],
                            elem_ty.clone(),
                            mir_func,
                        );
                        Ok(mir::Operand::Local(result_local))
                    }
                }
            }
            "update" => {
                // .update(other) - adds all elements from other; returns None
                let other_arg = crate::first_arg_or_none(arg_operands);
                self.emit_void_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_SET_UPDATE),
                    vec![obj_operand, other_arg],
                    mir_func,
                );
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "intersection_update" => {
                // .intersection_update(other) - keeps only elements also in other; returns None
                let other_arg = crate::first_arg_or_none(arg_operands);
                self.emit_void_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_INTERSECTION_UPDATE,
                    ),
                    vec![obj_operand, other_arg],
                    mir_func,
                );
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "difference_update" => {
                // .difference_update(other) - removes elements also in other; returns None
                let other_arg = crate::first_arg_or_none(arg_operands);
                self.emit_void_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_DIFFERENCE_UPDATE,
                    ),
                    vec![obj_operand, other_arg],
                    mir_func,
                );
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "symmetric_difference_update" => {
                // .symmetric_difference_update(other) - keeps elements in exactly one set; returns None
                let other_arg = crate::first_arg_or_none(arg_operands);
                self.emit_void_call(
                    mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_SET_SYMMETRIC_DIFFERENCE_UPDATE,
                    ),
                    vec![obj_operand, other_arg],
                    mir_func,
                );
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            _ => {
                // Unknown set method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
