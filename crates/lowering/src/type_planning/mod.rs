//! Unified type planning system
//!
//! Single module for all type inference in lowering:
//! - `infer`: bottom-up type synthesis (`compute_expr_type`)
//! - `pre_scan`: closure/lambda/decorator discovery before codegen
//! - `check`: top-down type validation + error reporting

mod check;
pub(crate) mod helpers;
mod infer;
mod pre_scan;

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_stdlib_defs::{lookup_object_field, lookup_object_type};
use pyaot_types::{typespec_to_type, Type};
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Run type planning: pre-scan + return type inference for all functions.
    pub(crate) fn run_type_planning(&mut self, hir_module: &hir::Module) {
        self.precompute_closure_capture_types(hir_module);
        self.process_module_decorated_functions(hir_module);
        self.refine_empty_container_types(hir_module);
        self.infer_all_return_types(hir_module);
    }

    /// Get the type of an expression by its ID (memoized).
    pub(crate) fn get_type_of_expr_id(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
    ) -> Type {
        if let Some(cached) = self.expr_types.get(&expr_id).cloned() {
            return cached;
        }
        let expr = &hir_module.exprs[expr_id];
        let result = self.compute_expr_type(expr, hir_module);
        self.expr_types.insert(expr_id, result.clone());
        result
    }

    /// Get the effective type of an expression.
    pub(crate) fn get_expr_type(&mut self, expr: &hir::Expr, hir_module: &hir::Module) -> Type {
        self.compute_expr_type(expr, hir_module)
    }
}

// =============================================================================
// Return Type Inference Pass
// =============================================================================

impl<'a> Lowering<'a> {
    /// Infer return types for ALL functions without explicit annotations.
    /// Runs before codegen so that `compute_expr_type` for Call expressions
    /// can look up return types in `func_return_types`.
    fn infer_all_return_types(&mut self, hir_module: &hir::Module) {
        // Collect func_ids to avoid borrow issues
        let func_ids = hir_module.functions.to_vec();

        // Pass 1: Collect explicitly annotated return types so they are available
        // for cross-function inference (fixes forward-reference ordering).
        // This includes `-> None` annotations — the HIR distinguishes
        // `Option::None` (no annotation) from `Some(Type::None)` (explicit `-> None`).
        for func_id in &func_ids {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                if let Some(ref return_type) = func.return_type {
                    self.func_return_types.insert(*func_id, return_type.clone());
                }
            }
        }

