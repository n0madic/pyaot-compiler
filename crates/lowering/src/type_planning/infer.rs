//! Infer mode: bottom-up type synthesis
//!
//! Contains `compute_expr_type()` — the core expression type inference.
//! Moved from type_inference.rs.

use pyaot_hir as hir;
use pyaot_stdlib_defs::{lookup_object_field, lookup_object_type};
use pyaot_types::{typespec_to_type, Type};

use super::helpers;
use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Internal type computation (the actual inference logic).
    pub(crate) fn compute_expr_type(&mut self, expr: &hir::Expr, hir_module: &hir::Module) -> Type {
        match &expr.kind {
            hir::ExprKind::Var(var_id) => self
                .get_var_type(var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any),
            hir::ExprKind::BinOp { op, left, right } => {
                let left_ty = self.get_type_of_expr_id(*left, hir_module);
                let right_ty = self.get_type_of_expr_id(*right, hir_module);

                if let Some(ty) = helpers::resolve_binop_type(op, &left_ty, &right_ty) {
                    ty
                } else {
                    expr.ty.clone().unwrap_or(Type::Any)
                }
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let raw_obj_ty = self.get_type_of_expr_id(*obj, hir_module);
                let method_name = self.resolve(*method);
                // Unwrap Optional[T] (Union[T, None]) → T so method dispatch works.
                let obj_ty = helpers::unwrap_optional(&raw_obj_ty);
                // Try shared dispatch table first (Str, List, Dict, Set, File)
                if let Some(ty) = helpers::resolve_method_return_type(&obj_ty, method_name) {
                    return ty;
                }
                match &obj_ty {
                    Type::Class { ref class_id, .. } => {
                        // Get method return type from class info
                        if let Some(class_info) = self.get_class_info(class_id) {
                            // Check all method categories: instance, class, and static methods
                            let method_maps = [
                                &class_info.method_funcs,
                                &class_info.class_methods,
                                &class_info.static_methods,
                            ];
                            for methods in method_maps {
                                if let Some(&method_func_id) = methods.get(method) {
                                    // Check inferred return types first (from infer_all_return_types)
                                    if let Some(ret_ty) =
                                        self.func_return_types.get(&method_func_id)
                                    {
                                        return ret_ty.clone();
                                    }
                                    if let Some(func_def) =
                                        hir_module.func_defs.get(&method_func_id)
                                    {
                                        return func_def.return_type.clone().unwrap_or(Type::None);
                                    }
                                }
                            }
                        }
                        expr.ty.clone().unwrap_or(Type::Any)
                    }
                    // Handle RuntimeObject methods (Match.group(), etc.)
                    // using Single Source of Truth from stdlib-defs
                    Type::RuntimeObject(type_tag) => {
                        if let Some(obj_def) = lookup_object_type(*type_tag) {
                            if let Some(method_def) = obj_def.get_method(method_name) {
                                return typespec_to_type(&method_def.return_type);
                            }
                        }
                        expr.ty.clone().unwrap_or(Type::Any)
                    }
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }
            hir::ExprKind::Slice { obj, .. } => {
                // Slicing preserves the type
                self.get_type_of_expr_id(*obj, hir_module)
            }
            hir::ExprKind::Index { obj, index } => {
                let obj_ty = self.get_type_of_expr_id(*obj, hir_module);
                let index_expr = &hir_module.exprs[*index];
                let base = helpers::resolve_index_type(&obj_ty, index_expr);
                if base != Type::Any {
                    base
                } else if let Type::Class { class_id, .. } = &obj_ty {
                    // Class with __getitem__ — return type from the dunder method
                    self.get_class_info(class_id)
                        .and_then(|info| info.getitem_func)
                        .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
                        .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
                } else {
                    expr.ty.clone().unwrap_or(Type::Any)
                }
            }
            hir::ExprKind::List(elements) => {
                if elements.is_empty() {
                    expr.ty.clone().unwrap_or(Type::List(Box::new(Type::Any)))
                } else {
                    let elem_types: Vec<Type> = elements
                        .iter()
                        .map(|e| self.get_type_of_expr_id(*e, hir_module))
                        .collect();
                    Type::List(Box::new(helpers::unify_element_types(elem_types)))
                }
            }
            hir::ExprKind::Tuple(elements) => {
                // Infer tuple element types from all elements
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.get_type_of_expr_id(*e, hir_module))
                    .collect();
                Type::Tuple(elem_types)
            }
            hir::ExprKind::Dict(pairs) => {
                if pairs.is_empty() {
                    Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
                } else {
                    let key_types: Vec<Type> = pairs
                        .iter()
                        .map(|(k, _)| self.get_type_of_expr_id(*k, hir_module))
                        .collect();
                    let val_types: Vec<Type> = pairs
                        .iter()
                        .map(|(_, v)| self.get_type_of_expr_id(*v, hir_module))
                        .collect();
                    Type::Dict(
                        Box::new(helpers::unify_element_types(key_types)),
                        Box::new(helpers::unify_element_types(val_types)),
                    )
                }
            }
            hir::ExprKind::Set(elements) => {
                if elements.is_empty() {
                    Type::Set(Box::new(Type::Any))
                } else {
                    let elem_types: Vec<Type> = elements
                        .iter()
                        .map(|e| self.get_type_of_expr_id(*e, hir_module))
                        .collect();
                    Type::Set(Box::new(helpers::unify_element_types(elem_types)))
                }
            }
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg => self.get_type_of_expr_id(*operand, hir_module),
                hir::UnOp::Invert => Type::Int, // Bitwise NOT always returns Int
            },
            hir::ExprKind::Call { func, .. } => {
                // Get function return type for direct calls
                let func_expr = &hir_module.exprs[*func];
                if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
                    // Check inferred return types first (for generators), then HIR definition
                    if let Some(return_type) = self.get_func_return_type(func_id) {
                        return return_type.clone();
                    }
                    if let Some(func_def) = hir_module.func_defs.get(func_id) {
                        return func_def.return_type.clone().unwrap_or(Type::None);
                    }
                }
                // Check for variable that holds a function reference
                if let hir::ExprKind::Var(var_id) = &func_expr.kind {
                    // Check if this variable holds a class instance with __call__
                    if let Some(Type::Class { class_id, .. }) =
                        self.get_var_type(var_id).cloned().as_ref()
                    {
                        if let Some(call_func_id) = self
                            .get_class_info(class_id)
                            .and_then(|info| info.call_func)
                        {
                            if let Some(return_type) = self.get_func_return_type(&call_func_id) {
                                return return_type.clone();
                            }
                            if let Some(func_def) = hir_module.func_defs.get(&call_func_id) {
                                return func_def.return_type.clone().unwrap_or(Type::None);
                            }
                        }
                    }
                    // Check if this is a function pointer parameter (inside wrapper function)
                    // In this case, use the wrapper function's return type
                    if self.is_func_ptr_param(var_id) {
                        if let Some(return_type) = self.get_current_func_return_type() {
                            return return_type.clone();
                        }
                        return Type::Any;
                    }
                    // Check if this is a wrapped function (decorator that returns wrapper closure)
                    if let Some((wrapper_func_id, _)) = self.get_var_wrapper(var_id) {
                        // Return the wrapper function's return type
                        if let Some(return_type) = self.get_func_return_type(&wrapper_func_id) {
                            return return_type.clone();
                        }
                        if let Some(func_def) = hir_module.func_defs.get(&wrapper_func_id) {
                            return func_def.return_type.clone().unwrap_or(Type::None);
                        }
                    }
                    if let Some(func_id) = self.get_var_func(var_id) {
                        if let Some(return_type) = self.get_func_return_type(&func_id) {
                            return return_type.clone();
                        }
                        if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                            return func_def.return_type.clone().unwrap_or(Type::None);
                        }
                    }
                }
                // Check for class instantiation
                if let hir::ExprKind::ClassRef(class_id) = &func_expr.kind {
                    if let Some(class_def) = hir_module.class_defs.get(class_id) {
                        return Type::Class {
                            class_id: *class_id,
                            name: class_def.name,
                        };
                    }
                }
                // Check for cross-module class instantiation
                if let hir::ExprKind::ModuleAttr { module, attr } = &func_expr.kind {
                    let attr_name = self.resolve(*attr).to_string();
                    let key = (module.clone(), attr_name);
                    if let Some((class_id, _)) = self.get_module_class_export(&key) {
                        // Return proper Type::Class with the remapped ClassId
                        // attr is already the InternedString for the class name
                        return Type::Class {
                            class_id: *class_id,
                            name: *attr,
                        };
                    }
                }
                // Check for cross-module function calls via ImportedRef
                if let hir::ExprKind::ImportedRef { module, name } = &func_expr.kind {
                    let key = (module.clone(), name.clone());
                    if let Some(return_type) = self.get_module_func_export(&key) {
                        return return_type.clone();
                    }
                }
                expr.ty.clone().unwrap_or(Type::Any)
            }
            hir::ExprKind::Attribute { obj, attr } => {
                // Get field type from class definition
                let obj_ty = self.get_type_of_expr_id(*obj, hir_module);

                // Handle RuntimeObject attributes (StructTime, CompletedProcess, Match, etc.)
                // using Single Source of Truth from stdlib-defs
                if let Type::RuntimeObject(type_tag) = &obj_ty {
                    let attr_name = self.resolve(*attr);
                    if let Some(field_def) = lookup_object_field(*type_tag, attr_name) {
                        return typespec_to_type(&field_def.field_type);
                    }
                    return Type::Any;
                }

                // Handle File attributes
                if matches!(obj_ty, Type::File) {
                    let attr_name = self.resolve(*attr);
                    return match attr_name {
                        "closed" => Type::Bool,
                        "name" => Type::Str,
                        _ => Type::Any,
                    };
                }

                if let Type::Class { class_id, .. } = &obj_ty {
                    if let Some(class_info) = self.get_class_info(class_id) {
                        // Instance fields
                        if let Some(field_ty) = class_info.field_types.get(attr) {
                            return field_ty.clone();
                        }
                        // Properties (@property)
                        if let Some(prop_ty) = class_info.property_types.get(attr) {
                            return prop_ty.clone();
                        }
                        // Class attributes
                        if let Some(attr_ty) = class_info.class_attr_types.get(attr) {
                            return attr_ty.clone();
                        }
                    }
                }
                expr.ty.clone().unwrap_or(Type::Any)
            }
            hir::ExprKind::ClassRef(class_id) => {
                // ClassRef itself doesn't have a value type (it's used in calls)
                // Return Any since this shouldn't be used as a value directly
                if let Some(class_def) = hir_module.class_defs.get(class_id) {
                    Type::Class {
                        class_id: *class_id,
                        name: class_def.name,
                    }
                } else {
                    Type::Any
                }
            }
            hir::ExprKind::ClassAttrRef { class_id, attr } => {
                // Look up class attribute type from class info
                if let Some(class_info) = self.get_class_info(class_id) {
                    if let Some(attr_type) = class_info.class_attr_types.get(attr) {
                        return attr_type.clone();
                    }
                }
                // Fall back to Any if not found
                Type::Any
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|id| self.get_type_of_expr_id(*id, hir_module))
                    .collect();
                // Handle class-specific overrides before shared helper
                if matches!(builtin, hir::Builtin::Iter) && !arg_types.is_empty() {
                    if let Type::Class { class_id, .. } = &arg_types[0] {
                        if self
                            .get_class_info(class_id)
                            .and_then(|info| info.iter_func)
                            .is_some()
                        {
                            return arg_types[0].clone();
                        }
                    }
                }
                if matches!(builtin, hir::Builtin::Next) && !arg_types.is_empty() {
                    if let Type::Class { class_id, .. } = &arg_types[0] {
                        if let Some(ret) = self
                            .get_class_info(class_id)
                            .and_then(|info| info.next_func)
                            .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
                        {
                            return ret;
                        }
                    }
                }
                if let Some(ty) =
                    helpers::resolve_builtin_call_type(builtin, args, &arg_types, hir_module)
                {
                    ty
                } else if matches!(builtin, hir::Builtin::Map) {
                    // Map needs func_return_types access
                    let elem_type = if args.len() >= 2 {
                        let func_expr = &hir_module.exprs[args[0]];
                        let func_id = match &func_expr.kind {
                            hir::ExprKind::FuncRef(id) => Some(*id),
                            hir::ExprKind::Closure { func, .. } => Some(*func),
                            _ => None,
                        };
                        if let Some(func_id) = func_id {
                            if let Some(return_type) = self.get_func_return_type(&func_id) {
                                return_type.clone()
                            } else if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                                func_def.return_type.clone().unwrap_or(Type::Any)
                            } else {
                                Type::Any
                            }
                        } else {
                            Type::Any
                        }
                    } else {
                        Type::Any
                    };
                    Type::Iterator(Box::new(elem_type))
                } else {
                    expr.ty.clone().unwrap_or(Type::Any)
                }
            }
            hir::ExprKind::Closure { func, .. } => {
                // Closure type is the return type of the underlying function
                if let Some(func_def) = hir_module.func_defs.get(func) {
                    func_def.return_type.clone().unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            }
            hir::ExprKind::LogicalOp { left, right, .. } => {
                // Logical and/or return one of the operands
                let left_ty = self.get_type_of_expr_id(*left, hir_module);
                let right_ty = self.get_type_of_expr_id(*right, hir_module);

                if left_ty == right_ty {
                    left_ty
                } else if left_ty == Type::Any || right_ty == Type::Any {
                    Type::Any
                } else {
                    Type::normalize_union(vec![left_ty, right_ty])
                }
            }
            hir::ExprKind::Compare { .. } => {
                // Comparison always returns bool
                Type::Bool
            }
            hir::ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                // Ternary: if condition then then_val else else_val
                let then_ty = self.get_type_of_expr_id(*then_val, hir_module);
                let else_ty = self.get_type_of_expr_id(*else_val, hir_module);

                if then_ty == else_ty {
                    then_ty
                } else if then_ty == Type::Any || else_ty == Type::Any {
                    Type::Any
                } else {
                    // Codegen (lower_if_expr) already handles boxing for mismatched
                    // branches — it checks types_differ and boxes primitives.
                    Type::normalize_union(vec![then_ty, else_ty])
                }
            }
            hir::ExprKind::StdlibCall { func, .. } => {
                // Use return type from function definition (Single Source of Truth)
                typespec_to_type(&func.return_type)
            }
            hir::ExprKind::StdlibAttr(attr_def) => {
                // Use type from definition (Single Source of Truth)
                typespec_to_type(&attr_def.ty)
            }
            hir::ExprKind::StdlibConst(const_def) => {
                // Use type from definition (Single Source of Truth)
                typespec_to_type(&const_def.ty)
            }
            hir::ExprKind::ModuleAttr { module, attr } => {
                // Look up type from module_var_exports
                let attr_name = self.resolve(*attr).to_string();
                let key = (module.clone(), attr_name);
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
                // Not found - return Any
                Type::Any
            }
            hir::ExprKind::ImportedRef { module, name } => {
                // Look up type from module_var_exports
                let key = (module.clone(), name.clone());
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
                // Not found - return Any
                Type::Any
            }
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
    }
}

/// Extract the element type from an iterable type.
pub(crate) fn extract_iterable_element_type(ty: &Type) -> Type {
    match ty {
        Type::List(elem) => (**elem).clone(),
        Type::Tuple(elems) if !elems.is_empty() => Type::normalize_union(elems.clone()),
        Type::Tuple(_) => Type::Any,
        Type::Set(elem) => (**elem).clone(),
        Type::Dict(key, _) => (**key).clone(),
        Type::Str => Type::Str,
        Type::Bytes => Type::Int,
        Type::Iterator(elem) => (**elem).clone(),
        _ => Type::Any,
    }
}
