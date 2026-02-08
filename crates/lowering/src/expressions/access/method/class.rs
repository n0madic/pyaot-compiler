//! Class method lowering (instance methods with virtual dispatch)

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::InternedString;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower instance method calls on user-defined classes.
    /// Uses virtual dispatch via vtable for polymorphic method calls.
    /// Also handles @staticmethod and @classmethod calls.
    pub(super) fn lower_class_method_call(
        &mut self,
        obj_operand: mir::Operand,
        method: InternedString,
        arg_operands: Vec<mir::Operand>,
        class_id: &pyaot_utils::ClassId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Try local class_info first
        if let Some(class_info) = self.get_class_info(class_id).cloned() {
            // 1. Check for @staticmethod first (no self/cls)
            if let Some(&static_func_id) = class_info.static_methods.get(&method) {
                let return_type = hir_module
                    .func_defs
                    .get(&static_func_id)
                    .and_then(|f| f.return_type.clone())
                    .unwrap_or(Type::None);

                let result_local = self.alloc_and_add_local(return_type.clone(), mir_func);

                // Static method: call directly without self
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: static_func_id,
                    args: arg_operands,
                });

                return Ok(mir::Operand::Local(result_local));
            }

            // 2. Check for @classmethod (receives cls as first arg)
            if let Some(&class_method_func_id) = class_info.class_methods.get(&method) {
                let return_type = hir_module
                    .func_defs
                    .get(&class_method_func_id)
                    .and_then(|f| f.return_type.clone())
                    .unwrap_or(Type::None);

                let result_local = self.alloc_and_add_local(return_type.clone(), mir_func);

                // Class method: call with class_id as first arg
                let mut call_args = vec![mir::Operand::Constant(mir::Constant::Int(
                    class_id.index() as i64,
                ))];
                call_args.extend(arg_operands);

                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: result_local,
                    func: class_method_func_id,
                    args: call_args,
                });

                return Ok(mir::Operand::Local(result_local));
            }

            // 3. Check for regular instance method with vtable dispatch
            if let Some(&method_func_id) = class_info.method_funcs.get(&method) {
                // Get the method's return type
                let return_type = hir_module
                    .func_defs
                    .get(&method_func_id)
                    .and_then(|f| f.return_type.clone())
                    .unwrap_or(Type::None);

                let result_local = self.alloc_and_add_local(return_type.clone(), mir_func);

                // Check if this method has a vtable slot (for virtual dispatch)
                if let Some(&slot) = class_info.vtable_slots.get(&method) {
                    // Use virtual dispatch via vtable
                    // Note: args don't include self - it's passed separately as obj
                    self.emit_instruction(mir::InstructionKind::CallVirtual {
                        dest: result_local,
                        obj: obj_operand,
                        slot,
                        args: arg_operands,
                    });
                } else {
                    // Fallback to static dispatch (shouldn't happen for class methods)
                    let mut call_args = vec![obj_operand];
                    call_args.extend(arg_operands);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: method_func_id,
                        args: call_args,
                    });
                }

                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Try cross-module class method call
        // Find which module this class belongs to by searching module_class_exports
        let method_name = self.resolve(method).to_string();

        // Collect the iterator to avoid holding immutable borrow while calling mutable methods
        let exports: Vec<_> = self.module_class_exports_iter().collect();
        for ((module_name, class_name), (export_class_id, _)) in exports {
            if export_class_id == class_id {
                // Found the class's source module
                // Construct mangled method name: __module_{module}_{Class}${method}
                let mangled_name =
                    format!("__module_{}_{}${}", module_name, class_name, method_name);

                // Look up method return type from cross-module class info
                let return_type = self
                    .get_cross_module_class_info(class_id)
                    .and_then(|info| info.method_return_types.get(&method_name))
                    .cloned()
                    .unwrap_or(Type::Int); // Default to Int if not found

                let result_local = self.alloc_and_add_local(return_type.clone(), mir_func);

                // Build call args: self first, then method args
                let mut call_args = vec![obj_operand];
                call_args.extend(arg_operands);

                self.emit_instruction(mir::InstructionKind::CallNamed {
                    dest: result_local,
                    name: mangled_name,
                    args: call_args,
                });

                return Ok(mir::Operand::Local(result_local));
            }
        }

        // Method not found
        Ok(mir::Operand::Constant(mir::Constant::None))
    }
}
