//! Augmented assignment and delete lowering
//!
//! Handles: IndexAssign (obj[i] = v), FieldAssign (obj.f = v),
//!          ClassAttrAssign (Cls.attr = v), IndexDelete (del obj[i])

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{ClassId, InternedString};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower an index assignment: obj[index] = value
    pub(crate) fn lower_index_assign(
        &mut self,
        obj: hir::ExprId,
        index: hir::ExprId,
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_type_of_expr_id(obj, hir_module);

        let index_expr = &hir_module.exprs[index];
        let index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;
        let index_type = self.get_type_of_expr_id(index, hir_module);

        let value_expr = &hir_module.exprs[value];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;
        let value_type = self.get_type_of_expr_id(value, hir_module);

        // Create a dummy local for void returns
        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        match obj_type {
            Type::Dict(ref key_ty, ref val_ty) => {
                // Refine Dict(Any, Any) type based on actual key/value types
                // This happens with dict comprehensions where the initial empty dict has unknown types
                if **key_ty == Type::Any || **val_ty == Type::Any {
                    if let hir::ExprKind::Var(var_id) = &obj_expr.kind {
                        let refined_key = if **key_ty == Type::Any && index_type != Type::Any {
                            Box::new(index_type.clone())
                        } else {
                            key_ty.clone()
                        };
                        let refined_val = if **val_ty == Type::Any && value_type != Type::Any {
                            Box::new(value_type.clone())
                        } else {
                            val_ty.clone()
                        };
                        self.insert_var_type(*var_id, Type::Dict(refined_key, refined_val));
                    }
                }

                // dict[key] = value - box key and value if needed (primitives must be boxed for GC)
                let boxed_key = self.box_primitive_if_needed(index_operand, &index_type, mir_func);
                let boxed_value =
                    self.box_primitive_if_needed(value_operand, &value_type, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                    args: vec![obj_operand, boxed_key, boxed_value],
                });
            }
            Type::DefaultDict(ref _key_ty, ref val_ty) => {
                // defaultdict[key] = value — same as dict assignment, uses DictSet
                let boxed_key = self.box_primitive_if_needed(index_operand, &index_type, mir_func);
                // For augmented assignment, value_type may be Any even though the actual
                // value is a primitive. Use the defaultdict's value type for boxing decision.
                let box_type = if value_type == Type::Any {
                    val_ty.as_ref()
                } else {
                    &value_type
                };
                let boxed_value = self.box_primitive_if_needed(value_operand, box_type, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_SET),
                    args: vec![obj_operand, boxed_key, boxed_value],
                });
            }
            Type::List(ref elem_ty) => {
                // list[index] = value
                // Box float/bool values before storing (lists use ELEM_HEAP_OBJ for these types)
                let store_operand = if **elem_ty == Type::Float {
                    let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: boxed_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT,
                        ),
                        args: vec![value_operand],
                    });
                    mir::Operand::Local(boxed_local)
                } else if **elem_ty == Type::Bool {
                    let boxed_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: boxed_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BOX_BOOL,
                        ),
                        args: vec![value_operand],
                    });
                    mir::Operand::Local(boxed_local)
                } else {
                    value_operand
                };
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_SET),
                    args: vec![obj_operand, index_operand, store_operand],
                });
            }
            Type::Class { class_id, .. } => {
                // Class with __setitem__ dunder
                let setitem_func = self
                    .get_class_info(&class_id)
                    .and_then(|info| info.get_dunder_func("__setitem__"));

                if let Some(func_id) = setitem_func {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: func_id,
                        args: vec![obj_operand, index_operand, value_operand],
                    });
                }
            }
            _ => {
                // Unsupported type for indexed assignment
            }
        }

        Ok(())
    }

    /// Lower a delete indexed item: del obj[key]
    /// Uses DictPop for dicts and ListPop for lists (discarding the result).
    pub(crate) fn lower_index_delete(
        &mut self,
        obj: hir::ExprId,
        index: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_type_of_expr_id(obj, hir_module);

        let index_expr = &hir_module.exprs[index];
        let index_operand = self.lower_expr(index_expr, hir_module, mir_func)?;
        let index_type = self.get_type_of_expr_id(index, hir_module);

        // Create a dummy local for the discarded return value
        // Use Type::Any (i64) since DictPop/ListPop return heap pointers
        let dummy_local = self.alloc_and_add_local(Type::Any, mir_func);

        match obj_type {
            Type::Dict(_, _) | Type::DefaultDict(_, _) => {
                // del dict[key] → rt_dict_pop(dict, key) and discard result
                let boxed_key = self.box_primitive_if_needed(index_operand, &index_type, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_POP),
                    args: vec![obj_operand, boxed_key],
                });
            }
            Type::List(_) => {
                // del list[index] → rt_list_pop(list, index) and discard result
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_POP),
                    args: vec![obj_operand, index_operand],
                });
            }
            Type::Class { class_id, .. } => {
                // Class with __delitem__ dunder
                let delitem_func = self
                    .get_class_info(&class_id)
                    .and_then(|info| info.get_dunder_func("__delitem__"));

                if let Some(func_id) = delitem_func {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: func_id,
                        args: vec![obj_operand, index_operand],
                    });
                }
            }
            _ => {
                // Unsupported type for indexed delete
            }
        }

        Ok(())
    }

    /// Lower a field assignment: obj.field = value
    /// Also handles @property setters.
    pub(crate) fn lower_field_assign(
        &mut self,
        obj: hir::ExprId,
        field: InternedString,
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_type_of_expr_id(obj, hir_module);

        let value_expr = &hir_module.exprs[value];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;

        // Look up field offset from class info
        if let Type::Class { class_id, .. } = &obj_type {
            if let Some(class_info) = self.get_class_info(class_id).cloned() {
                // 1. Check for @property setter first
                if let Some((_getter, Some(setter_id))) = class_info.properties.get(&field) {
                    let setter_id = *setter_id;
                    // Create a dummy local for void return
                    let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Call the setter with (self, value)
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: dummy_local,
                        func: setter_id,
                        args: vec![obj_operand, value_operand],
                    });

                    return Ok(());
                }

                // 2. Regular field assignment
                if let Some(&offset) = class_info.field_offsets.get(&field) {
                    // Create a dummy local for void return
                    let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Set the field value
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_INSTANCE_SET_FIELD,
                        ),
                        args: vec![
                            obj_operand,
                            mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                            value_operand,
                        ],
                    });
                    return Ok(());
                }

                // 3. Fallback to class attribute assignment (Python: instance.class_attr = value)
                if let (Some(&(owning_class_id, attr_offset)), Some(attr_type)) = (
                    class_info.class_attr_offsets.get(&field),
                    class_info.class_attr_types.get(&field).cloned(),
                ) {
                    let dummy_local = self.alloc_and_add_local(Type::None, mir_func);
                    let set_func = self.get_class_attr_set_func(&attr_type);
                    let effective_class_id = self.get_effective_class_id(owning_class_id);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: set_func,
                        args: vec![
                            mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                            mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                            value_operand,
                        ],
                    });
                    return Ok(());
                }
            }
        }

        Ok(())
    }

    /// Lower a class attribute assignment: ClassName.attr = value
    pub(crate) fn lower_class_attr_assign(
        &mut self,
        class_id: ClassId,
        attr: InternedString,
        value: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let value_expr = &hir_module.exprs[value];
        let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;

        // Look up class attribute (owning_class_id, offset) and type from class info
        // The owning_class_id is the class where the attribute was actually defined
        if let Some(class_info) = self.get_class_info(&class_id) {
            if let (Some(&(owning_class_id, attr_offset)), Some(attr_type)) = (
                class_info.class_attr_offsets.get(&attr),
                class_info.class_attr_types.get(&attr).cloned(),
            ) {
                // Create a dummy local for void return
                let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

                // Get the appropriate runtime function based on type
                let set_func = self.get_class_attr_set_func(&attr_type);

                // Emit runtime call: rt_class_attr_set_*(owning_class_id, attr_idx, value)
                // Use the owning_class_id, not the accessed class_id, to handle inheritance
                let effective_class_id = self.get_effective_class_id(owning_class_id);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: set_func,
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                        value_operand,
                    ],
                });
            }
        }

        Ok(())
    }
}
