//! Unified type planning system
//!
//! Single module for all type inference in lowering:
//! - `infer`: bottom-up type synthesis (`compute_expr_type`)
//! - `pre_scan`: closure/lambda/decorator discovery before codegen
//! - `check`: top-down type validation + error reporting

mod check;
mod infer;
mod pre_scan;

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_stdlib_defs::lookup_object_field;
use pyaot_types::{typespec_to_type, Type};
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Run type planning: pre-scan + return type inference for all functions.
    pub(crate) fn run_type_planning(&mut self, hir_module: &hir::Module) {
        self.precompute_closure_capture_types(hir_module);
        self.process_module_decorated_functions(hir_module);
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
        let func_ids: Vec<_> = hir_module.functions.clone();

        for func_id in &func_ids {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                // Skip functions that already have explicit return type annotations
                let has_explicit = func.return_type.is_some()
                    && func.return_type.as_ref() != Some(&Type::None);
                if has_explicit {
                    self.func_return_types
                        .insert(*func_id, func.return_type.clone().unwrap());
                    continue;
                }

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
            // Multiple return types — take first concrete one
            return_types
                .into_iter()
                .find(|t| *t != Type::Any && *t != Type::None)
                .unwrap_or(Type::None)
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
            hir::StmtKind::For { body, .. }
            | hir::StmtKind::ForUnpack { body, .. }
            | hir::StmtKind::While { body, .. } => {
                for s in body {
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
                let left_ty =
                    self.infer_deep_expr_type(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_deep_expr_type(&module.exprs[*right], module, param_types);

                if matches!(op, hir::BinOp::Div) {
                    return Type::Float;
                }
                if left_ty == Type::Str || right_ty == Type::Str {
                    return Type::Str;
                }
                if left_ty == Type::Float || right_ty == Type::Float {
                    return Type::Float;
                }
                if left_ty == Type::Int && right_ty == Type::Int {
                    return Type::Int;
                }
                if matches!(left_ty, Type::List(_)) && matches!(op, hir::BinOp::Add) {
                    return left_ty;
                }
                Type::Any
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
                let left_ty =
                    self.infer_deep_expr_type(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_deep_expr_type(&module.exprs[*right], module, param_types);
                if left_ty == right_ty {
                    left_ty
                } else {
                    Type::Any
                }
            }

            // === If expression ===
            hir::ExprKind::IfExpr {
                then_val,
                else_val,
                ..
            } => {
                let then_ty =
                    self.infer_deep_expr_type(&module.exprs[*then_val], module, param_types);
                let else_ty =
                    self.infer_deep_expr_type(&module.exprs[*else_val], module, param_types);
                if then_ty == else_ty {
                    then_ty
                } else {
                    Type::Any
                }
            }

            // === Containers ===
            hir::ExprKind::List(elems) => {
                if elems.is_empty() {
                    expr.ty.clone().unwrap_or(Type::List(Box::new(Type::Any)))
                } else {
                    let first =
                        self.infer_deep_expr_type(&module.exprs[elems[0]], module, param_types);
                    Type::List(Box::new(first))
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
                    let key =
                        self.infer_deep_expr_type(&module.exprs[pairs[0].0], module, param_types);
                    let val =
                        self.infer_deep_expr_type(&module.exprs[pairs[0].1], module, param_types);
                    Type::Dict(Box::new(key), Box::new(val))
                }
            }
            hir::ExprKind::Set(elems) => {
                if elems.is_empty() {
                    Type::Set(Box::new(Type::Any))
                } else {
                    let first =
                        self.infer_deep_expr_type(&module.exprs[elems[0]], module, param_types);
                    Type::Set(Box::new(first))
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
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                use hir::Builtin;
                match builtin {
                    Builtin::Int => Type::Int,
                    Builtin::Float => Type::Float,
                    Builtin::Bool => Type::Bool,
                    Builtin::Str => Type::Str,
                    Builtin::Bytes => Type::Bytes,
                    Builtin::Len | Builtin::Hash | Builtin::Id | Builtin::Ord => Type::Int,
                    Builtin::Chr | Builtin::Repr | Builtin::Ascii | Builtin::Format
                    | Builtin::Input | Builtin::Bin | Builtin::Hex | Builtin::Oct
                    | Builtin::Type => Type::Str,
                    Builtin::Isinstance | Builtin::Issubclass | Builtin::All | Builtin::Any
                    | Builtin::Callable | Builtin::Hasattr => Type::Bool,
                    Builtin::Abs => {
                        if !args.is_empty() {
                            self.infer_deep_expr_type(
                                &module.exprs[args[0]],
                                module,
                                param_types,
                            )
                        } else {
                            Type::Int
                        }
                    }
                    Builtin::Print => Type::None,
                    Builtin::Range => Type::Iterator(Box::new(Type::Int)),
                    Builtin::Pow => Type::Float,
                    Builtin::Open => Type::File,
                    Builtin::Sorted => {
                        if !args.is_empty() {
                            let arg_ty = self.infer_deep_expr_type(
                                &module.exprs[args[0]],
                                module,
                                param_types,
                            );
                            let elem = infer::extract_iterable_element_type(&arg_ty);
                            Type::List(Box::new(elem))
                        } else {
                            Type::List(Box::new(Type::Any))
                        }
                    }
                    Builtin::List => {
                        if !args.is_empty() {
                            let arg_ty = self.infer_deep_expr_type(
                                &module.exprs[args[0]],
                                module,
                                param_types,
                            );
                            let elem = infer::extract_iterable_element_type(&arg_ty);
                            Type::List(Box::new(elem))
                        } else {
                            Type::List(Box::new(Type::Any))
                        }
                    }
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }

            // === Stdlib calls ===
            hir::ExprKind::StdlibCall { func, .. } => typespec_to_type(&func.return_type),
            hir::ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),
            hir::ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),

            // === Method calls (common patterns) ===
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let obj_ty =
                    self.infer_deep_expr_type(&module.exprs[*obj], module, param_types);
                let method_name = self.interner.resolve(*method);
                match &obj_ty {
                    Type::Str => match method_name {
                        "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace"
                        | "title" | "capitalize" | "swapcase" | "join" | "format"
                        | "center" | "ljust" | "rjust" | "zfill" => Type::Str,
                        "split" | "splitlines" => Type::List(Box::new(Type::Str)),
                        "find" | "rfind" | "index" | "rindex" | "count" => Type::Int,
                        "startswith" | "endswith" | "isdigit" | "isalpha" | "isalnum"
                        | "isspace" | "isupper" | "islower" => Type::Bool,
                        "encode" => Type::Bytes,
                        _ => Type::Any,
                    },
                    Type::List(elem_ty) => match method_name {
                        "pop" => (**elem_ty).clone(),
                        "copy" => Type::List(elem_ty.clone()),
                        "index" | "count" => Type::Int,
                        _ => Type::Any,
                    },
                    Type::Dict(key_ty, val_ty) => match method_name {
                        "get" | "pop" | "setdefault" => (**val_ty).clone(),
                        "keys" => Type::List(key_ty.clone()),
                        "values" => Type::List(val_ty.clone()),
                        _ => Type::Any,
                    },
                    _ => Type::Any,
                }
            }

            // === Indexing ===
            hir::ExprKind::Index { obj, .. } => {
                let obj_ty =
                    self.infer_deep_expr_type(&module.exprs[*obj], module, param_types);
                match obj_ty {
                    Type::List(elem) => *elem,
                    Type::Dict(_, val) => *val,
                    Type::Str => Type::Str,
                    Type::Bytes => Type::Int,
                    Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
                    _ => Type::Any,
                }
            }

            // === Slicing ===
            hir::ExprKind::Slice { obj, .. } => {
                self.infer_deep_expr_type(&module.exprs[*obj], module, param_types)
            }

            // === Attribute access ===
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty =
                    self.infer_deep_expr_type(&module.exprs[*obj], module, param_types);
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