        // Pass 2: Infer return types for unannotated functions
        for func_id in &func_ids {
            // Skip functions already resolved in pass 1
            if self.func_return_types.contains_key(func_id) {
                continue;
            }

            if let Some(func) = hir_module.func_defs.get(func_id) {
                // Skip empty functions
                if func.body.is_empty() {
                    continue;
                }

                // Build param type map for this function
                let mut param_types: IndexMap<VarId, Type> = IndexMap::new();
                // Use lambda_param_type_hints if available (from map/filter/reduce pre-scan)
                let hints = self.lambda_param_type_hints.get(func_id).cloned();
                for (i, param) in func.params.iter().enumerate() {
                    let ty = param.ty.clone().unwrap_or_else(|| {
                        hints
                            .as_ref()
                            .and_then(|h| h.get(i).cloned())
                            .unwrap_or(Type::Any)
                    });
                    param_types.insert(param.var, ty);
                }

                // Scan body for return statements
                let return_type =
                    self.infer_return_type_from_body(&func.body, hir_module, &param_types);

                // Check for closure-returning functions (decorators)
                let return_type = if return_type == Type::None {
                    if self.find_returned_closure(func, hir_module).is_some() {
                        Type::Any
                    } else {
                        Type::None
                    }
                } else {
                    return_type
                };

                self.func_return_types.insert(*func_id, return_type);
            }
        }
    }

    /// Scan a function body for return statements and infer the return type.
    fn infer_return_type_from_body(
        &self,
        body: &[hir::StmtId],
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        let mut return_types = Vec::new();

        for stmt_id in body {
            self.collect_return_types(*stmt_id, module, param_types, &mut return_types);
        }

        if return_types.is_empty() {
            Type::None
        } else if return_types.len() == 1 {
            return_types.into_iter().next().unwrap()
        } else {
            // Multiple return types — union concrete types
            let concrete: Vec<Type> = return_types
                .into_iter()
                .filter(|t| *t != Type::Any)
                .collect();
            if concrete.is_empty() {
                Type::Any
            } else {
                Type::normalize_union(concrete)
            }
        }
    }

    /// Recursively collect return types from statements.
    fn collect_return_types(
        &self,
        stmt_id: hir::StmtId,
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
        return_types: &mut Vec<Type>,
    ) {
        let stmt = &module.stmts[stmt_id];
        match &stmt.kind {
            hir::StmtKind::Return(Some(expr_id)) => {
                let expr = &module.exprs[*expr_id];
                let ty = self.infer_deep_expr_type(expr, module, param_types);
                return_types.push(ty);
            }
            hir::StmtKind::Return(None) => {
                return_types.push(Type::None);
            }
            hir::StmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                for s in then_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for s in else_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
            }
            hir::StmtKind::For {
                body, else_block, ..
            }
            | hir::StmtKind::ForUnpack {
                body, else_block, ..
            }
            | hir::StmtKind::ForUnpackStarred {
                body, else_block, ..
            }
            | hir::StmtKind::While {
                body, else_block, ..
            } => {
                for s in body {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for s in else_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
            }
            hir::StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                for s in body {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for handler in handlers {
                    for s in &handler.body {
                        self.collect_return_types(*s, module, param_types, return_types);
                    }
                }
                for s in else_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
                for s in finally_block {
                    self.collect_return_types(*s, module, param_types, return_types);
                }
            }
            hir::StmtKind::Match { cases, .. } => {
                for case in cases {
                    for s in &case.body {
                        self.collect_return_types(*s, module, param_types, return_types);
                    }
                }
            }
            _ => {}
        }
    }

    /// Deep expression type inference for return type analysis.
    /// More comprehensive than the old `infer_expr_return_type_with_params` —
    /// handles Call, MethodCall, Index, Attribute, BuiltinCall, containers.
    fn infer_deep_expr_type(
        &self,
        expr: &hir::Expr,
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        match &expr.kind {
            // === Literals ===
            hir::ExprKind::Int(_) => Type::Int,
            hir::ExprKind::Float(_) => Type::Float,
            hir::ExprKind::Bool(_) => Type::Bool,
            hir::ExprKind::Str(_) => Type::Str,
            hir::ExprKind::Bytes(_) => Type::Bytes,
            hir::ExprKind::None => Type::None,

            // === Variables ===
            hir::ExprKind::Var(var_id) => param_types
                .get(var_id)
                .cloned()
                .or_else(|| self.get_var_type(var_id).cloned())
                .or_else(|| self.global_var_types.get(var_id).cloned())
                .unwrap_or(Type::Any),

            // === Binary operations ===
            hir::ExprKind::BinOp { op, left, right } => {
                let left_ty = self.infer_deep_expr_type(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_deep_expr_type(&module.exprs[*right], module, param_types);

                helpers::resolve_binop_type(op, &left_ty, &right_ty).unwrap_or(Type::Any)
            }

            // === Unary operations ===
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg => {
                    self.infer_deep_expr_type(&module.exprs[*operand], module, param_types)
                }
                hir::UnOp::Invert => Type::Int,
            },

            // === Comparisons / logical ===
            hir::ExprKind::Compare { .. } => Type::Bool,
            hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_ty = self.infer_deep_expr_type(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_deep_expr_type(&module.exprs[*right], module, param_types);
                if left_ty == right_ty {
                    left_ty
                } else if left_ty == Type::Any || right_ty == Type::Any {
                    Type::Any
                } else {
                    Type::normalize_union(vec![left_ty, right_ty])
                }
            }

            // === If expression ===
            hir::ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                let then_ty =
                    self.infer_deep_expr_type(&module.exprs[*then_val], module, param_types);
                let else_ty =
                    self.infer_deep_expr_type(&module.exprs[*else_val], module, param_types);
                if then_ty == else_ty {
                    then_ty
                } else if then_ty == Type::Any || else_ty == Type::Any {
                    Type::Any
                } else {
                    Type::normalize_union(vec![then_ty, else_ty])
                }
            }

            // === Containers ===
            hir::ExprKind::List(elems) => {
                if elems.is_empty() {
                    expr.ty.clone().unwrap_or(Type::List(Box::new(Type::Any)))
                } else {
                    let elem_types: Vec<Type> = elems
                        .iter()
                        .map(|e| self.infer_deep_expr_type(&module.exprs[*e], module, param_types))
                        .collect();
                    Type::List(Box::new(helpers::unify_element_types(elem_types)))
                }
            }
            hir::ExprKind::Tuple(elems) => {
                let types: Vec<Type> = elems
                    .iter()
                    .map(|e| self.infer_deep_expr_type(&module.exprs[*e], module, param_types))
                    .collect();
                Type::Tuple(types)
            }
            hir::ExprKind::Dict(pairs) => {
                if pairs.is_empty() {
                    Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
                } else {
                    let key_types: Vec<Type> = pairs
                        .iter()
                        .map(|(k, _)| {
                            self.infer_deep_expr_type(&module.exprs[*k], module, param_types)
                        })
                        .collect();
                    let val_types: Vec<Type> = pairs
                        .iter()
                        .map(|(_, v)| {
                            self.infer_deep_expr_type(&module.exprs[*v], module, param_types)
                        })
                        .collect();
                    Type::Dict(
                        Box::new(helpers::unify_element_types(key_types)),
                        Box::new(helpers::unify_element_types(val_types)),
                    )
                }
            }
            hir::ExprKind::Set(elems) => {
                if elems.is_empty() {
                    Type::Set(Box::new(Type::Any))
                } else {
                    let elem_types: Vec<Type> = elems
                        .iter()
                        .map(|e| self.infer_deep_expr_type(&module.exprs[*e], module, param_types))
                        .collect();
                    Type::Set(Box::new(helpers::unify_element_types(elem_types)))
                }
            }

            // === Function calls ===
            hir::ExprKind::Call { func, .. } => {
                let func_expr = &module.exprs[*func];
                match &func_expr.kind {
                    hir::ExprKind::FuncRef(func_id) => {
                        // Check pre-computed return types first
                        if let Some(ret) = self.func_return_types.get(func_id) {
                            return ret.clone();
                        }
                        // Then check HIR annotation
                        if let Some(func_def) = module.func_defs.get(func_id) {
                            return func_def.return_type.clone().unwrap_or(Type::None);
                        }
                        Type::Any
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
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }

            // === Builtin calls ===
            // === Builtin calls ===
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|id| self.infer_deep_expr_type(&module.exprs[*id], module, param_types))
                    .collect();
                if let Some(ty) =
                    helpers::resolve_builtin_call_type(builtin, args, &arg_types, module)
                {
                    ty
                } else if matches!(builtin, hir::Builtin::Map) {
                    // Map needs func_return_types access
                    let elem_type = if args.len() >= 2 {
                        let func_expr = &module.exprs[args[0]];
                        let func_id = match &func_expr.kind {
                            hir::ExprKind::FuncRef(id) => Some(*id),
                            hir::ExprKind::Closure { func, .. } => Some(*func),
                            _ => None,
                        };
                        if let Some(func_id) = func_id {
                            if let Some(ret) = self.func_return_types.get(&func_id) {
                                ret.clone()
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
                    Type::Iterator(Box::new(elem_type))
                } else {
                    expr.ty.clone().unwrap_or(Type::Any)
                }
            }

            // === Stdlib calls ===
            hir::ExprKind::StdlibCall { func, .. } => typespec_to_type(&func.return_type),
            hir::ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),
            hir::ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),

            // === Method calls (common patterns) ===
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let raw_obj_ty =
                    self.infer_deep_expr_type(&module.exprs[*obj], module, param_types);
                let method_name = self.interner.resolve(*method);
                // Unwrap Optional[T] (Union[T, None]) → T so method dispatch works.
                let obj_ty = helpers::unwrap_optional(&raw_obj_ty);
                // Try shared dispatch table first (Str, List, Dict, Set, File)
                if let Some(ty) = helpers::resolve_method_return_type(&obj_ty, method_name) {
                    return ty;
                }
                match &obj_ty {
                    Type::Class { class_id, .. } => {
                        if let Some(info) = self.class_info.get(class_id) {
                            let method_maps = [
                                &info.method_funcs,
                                &info.class_methods,
                                &info.static_methods,
                            ];
                            for methods in method_maps {
                                if let Some(&method_func_id) = methods.get(method) {
                                    if let Some(ret_ty) =
                                        self.func_return_types.get(&method_func_id)
                                    {
                                        return ret_ty.clone();
                                    }
                                    if let Some(func_def) = module.func_defs.get(&method_func_id) {
                                        return func_def.return_type.clone().unwrap_or(Type::None);
                                    }
                                }
                            }
                        }
                        Type::Any
                    }
                    Type::RuntimeObject(type_tag) => {
                        if let Some(obj_def) = lookup_object_type(*type_tag) {
                            if let Some(method_def) = obj_def.get_method(method_name) {
                                return typespec_to_type(&method_def.return_type);
                            }
                        }
                        Type::Any
                    }
                    _ => Type::Any,
                }
            }

            // === Indexing ===
            hir::ExprKind::Index { obj, index } => {
                let obj_ty = self.infer_deep_expr_type(&module.exprs[*obj], module, param_types);
                let index_expr = &module.exprs[*index];
                helpers::resolve_index_type(&obj_ty, index_expr)
            }

            // === Slicing ===
            hir::ExprKind::Slice { obj, .. } => {
                self.infer_deep_expr_type(&module.exprs[*obj], module, param_types)
            }

            // === Attribute access ===
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty = self.infer_deep_expr_type(&module.exprs[*obj], module, param_types);
                if let Type::RuntimeObject(tag) = &obj_ty {
                    let attr_name = self.interner.resolve(*attr);
                    if let Some(field) = lookup_object_field(*tag, attr_name) {
                        return typespec_to_type(&field.field_type);
                    }
                }
                if let Type::Class { class_id, .. } = &obj_ty {
                    if let Some(info) = self.class_info.get(class_id) {
                        if let Some(ty) = info.field_types.get(attr) {
                            return ty.clone();
                        }
                        if let Some(ty) = info.property_types.get(attr) {
                            return ty.clone();
                        }
                    }
                }
                Type::Any
            }

            // === Class instantiation ===
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

            // === Closures ===
            hir::ExprKind::Closure { func, .. } => {
                if let Some(func_def) = module.func_defs.get(func) {
                    func_def.return_type.clone().unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            }

            // === Fallback ===
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
    }
}
