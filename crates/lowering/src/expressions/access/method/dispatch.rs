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
        // Use seed_expr_type for proper type inference
        let obj_type = self.seed_expr_type(obj, hir_module);

        let method_name = self.resolve(method).to_string();

        // Lower method arguments and collect their types
        let mut arg_operands = Vec::new();
        let mut arg_types = Vec::new();
        for arg_id in args {
            let arg_expr = &hir_module.exprs[*arg_id];
            arg_operands.push(self.lower_expr(arg_expr, hir_module, mir_func)?);
            arg_types.push(self.seed_expr_type(*arg_id, hir_module));
        }

        match obj_type {
            Type::Str => self.lower_str_method(
                obj_operand,
                &method_name,
                arg_operands,
                &arg_types,
                mir_func,
            ),
            Type::Bytes => {
                self.lower_bytes_method(obj_operand, &method_name, arg_operands, mir_func)
            }
            // `int` / `bool` methods (bool is an int subtype). Receiver is a
            // raw scalar; see `lower_int_method`.
            Type::Int | Type::Bool => {
                self.lower_int_method(obj_operand, &obj_type, &method_name, mir_func)
            }
            _ if obj_type.is_list_like() => {
                let elem_ty = Box::new(obj_type.list_elem().expect("list_like").clone());
                self.lower_list_method(
                    obj_operand,
                    &method_name,
                    arg_operands,
                    arg_types,
                    kwargs,
                    elem_ty,
                    hir_module,
                    mir_func,
                )
            }
            _ if obj_type.dict_kv().is_some() => {
                let (key_ty, value_ty) = obj_type.dict_kv().expect("dict_kv invariant");
                self.lower_dict_method(
                    obj_operand,
                    &method_name,
                    arg_operands,
                    Box::new(key_ty.clone()),
                    Box::new(value_ty.clone()),
                    mir_func,
                )
            }
            _ if obj_type.is_set_like() => {
                let elem_ty = obj_type.set_elem().expect("set_like").clone();
                self.lower_set_method(
                    obj_operand,
                    &method_name,
                    arg_operands,
                    arg_types,
                    &elem_ty,
                    mir_func,
                )
            }
            _ if obj_type.is_tuple_like() => self.lower_tuple_method(
                obj_operand,
                &method_name,
                arg_operands,
                arg_types,
                mir_func,
            ),
            // `deque[T]` is `Type::Generic{DEQUE_ID, [T]}` but runtime-backed
            // (`TypeTagKind::Deque`). This guard MUST precede the
            // `Type::Generic` arm below: otherwise a deque is routed to
            // `lower_class_method_call` (the user-class path) and every deque
            // method is silently lost. The runtime tag is statically known
            // from `is_deque_like()`.
            _ if obj_type.is_deque_like() => {
                if let Some(method_def) =
                    lookup_object_method(pyaot_types::TypeTagKind::Deque, &method_name)
                {
                    let result = self.lower_object_method_call(
                        obj_operand,
                        method_def,
                        args,
                        hir_module,
                        mir_func,
                    )?;
                    // `pop`/`popleft` return the stored tagged Value; unbox
                    // primitive element types so the result matches the deque's
                    // declared element type (mirror the `dq[i]` read in
                    // indexing.rs). Heap/Any elements pass through unchanged.
                    if matches!(method_name.as_str(), "pop" | "popleft") {
                        if let Some(elem_ty) = obj_type.deque_elem() {
                            if matches!(elem_ty, Type::Int | Type::Bool | Type::Float) {
                                let elem_ty = elem_ty.clone();
                                return Ok(self.unbox_if_needed(result, &elem_ty, mir_func));
                            }
                        }
                    }
                    return Ok(result);
                }
                // A genuinely-unimplemented deque method must fail the compile
                // loudly rather than silently evaluating to `None`.
                Err(pyaot_diagnostics::CompilerError::semantic_error(
                    format!("deque has no method '{}'", method_name),
                    self.call_span(),
                ))
            }
            Type::Class { ref class_id, .. } => self.lower_class_method_call(
                obj_operand,
                method,
                arg_operands,
                class_id,
                &obj_type,
                hir_module,
                mir_func,
            ),
            Type::Generic { ref base, .. } => self.lower_class_method_call(
                obj_operand,
                method,
                arg_operands,
                base,
                &obj_type,
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
            Type::File(binary) => {
                self.lower_file_method(obj_operand, &method_name, arg_operands, binary, mir_func)
            }
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
