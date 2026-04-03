//! Class instantiation and imported function call lowering

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

use super::ExpandedArg;

impl<'a> Lowering<'a> {
    /// Lower a class instantiation: ClassName(args)
    /// Creates instance, initializes fields to null, then calls __init__ if present
    pub(crate) fn lower_class_instantiation(
        &mut self,
        class_id: pyaot_utils::ClassId,
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let class_info = self.get_class_info(&class_id).cloned();

        if let Some(info) = class_info {
            // Create the class type - get class name from class definition
            let class_name = match hir_module.class_defs.get(&class_id) {
                Some(class_def) => class_def.name,
                None => {
                    // If we can't find the class, return None
                    return Ok(mir::Operand::Constant(mir::Constant::None));
                }
            };

            let class_type = Type::Class {
                class_id,
                name: class_name,
            };

            // Allocate result local for the instance
            let result_local = self.alloc_and_add_local(class_type.clone(), mir_func);

            // Use offset-adjusted class_id for local classes
            let effective_class_id = self.get_effective_class_id(class_id);

            if let Some(new_func_id) = info.new_func {
                // __new__ path: call __new__(cls, *args) which returns an instance,
                // then call __init__ on the result.
                // __new__ receives cls (class_id as int) as first arg.
                if let Some(new_func) = hir_module.func_defs.get(&new_func_id) {
                    let new_params: Vec<_> = new_func.params.iter().skip(1).cloned().collect();
                    let user_args = self.resolve_call_args(
                        args,
                        kwargs,
                        &new_params,
                        Some(new_func_id),
                        1,
                        self.call_span(),
                        hir_module,
                        mir_func,
                    )?;

                    let mut new_args = vec![mir::Operand::Constant(mir::Constant::Int(
                        effective_class_id,
                    ))];
                    new_args.extend(user_args);

                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: new_func_id,
                        args: new_args,
                    });
                }
            } else {
                // Default path: rt_make_instance(class_id, total_field_count)
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_MAKE_INSTANCE,
                    ),
                    args: vec![
                        mir::Operand::Constant(mir::Constant::Int(effective_class_id)),
                        mir::Operand::Constant(mir::Constant::Int(info.total_field_count as i64)),
                    ],
                });
            }

            // Call __init__ if present
            if let Some(init_func_id) = info.init_func {
                // Get the __init__ function definition
                if let Some(init_func) = hir_module.func_defs.get(&init_func_id) {
                    // Resolve arguments: __init__ takes self as first argument
                    // Note: __init__ params include 'self', so we skip it when matching user args
                    let init_params: Vec<_> = init_func.params.iter().skip(1).cloned().collect();

                    // Lower the user-provided arguments (skip self)
                    // We use param_index_offset=1 because:
                    // - default_value_slots uses (FuncId, param_index) where param_index is
                    //   relative to the original function parameters (including self)
                    // - init_params skips self, so param at index 0 in init_params is actually
                    //   at index 1 in the original function
                    let user_args = self.resolve_call_args(
                        args,
                        kwargs,
                        &init_params,
                        Some(init_func_id),
                        1, // Offset by 1 because self is skipped
                        self.call_span(),
                        hir_module,
                        mir_func,
                    )?;

                    // Build full args: self + user args
                    let mut all_args = vec![mir::Operand::Local(result_local)];
                    all_args.extend(user_args);

                    // Create dummy local for __init__ return (always None)
                    let init_result_local = self.alloc_and_add_local(Type::None, mir_func);

                    // Call __init__
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: init_result_local,
                        func: init_func_id,
                        args: all_args,
                    });
                }
            }

            Ok(mir::Operand::Local(result_local))
        } else {
            Err(pyaot_diagnostics::CompilerError::semantic_error(
                "cannot instantiate unknown class",
                self.call_span(),
            ))
        }
    }

    /// Lower a cross-module class instantiation: module.ClassName(args)
    /// The class_id is already remapped (offset-adjusted) from module_class_exports.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_cross_module_class_instantiation(
        &mut self,
        source_module: &str,
        class_id: pyaot_utils::ClassId,
        class_name: &str,
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // For cross-module classes, we don't have class_info available.
        // We create the instance and call __init__ via CallNamed.

        // Lower arguments with runtime unpacking support
        let _ = kwargs; // Ignore kwargs for now
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;

        // Use Type::Any for cross-module classes
        // We can't create a proper Type::Class because the class_name InternedString
        // is from a different module's interner. Type::Any maps to pointer type
        // which is correct for class instances.
        let _ = class_name; // Used for __init__ call only
        let class_type = Type::Any;

        // Allocate result local for the instance
        let result_local = self.alloc_and_add_local(class_type, mir_func);

        // Get actual field count: try cross-module metadata first, then local class info.
        let default_field_count = self
            .get_cross_module_class_info(&class_id)
            .map(|info| info.total_field_count as i64)
            .or_else(|| {
                self.get_class_info(&class_id)
                    .map(|info| info.total_field_count as i64)
            })
            .unwrap_or(32);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_INSTANCE),
            args: vec![
                mir::Operand::Constant(mir::Constant::Int(class_id.0 as i64)),
                mir::Operand::Constant(mir::Constant::Int(default_field_count)),
            ],
        });

        // Call __init__ via CallNamed
        // The __init__ function name is mangled as __module_{module}_{class}$__init__
        // (uses $ as separator between class name and method name)
        // Replace dots with underscores for package paths
        let safe_module = source_module.replace('.', "_");
        let init_func_name = format!("__module_{}_{}$__init__", safe_module, class_name);

        // Create dummy local for __init__ return (always None)
        let init_result_local = self.alloc_and_add_local(Type::None, mir_func);

        // Build args: self + user args
        let mut all_args = vec![mir::Operand::Local(result_local)];
        all_args.extend(arg_operands);

        // Call __init__ via CallNamed
        self.emit_instruction(mir::InstructionKind::CallNamed {
            dest: init_result_local,
            name: init_func_name,
            args: all_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a call to an imported function.
    /// Generates a CallNamed instruction that will be resolved at codegen time.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_imported_call(
        &mut self,
        module: &str,
        name: &str,
        args: &[ExpandedArg],
        kwargs: &[hir::KeywordArg],
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Generate the mangled function name (replace dots with underscores for packages)
        let safe_module = module.replace('.', "_");
        let mangled_name = format!("__module_{}_{}", safe_module, name);

        // Lower arguments with runtime unpacking support (ignore kwargs for imported functions for now)
        let _ = kwargs;
        let arg_operands = self.lower_expanded_args(args, hir_module, mir_func)?;

        // Determine result type:
        // 1. Check module_func_exports for cross-module function return types
        // 2. Fall back to expression type hint
        // 3. Default to Any
        let key = (module.to_string(), name.to_string());
        let result_ty = self
            .get_module_func_export(&key)
            .cloned()
            .or_else(|| expr.ty.clone())
            .unwrap_or(Type::Any);
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Emit CallNamed instruction - will be resolved at codegen time
        self.emit_instruction(mir::InstructionKind::CallNamed {
            dest: result_local,
            name: mangled_name,
            args: arg_operands,
        });

        Ok(mir::Operand::Local(result_local))
    }
}
