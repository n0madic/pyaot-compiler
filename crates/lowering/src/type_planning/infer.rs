//! Infer mode: bottom-up type synthesis
//!
//! Two parallel type inference entry points:
//!
//! - `compute_expr_type` — codegen path (`&mut self`), recurses via
//!   `get_type_of_expr_id` for memoized sub-expression resolution.
//!
//! - `infer_expr_type_inner` — pre-scan path (`&self`), recurses directly
//!   without memoization. Used by `infer_deep_expr_type` for return type
//!   inference and lambda/closure analysis before codegen starts.
//!
//! Complex match arms (MethodCall, Call, BuiltinCall, Attribute, Index)
//! are factored into shared `resolve_*` helpers that both paths call
//! after resolving sub-expression types.
//!
//! # Why two functions instead of one
//!
//! `compute_expr_type` MUST recurse through `get_type_of_expr_id` (which
//! caches results in `expr_types`). During lowering, `var_types` evolves
//! as statements are processed. If sub-expression types are computed fresh
//! (without cache), the same expression can produce different types at
//! different points in time — e.g., a variable starts as `Any` before
//! assignment, then becomes `Str` after. The cache freezes the type at
//! first computation, ensuring consistent codegen.
//!
//! `infer_expr_type_inner` takes `&self` and cannot call `get_type_of_expr_id`
//! (which requires `&mut self`). It also MUST NOT cache into `expr_types`
//! because it runs during pre-scan before lowering — caching at that point
//! would freeze stale types that the codegen path would later pick up.
//!
//! A closure-based unification also fails: a shared `unified(&self, ..., F)`
//! cannot accept a closure `|id| self.get_type_of_expr_id(id)` because the
//! closure needs `&mut self` while `unified` already holds `&self`.
//!
//! # Adding new match arms
//!
//! When adding support for a new `ExprKind` variant:
//! 1. Add the match arm to BOTH `compute_expr_type` and `infer_expr_type_inner`.
//! 2. If the arm has complex logic (>5 lines) that doesn't depend on how
//!    sub-expressions are resolved, extract it into a `resolve_*` helper
//!    method (takes `&self`, receives pre-computed types as arguments).
//! 3. Do NOT add explicit literal arms (Int/Float/Bool/Str/Bytes/None) to
//!    `compute_expr_type` — the codegen path relies on `expr.ty` fallback
//!    for literals to maintain consistency with how `var_types` caching
//!    interacts with type resolution order.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_stdlib_defs::{lookup_object_field, lookup_object_type};
use pyaot_types::{typespec_to_type, Type};
use pyaot_utils::interner::InternedString;
use pyaot_utils::VarId;

use super::helpers;
use crate::context::Lowering;

// =============================================================================
// Shared helpers: type resolution after sub-expressions are computed.
// All take `&self` so both `&mut self` (codegen) and `&self` (pre-scan) can use them.
// =============================================================================

