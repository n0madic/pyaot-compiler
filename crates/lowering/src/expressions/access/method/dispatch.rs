//! Method call dispatch based on object type

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_stdlib_defs::{lookup_object_method, StdlibMethodDef, ALL_OBJECT_TYPES};
use pyaot_types::Type;
use pyaot_utils::InternedString;

use crate::context::Lowering;

/// Look up a method by name across all object types that have methods
/// Returns the first matching method definition
/// This is used for fallback method dispatch when the type is not known
/// (e.g., Match objects which map to Any in the type system)
fn lookup_method_in_all_types(method_name: &str) -> Option<&'static StdlibMethodDef> {
    for obj_def in ALL_OBJECT_TYPES {
        if let Some(method_def) = obj_def.get_method(method_name) {
            return Some(method_def);
        }
    }
    None
}

impl<'a> Lowering<'a> {
    /// Lower a method call expression: obj.method(args, kwargs)
    pub(in crate::expressions) fn lower_method_call(
        &mut self,
        obj: hir::ExprId,
        method: InternedString,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        // Use get_expr_type for proper type inference
        let obj_type = self.get_expr_type(obj_expr, hir_module);

        let method_name = self.resolve(method).to_string();

        // Lower method arguments and collect their types
        let mut arg_operands = Vec::new();
        let mut arg_types = Vec::new();
        for arg_id in args {
            let arg_expr = &hir_module.exprs[*arg_id];
            arg_operands.push(self.lower_expr(arg_expr, hir_module, mir_func)?);
            arg_types.push(self.get_expr_type(arg_expr, hir_module));
        }

        match obj_type {
            Type::Str => self.lower_str_method(obj_operand, &method_name, arg_operands, mir_func),
            Type::Bytes => {
                self.lower_bytes_method(obj_operand, &method_name, arg_operands, mir_func)
            }
            Type::List(elem_ty) => self.lower_list_method(
                obj_operand,
                &method_name,
                arg_operands,
                arg_types,
                kwargs,
                elem_ty,
                hir_module,
                mir_func,
            ),
            Type::Dict(key_ty, value_ty) => self.lower_dict_method(
                obj_operand,
                &method_name,
                arg_operands,
                key_ty,
                value_ty,
                mir_func,
            ),
            Type::Set(elem_ty) => self.lower_set_method(
                obj_operand,
                &method_name,
                arg_operands,
                arg_types,
                &elem_ty,
                mir_func,
            ),
            Type::Tuple(_) => self.lower_tuple_method(
                obj_operand,
                &method_name,
                arg_operands,
                arg_types,
                mir_func,
            ),
            Type::Class { ref class_id, .. } => self.lower_class_method_call(
                obj_operand,
                method,
                arg_operands,
                class_id,
                hir_module,
                mir_func,
            ),
            Type::Iterator(elem_ty) => self.lower_generator_method(
                obj_operand,
                &method_name,
                arg_operands,
                &elem_ty,
                mir_func,
            ),
            Type::File => self.lower_file_method(obj_operand, &method_name, arg_operands, mir_func),
            Type::RuntimeObject(type_tag) => {
                // Handle RuntimeObject methods using the specific type tag
                // This correctly routes methods like geturl() to the right object type
                // (e.g., HTTPResponse.geturl() vs ParseResult.geturl())
                if let Some(method_def) = lookup_object_method(type_tag, &method_name) {
                    return self.lower_object_method_call(
                        obj_operand,
                        method_def,
                        args,
                        hir_module,
                        mir_func,
                    );
                }
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            _ => {
                // Check for object methods using ObjectTypeDef registry (generic)
                // This handles Match object methods (group, start, end, groups, span)
                // when the type is Any (e.g., from function return values before type inference).
                //
                // Note: This is a fallback for when the specific type is not known.
                // When possible, prefer using Type::RuntimeObject for proper dispatch.
                if let Some(method_def) = lookup_method_in_all_types(&method_name) {
                    return self.lower_object_method_call(
                        obj_operand,
                        method_def,
                        args,
                        hir_module,
                        mir_func,
                    );
                }
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
