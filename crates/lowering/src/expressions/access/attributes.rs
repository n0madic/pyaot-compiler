//! Attribute access and super call lowering

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_stdlib_defs::lookup_object_field;
use pyaot_types::{typespec_to_type, Type};
use pyaot_utils::InternedString;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower an attribute access expression: obj.attr
    pub(in crate::expressions) fn lower_attribute(
        &mut self,
        obj: hir::ExprId,
        attr: InternedString,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Field access: obj.field
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        let obj_type = self.get_type_of_expr_id(obj, hir_module);

        // Handle file attributes
        if matches!(obj_type, Type::File) {
            let attr_name = self.resolve(attr);
            match attr_name {
                "closed" => {
                    let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_FILE_IS_CLOSED,
                        ),
                        args: vec![obj_operand],
                    });

                    return Ok(mir::Operand::Local(result_local));
                }
                "name" => {
                    let result_local = self.alloc_and_add_local(Type::Str, mir_func);

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_FILE_NAME,
                        ),
                        args: vec![obj_operand],
                    });

                    return Ok(mir::Operand::Local(result_local));
                }
                _ => {
                    // Unknown file attribute
                    return Ok(mir::Operand::Constant(mir::Constant::None));
                }
            }
        }

        // Handle runtime object type attributes using generic ObjectFieldGet
        // TypeTagKind comes directly from Type::RuntimeObject - no mapping needed!
        if let Type::RuntimeObject(type_tag) = &obj_type {
            let attr_name = self.resolve(attr);
            if let Some(field_def) = lookup_object_field(*type_tag, attr_name) {
                let result_type = typespec_to_type(&field_def.field_type);
                let result_local = self.alloc_and_add_local(result_type, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(&field_def.codegen),
                    args: vec![obj_operand],
                });

                return Ok(mir::Operand::Local(result_local));
            } else {
                // Unknown field for this object type
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        }

        // Handle __name__ attribute on strings (for type(x).__name__ pattern)
        // type() returns a string like "<class 'int'>", and __name__ extracts "int"
        let attr_name = self.resolve(attr);
        if attr_name == "__name__" && matches!(obj_type, Type::Str) {
            let result_local = self.alloc_and_add_local(Type::Str, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_TYPE_NAME_EXTRACT,
                ),
                args: vec![obj_operand],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        // Handle built-in exception attributes (.args, __class__)
        if matches!(&obj_type, Type::BuiltinException(_)) {
            if attr_name == "args" {
                let result_local = self.alloc_and_add_local(Type::Tuple(vec![Type::Str]), mir_func);
                // .args is field 0 on built-in exception instances
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD,
                    ),
                    args: vec![obj_operand, mir::Operand::Constant(mir::Constant::Int(0))],
                });
                return Ok(mir::Operand::Local(result_local));
            }
            if attr_name == "__class__" {
                // Return a type proxy string like "<class 'ValueError'>"
                // This allows chaining: e.__class__.__name__ -> "ValueError"
                let result_local = self.alloc_and_add_local(Type::Str, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_EXC_CLASS_NAME,
                    ),
                    args: vec![obj_operand],
                });
                return Ok(mir::Operand::Local(result_local));
            }
            // Other built-in exception attributes not supported yet
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        // Determine the field type and offset from class info
        if let Type::Class { class_id, .. } = &obj_type {
            // Try local class_info first
            if let Some(class_info) = self.get_class_info(class_id).cloned() {
                // 0. Handle __class__ on exception class instances
                if attr_name == "__class__" && class_info.is_exception_class {
                    let result_local = self.alloc_and_add_local(Type::Str, mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_EXC_CLASS_NAME,
                        ),
                        args: vec![obj_operand],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }

                // 1. Check for @property first - call getter method
                if let Some((getter_id, _setter)) = class_info.properties.get(&attr) {
                    let getter_id = *getter_id;
                    let prop_type = class_info
                        .property_types
                        .get(&attr)
                        .cloned()
                        .unwrap_or(Type::Any);

                    let result_local = self.alloc_and_add_local(prop_type.clone(), mir_func);

                    // Call the getter with self as the only argument
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: getter_id,
                        args: vec![obj_operand],
                    });

                    return Ok(mir::Operand::Local(result_local));
                }

                // 2. Check for regular field access
                if let Some(&offset) = class_info.field_offsets.get(&attr) {
                    let field_type = class_info
                        .field_types
                        .get(&attr)
                        .cloned()
                        .unwrap_or(Type::Any);

                    let result_local = self.alloc_and_add_local(field_type.clone(), mir_func);

                    // Get the field value from instance
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD,
                        ),
                        args: vec![
                            obj_operand,
                            mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                        ],
                    });

                    return Ok(mir::Operand::Local(result_local));
                }

                // 3. Fallback to class attribute (Python: instance.class_attr)
                if let (Some(&(owning_class_id, attr_offset)), Some(attr_type)) = (
                    class_info.class_attr_offsets.get(&attr),
                    class_info.class_attr_types.get(&attr).cloned(),
                ) {
                    let result_local = self.alloc_and_add_local(attr_type.clone(), mir_func);
                    let get_func = self.get_class_attr_get_func(&attr_type);
                    let effective_class_id = self.get_effective_class_id(owning_class_id);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: get_func,
                        args: vec![
                            mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                            mir::Operand::Constant(mir::Constant::Int(attr_offset as i64)),
                        ],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
            }

            // Try cross-module class info (for classes from imported modules)
            let attr_name = self.resolve(attr).to_string();
            if let Some(class_info) = self.get_cross_module_class_info(class_id) {
                if let Some(&offset) = class_info.field_offsets.get(&attr_name) {
                    let field_type = class_info
                        .field_types
                        .get(&attr_name)
                        .cloned()
                        .unwrap_or(Type::Any);

                    let result_local = self.alloc_and_add_local(field_type.clone(), mir_func);

                    // Get the field value from instance
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_INSTANCE_GET_FIELD,
                        ),
                        args: vec![
                            obj_operand,
                            mir::Operand::Constant(mir::Constant::Int(offset as i64)),
                        ],
                    });

                    return Ok(mir::Operand::Local(result_local));
                }
            }
        }

        let attr_name = self.resolve(attr);
        Err(pyaot_diagnostics::CompilerError::semantic_error(
            format!("unknown attribute '{}'", attr_name),
            obj_expr.span,
        ))
    }

    /// Lower a super() call: super().method(args)
    /// This calls the parent class's method with self as the first argument.
    pub(in crate::expressions) fn lower_super_call(
        &mut self,
        method: InternedString,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Get the current class from the current function context
        // The function name is mangled as "ClassName$methodname"
        let func_name = &mir_func.name;

        // Extract class name from function name
        let class_name = if let Some(idx) = func_name.find('$') {
            &func_name[..idx]
        } else {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        };

        // Find the current class by name
        let current_class_id = if let Some(class_id) = self.get_class_by_name(class_name) {
            class_id
        } else {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        };

        // Get the current class info and find the parent
        let base_class_id = if let Some(class_info) = self.get_class_info(&current_class_id) {
            if let Some(base_id) = class_info.base_class {
                base_id
            } else {
                // No parent class - super() on a class without inheritance
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        } else {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        };

        // Find the method in the parent class
        let parent_info = self.get_class_info(&base_class_id).cloned();
        if let Some(parent_class_info) = parent_info {
            if let Some(&parent_method_func_id) = parent_class_info.method_funcs.get(&method) {
                // Get the method's return type
                let return_type = hir_module
                    .func_defs
                    .get(&parent_method_func_id)
                    .and_then(|f| f.return_type.clone())
                    .unwrap_or(Type::None);

                let result_local = self.alloc_and_add_local(return_type.clone(), mir_func);

                // Get 'self' from the current function's first parameter
                // The first parameter of instance methods is always 'self'
                let self_local = mir_func
                    .params
                    .first()
                    .map(|p| p.id)
                    .unwrap_or_else(|| pyaot_utils::LocalId::new(0));
                let self_operand = mir::Operand::Local(self_local);

                // Lower method arguments
                let mut call_args = vec![self_operand];
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    call_args.push(self.lower_expr(arg_expr, hir_module, mir_func)?);
                }

                // Emit direct call to parent method (static dispatch)
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: parent_method_func_id,
                    args: call_args,
                });

                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Method not found in parent class
        Ok(mir::Operand::Constant(mir::Constant::None))
    }
}