impl<'a> Lowering<'a> {
    /// Resolve method call type on an already-resolved object type.
    /// Returns `None` if no resolution found (caller applies fallback).
    fn resolve_method_on_type(
        &self,
        obj_ty: &Type,
        method: InternedString,
        method_name: &str,
        module: &hir::Module,
    ) -> Option<Type> {
        // Shared dispatch table (Str, List, Dict, Set, File)
        if let Some(ty) = helpers::resolve_method_return_type(obj_ty, method_name) {
            return Some(ty);
        }
        match obj_ty {
            Type::Class { ref class_id, .. } => {
                if let Some(class_info) = self.get_class_info(class_id) {
                    let method_maps = [
                        &class_info.method_funcs,
                        &class_info.class_methods,
                        &class_info.static_methods,
                    ];
                    for methods in method_maps {
                        if let Some(&method_func_id) = methods.get(&method) {
                            if let Some(ret_ty) = self.get_func_return_type(&method_func_id) {
                                return Some(ret_ty.clone());
                            }
                            if let Some(func_def) = module.func_defs.get(&method_func_id) {
                                return Some(func_def.return_type.clone().unwrap_or(Type::None));
                            }
                        }
                    }
                    if let Some(func_id) = class_info.get_dunder_func(method_name) {
                        if let Some(ret_ty) = self.get_func_return_type(&func_id) {
                            return Some(ret_ty.clone());
                        }
                        if let Some(func_def) = module.func_defs.get(&func_id) {
                            if let Some(ret_ty) = func_def.return_type.clone() {
                                return Some(ret_ty);
                            }
                        }
                        return Some(match method_name {
                            "__eq__" | "__ne__" | "__lt__" | "__le__" | "__gt__" | "__ge__"
                            | "__bool__" | "__contains__" => Type::Bool,
                            "__str__" | "__repr__" => Type::Str,
                            "__hash__" | "__len__" => Type::Int,
                            "__setitem__" | "__delitem__" => Type::None,
                            _ => obj_ty.clone(),
                        });
                    }
                }
                None
            }
            Type::RuntimeObject(type_tag) => {
                if let Some(obj_def) = lookup_object_type(*type_tag) {
                    if let Some(method_def) = obj_def.get_method(method_name) {
                        return Some(typespec_to_type(&method_def.return_type));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Resolve call target type from the function expression.
    /// Returns `None` if no resolution found (caller applies fallback).
    fn resolve_call_target_type(
        &self,
        func_expr: &hir::Expr,
        module: &hir::Module,
    ) -> Option<Type> {
        if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
            if let Some(return_type) = self.get_func_return_type(func_id) {
                return Some(return_type.clone());
            }
            if let Some(func_def) = module.func_defs.get(func_id) {
                return Some(func_def.return_type.clone().unwrap_or(Type::None));
            }
        }
        if let hir::ExprKind::Var(var_id) = &func_expr.kind {
            if let Some(Type::Class { class_id, .. }) = self.get_var_type(var_id).cloned().as_ref()
            {
                if let Some(call_func_id) = self
                    .get_class_info(class_id)
                    .and_then(|info| info.call_func)
                {
                    if let Some(return_type) = self.get_func_return_type(&call_func_id) {
                        return Some(return_type.clone());
                    }
                    if let Some(func_def) = module.func_defs.get(&call_func_id) {
                        return Some(func_def.return_type.clone().unwrap_or(Type::None));
                    }
                }
            }
            if self.is_func_ptr_param(var_id) {
                if let Some(return_type) = self.get_current_func_return_type() {
                    return Some(return_type.clone());
                }
                return Some(Type::Any);
            }
            if let Some((_, original_func_id)) = self.get_var_wrapper(var_id) {
                if let Some(return_type) = self.get_func_return_type(&original_func_id) {
                    return Some(return_type.clone());
                }
                if let Some(func_def) = module.func_defs.get(&original_func_id) {
                    return Some(func_def.return_type.clone().unwrap_or(Type::None));
                }
            }
            if let Some((_, original_func_id)) = self.get_module_var_wrapper(var_id) {
                if let Some(return_type) = self.get_func_return_type(&original_func_id) {
                    return Some(return_type.clone());
                }
                if let Some(func_def) = module.func_defs.get(&original_func_id) {
                    return Some(func_def.return_type.clone().unwrap_or(Type::None));
                }
            }
            if let Some(func_id) = self.get_var_func(var_id) {
                if let Some(return_type) = self.get_func_return_type(&func_id) {
                    return Some(return_type.clone());
                }
                if let Some(func_def) = module.func_defs.get(&func_id) {
                    return Some(func_def.return_type.clone().unwrap_or(Type::None));
                }
            }
        }
        if let hir::ExprKind::ClassRef(class_id) = &func_expr.kind {
            if let Some(class_def) = module.class_defs.get(class_id) {
                return Some(Type::Class {
                    class_id: *class_id,
                    name: class_def.name,
                });
            }
        }
        if let hir::ExprKind::ModuleAttr {
            module: mod_name,
            attr,
        } = &func_expr.kind
        {
            let attr_name = self.resolve(*attr).to_string();
            let key = (mod_name.clone(), attr_name);
            if let Some((class_id, _)) = self.get_module_class_export(&key) {
                return Some(Type::Class {
                    class_id: *class_id,
                    name: *attr,
                });
            }
        }
        if let hir::ExprKind::ImportedRef {
            module: mod_name,
            name,
        } = &func_expr.kind
        {
            let key = (mod_name.clone(), name.clone());
            if let Some(return_type) = self.get_module_func_export(&key) {
                return Some(return_type.clone());
            }
        }
        None
    }

    /// Resolve builtin call with class __iter__/__next__ overrides and Map handling.
    /// Returns `None` if the standard `resolve_builtin_call_type` should be used
    /// without class overrides (i.e., no special handling needed).
    fn resolve_builtin_with_overrides(
        &self,
        builtin: &hir::Builtin,
        args: &[hir::ExprId],
        arg_types: &[Type],
        module: &hir::Module,
    ) -> Option<Type> {
        if matches!(builtin, hir::Builtin::Iter) && !arg_types.is_empty() {
            if let Type::Class { class_id, .. } = &arg_types[0] {
                if self
                    .get_class_info(class_id)
                    .and_then(|info| info.iter_func)
                    .is_some()
                {
                    return Some(arg_types[0].clone());
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
                    return Some(ret);
                }
            }
        }
        if let Some(ty) = helpers::resolve_builtin_call_type(builtin, args, arg_types, module) {
            return Some(ty);
        }
        if matches!(builtin, hir::Builtin::Map) {
            let elem_type = if args.len() >= 2 {
                let func_expr = &module.exprs[args[0]];
                let func_id = match &func_expr.kind {
                    hir::ExprKind::FuncRef(id) => Some(*id),
                    hir::ExprKind::Closure { func, .. } => Some(*func),
                    _ => None,
                };
                if let Some(func_id) = func_id {
                    if let Some(return_type) = self.get_func_return_type(&func_id) {
                        return_type.clone()
                    } else if let Some(func_def) = module.func_defs.get(&func_id) {
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
            return Some(Type::Iterator(Box::new(elem_type)));
        }
        None
    }

    /// Resolve attribute type on an already-resolved object type.
    /// Returns `None` if no resolution found (caller applies fallback).
    fn resolve_attribute_on_type(&self, obj_ty: &Type, attr: InternedString) -> Option<Type> {
        if let Type::RuntimeObject(type_tag) = obj_ty {
            let attr_name = self.resolve(attr);
            if let Some(field_def) = lookup_object_field(*type_tag, attr_name) {
                return Some(typespec_to_type(&field_def.field_type));
            }
            return Some(Type::Any);
        }
        if matches!(obj_ty, Type::File) {
            let attr_name = self.resolve(attr);
            return Some(match attr_name {
                "closed" => Type::Bool,
                "name" => Type::Str,
                _ => Type::Any,
            });
        }
        if let Type::Class { class_id, .. } = obj_ty {
            if let Some(class_info) = self.get_class_info(class_id) {
                if let Some(field_ty) = class_info.field_types.get(&attr) {
                    return Some(field_ty.clone());
                }
                if let Some(prop_ty) = class_info.property_types.get(&attr) {
                    return Some(prop_ty.clone());
                }
                if let Some(attr_ty) = class_info.class_attr_types.get(&attr) {
                    return Some(attr_ty.clone());
                }
            }
        }
        None
    }

    /// Resolve index type with __getitem__ fallback for classes.
    fn resolve_index_with_getitem(&self, obj_ty: &Type, index_expr: &hir::Expr) -> Option<Type> {
        let base = helpers::resolve_index_type(obj_ty, index_expr);
        if base != Type::Any {
            return Some(base);
        }
        if let Type::Class { class_id, .. } = obj_ty {
            if let Some(ty) = self
                .get_class_info(class_id)
                .and_then(|info| info.getitem_func)
                .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
            {
                return Some(ty);
            }
        }
        None
    }
}

// =============================================================================
// Codegen path: memoized sub-expression resolution via get_type_of_expr_id
// =============================================================================

impl<'a> Lowering<'a> {
    /// Codegen entry point for type inference.
    /// Called from `get_type_of_expr_id` (memoized) and `get_expr_type`.
    /// Uses `get_type_of_expr_id` for sub-expressions to ensure caching.
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
                helpers::resolve_binop_type(op, &left_ty, &right_ty)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg | hir::UnOp::Pos => self.get_type_of_expr_id(*operand, hir_module),
                hir::UnOp::Invert => Type::Int,
            },
            hir::ExprKind::Compare { .. } => Type::Bool,
            hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_ty = self.get_type_of_expr_id(*left, hir_module);
                let right_ty = self.get_type_of_expr_id(*right, hir_module);
                helpers::union_or_any(left_ty, right_ty)
            }
            hir::ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                let then_ty = self.get_type_of_expr_id(*then_val, hir_module);
                let else_ty = self.get_type_of_expr_id(*else_val, hir_module);
                helpers::union_or_any(then_ty, else_ty)
            }
            hir::ExprKind::List(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.get_type_of_expr_id(*e, hir_module))
                    .collect();
                helpers::infer_list_type(elem_types, expr.ty.as_ref())
            }
            hir::ExprKind::Tuple(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.get_type_of_expr_id(*e, hir_module))
                    .collect();
                Type::Tuple(elem_types)
            }
            hir::ExprKind::Dict(pairs) => {
                let key_types: Vec<Type> = pairs
                    .iter()
                    .map(|(k, _)| self.get_type_of_expr_id(*k, hir_module))
                    .collect();
                let val_types: Vec<Type> = pairs
                    .iter()
                    .map(|(_, v)| self.get_type_of_expr_id(*v, hir_module))
                    .collect();
                helpers::infer_dict_type(key_types, val_types)
            }
            hir::ExprKind::Set(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.get_type_of_expr_id(*e, hir_module))
                    .collect();
                helpers::infer_set_type(elem_types)
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let raw_obj_ty = self.get_type_of_expr_id(*obj, hir_module);
                let method_name = self.resolve(*method);
                let obj_ty = helpers::unwrap_optional(&raw_obj_ty);
                self.resolve_method_on_type(&obj_ty, *method, method_name, hir_module)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::Slice { obj, .. } => self.get_type_of_expr_id(*obj, hir_module),
            hir::ExprKind::Index { obj, index } => {
                let obj_ty = self.get_type_of_expr_id(*obj, hir_module);
                let index_expr = &hir_module.exprs[*index];
                self.resolve_index_with_getitem(&obj_ty, index_expr)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::Call { func, .. } => {
                let func_expr = &hir_module.exprs[*func];
                self.resolve_call_target_type(func_expr, hir_module)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|id| self.get_type_of_expr_id(*id, hir_module))
                    .collect();
                self.resolve_builtin_with_overrides(builtin, args, &arg_types, hir_module)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::StdlibCall { func, .. } => typespec_to_type(&func.return_type),
            hir::ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),
            hir::ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty = self.get_type_of_expr_id(*obj, hir_module);
                self.resolve_attribute_on_type(&obj_ty, *attr)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::ClassRef(class_id) => {
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
                if let Some(class_info) = self.get_class_info(class_id) {
                    if let Some(attr_type) = class_info.class_attr_types.get(attr) {
                        return attr_type.clone();
                    }
                }
                Type::Any
            }
            hir::ExprKind::Closure { func, .. } => {
                if let Some(func_def) = hir_module.func_defs.get(func) {
                    func_def.return_type.clone().unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            }
            hir::ExprKind::ModuleAttr { module, attr } => {
                let attr_name = self.resolve(*attr).to_string();
                let key = (module.clone(), attr_name);
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
                Type::Any
            }
            hir::ExprKind::ImportedRef { module, name } => {
                let key = (module.clone(), name.clone());
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
                Type::Any
            }
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
    }
}

// =============================================================================
// Pre-scan path: direct recursion without memoization
// =============================================================================

impl<'a> Lowering<'a> {
    /// Pre-scan entry point.
    pub(crate) fn infer_deep_expr_type(
        &self,
        expr: &hir::Expr,
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        self.infer_expr_type_inner(expr, module, Some(param_types))
    }

    /// Pre-scan inference engine. Direct recursion, no memoization.
    /// Same match arms as `compute_expr_type` but different sub-expression
    /// resolution and variable lookup strategy.
    pub(crate) fn infer_expr_type_inner(
        &self,
        expr: &hir::Expr,
        module: &hir::Module,
        param_types: Option<&IndexMap<VarId, Type>>,
    ) -> Type {
        match &expr.kind {
            // === Literals ===
            hir::ExprKind::Int(_) => Type::Int,
            hir::ExprKind::Float(_) => Type::Float,
            hir::ExprKind::Bool(_) => Type::Bool,
            hir::ExprKind::Str(_) => Type::Str,
            hir::ExprKind::Bytes(_) => Type::Bytes,
            hir::ExprKind::None => Type::None,

            hir::ExprKind::Var(var_id) => {
                if let Some(pt) = param_types {
                    pt.get(var_id)
                        .cloned()
                        .or_else(|| self.get_var_type(var_id).cloned())
                        .unwrap_or(Type::Any)
                } else {
                    self.get_var_type(var_id)
                        .cloned()
                        .or_else(|| expr.ty.clone())
                        .unwrap_or(Type::Any)
                }
            }
            hir::ExprKind::BinOp { op, left, right } => {
                let left_ty = self.infer_expr_type_inner(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_expr_type_inner(&module.exprs[*right], module, param_types);
                helpers::resolve_binop_type(op, &left_ty, &right_ty)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg | hir::UnOp::Pos => {
                    self.infer_expr_type_inner(&module.exprs[*operand], module, param_types)
                }
                hir::UnOp::Invert => Type::Int,
            },
            hir::ExprKind::Compare { .. } => Type::Bool,
            hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_ty = self.infer_expr_type_inner(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_expr_type_inner(&module.exprs[*right], module, param_types);
                helpers::union_or_any(left_ty, right_ty)
            }
            hir::ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                let then_ty =
                    self.infer_expr_type_inner(&module.exprs[*then_val], module, param_types);
                let else_ty =
                    self.infer_expr_type_inner(&module.exprs[*else_val], module, param_types);
                helpers::union_or_any(then_ty, else_ty)
            }
            hir::ExprKind::List(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.infer_expr_type_inner(&module.exprs[*e], module, param_types))
                    .collect();
                helpers::infer_list_type(elem_types, expr.ty.as_ref())
            }
            hir::ExprKind::Tuple(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.infer_expr_type_inner(&module.exprs[*e], module, param_types))
                    .collect();
                Type::Tuple(elem_types)
            }
            hir::ExprKind::Dict(pairs) => {
                let key_types: Vec<Type> = pairs
                    .iter()
                    .map(|(k, _)| {
                        self.infer_expr_type_inner(&module.exprs[*k], module, param_types)
                    })
                    .collect();
                let val_types: Vec<Type> = pairs
                    .iter()
                    .map(|(_, v)| {
                        self.infer_expr_type_inner(&module.exprs[*v], module, param_types)
                    })
                    .collect();
                helpers::infer_dict_type(key_types, val_types)
            }
            hir::ExprKind::Set(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.infer_expr_type_inner(&module.exprs[*e], module, param_types))
                    .collect();
                helpers::infer_set_type(elem_types)
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let raw_obj_ty =
                    self.infer_expr_type_inner(&module.exprs[*obj], module, param_types);
                let method_name = self.resolve(*method);
                let obj_ty = helpers::unwrap_optional(&raw_obj_ty);
                self.resolve_method_on_type(&obj_ty, *method, method_name, module)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::Slice { obj, .. } => {
                self.infer_expr_type_inner(&module.exprs[*obj], module, param_types)
            }
            hir::ExprKind::Index { obj, index } => {
                let obj_ty = self.infer_expr_type_inner(&module.exprs[*obj], module, param_types);
                let index_expr = &module.exprs[*index];
                self.resolve_index_with_getitem(&obj_ty, index_expr)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::Call { func, .. } => {
                let func_expr = &module.exprs[*func];
                self.resolve_call_target_type(func_expr, module)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|id| self.infer_expr_type_inner(&module.exprs[*id], module, param_types))
                    .collect();
                self.resolve_builtin_with_overrides(builtin, args, &arg_types, module)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::StdlibCall { func, .. } => typespec_to_type(&func.return_type),
            hir::ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),
            hir::ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty = self.infer_expr_type_inner(&module.exprs[*obj], module, param_types);
                self.resolve_attribute_on_type(&obj_ty, *attr)
                    .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
            }
            hir::ExprKind::ClassRef(class_id) => {
                if let Some(class_def) = module.class_defs.get(class_id) {
                    Type::Class {
                        class_id: *class_id,
                        name: class_def.name,
                    }
                } else {
                    Type::Any
                }
            }
            hir::ExprKind::ClassAttrRef { class_id, attr } => {
                if let Some(class_info) = self.get_class_info(class_id) {
                    if let Some(attr_type) = class_info.class_attr_types.get(attr) {
                        return attr_type.clone();
                    }
                }
                Type::Any
            }
            hir::ExprKind::Closure { func, .. } => {
                if let Some(func_def) = module.func_defs.get(func) {
                    func_def.return_type.clone().unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            }
            hir::ExprKind::ModuleAttr {
                module: mod_name,
                attr,
            } => {
                let attr_name = self.resolve(*attr).to_string();
                let key = (mod_name.clone(), attr_name);
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
                Type::Any
            }
            hir::ExprKind::ImportedRef {
                module: mod_name,
                name,
            } => {
                let key = (mod_name.clone(), name.clone());
                if let Some((_var_id, var_type)) = self.get_module_var_export(&key) {
                    return var_type.clone();
                }
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
