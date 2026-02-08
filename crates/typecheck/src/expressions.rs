//! Expression type inference

use pyaot_hir::{ExprId, ExprKind, Module};
use pyaot_types::{typespec_to_type, Type};

use crate::context::TypeChecker;

impl<'a> TypeChecker<'a> {
    /// Infer the type of an expression
    pub(crate) fn infer_expr_type(&mut self, expr_id: ExprId, module: &Module) -> Type {
        let expr = &module.exprs[expr_id];

        // If already has a type, return it
        if let Some(ty) = &expr.ty {
            return ty.clone();
        }

        match &expr.kind {
            ExprKind::Int(_) => Type::Int,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Str(_) => Type::Str,
            ExprKind::Bytes(_) => Type::Bytes,
            ExprKind::None => Type::None,

            ExprKind::Var(var_id) => self.var_types.get(var_id).cloned().unwrap_or(Type::Any),

            ExprKind::FuncRef(_) => Type::Any, // Function references don't have first-class types

            ExprKind::ClassRef(class_id) => {
                if let Some(class_info) = self.class_info.get(class_id) {
                    Type::Class {
                        class_id: *class_id,
                        name: class_info.name,
                    }
                } else {
                    Type::Any
                }
            }

            ExprKind::ClassAttrRef { class_id, attr } => {
                // Look up the class attribute type from class definition
                if let Some(class_def) = module.class_defs.get(class_id) {
                    for class_attr in &class_def.class_attrs {
                        if class_attr.name == *attr {
                            return class_attr.ty.clone();
                        }
                    }
                }
                Type::Any
            }

            ExprKind::BinOp { op, left, right } => {
                let left_type = self.infer_expr_type(*left, module);
                let right_type = self.infer_expr_type(*right, module);
                self.infer_binop_type(*op, &left_type, &right_type)
            }

            ExprKind::UnOp { op, operand } => {
                let operand_type = self.infer_expr_type(*operand, module);
                self.infer_unop_type(*op, &operand_type)
            }

            ExprKind::Compare { .. } => Type::Bool,

            ExprKind::LogicalOp { left, right, .. } => {
                // Logical ops return one of their operands
                let left_type = self.infer_expr_type(*left, module);
                let right_type = self.infer_expr_type(*right, module);
                // Return union of types or most general
                if left_type == right_type {
                    left_type
                } else {
                    Type::Any
                }
            }

            ExprKind::Call {
                func,
                args,
                kwargs,
                kwargs_unpack,
            } => {
                // Expand **kwargs unpacking for type checking
                let mut expanded_kwargs = kwargs.to_vec();
                let mut has_runtime_kwargs = false;
                if let Some(kwargs_expr_id) = kwargs_unpack {
                    let kwargs_expr = &module.exprs[*kwargs_expr_id];
                    if let ExprKind::Dict(pairs) = &kwargs_expr.kind {
                        // Expand compile-time literal dict
                        for (key_id, value_id) in pairs {
                            let key_expr = &module.exprs[*key_id];
                            if let ExprKind::Str(key_str) = &key_expr.kind {
                                expanded_kwargs.push(pyaot_hir::KeywordArg {
                                    name: *key_str,
                                    value: *value_id,
                                    span: key_expr.span,
                                });
                            }
                        }
                    } else {
                        // Runtime **kwargs unpacking - skip required arg checks
                        has_runtime_kwargs = true;
                    }
                }

                self.infer_call_type_with_args(
                    *func,
                    args,
                    &expanded_kwargs,
                    has_runtime_kwargs,
                    module,
                    expr.span,
                )
            }

            ExprKind::BuiltinCall {
                builtin,
                args,
                kwargs,
            } => self.infer_builtin_type(*builtin, args, kwargs, module),

            ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                let then_type = self.infer_expr_type(*then_val, module);
                let else_type = self.infer_expr_type(*else_val, module);
                if then_type == else_type {
                    then_type
                } else {
                    Type::normalize_union(vec![then_type, else_type])
                }
            }

            ExprKind::List(items) => {
                if items.is_empty() {
                    Type::List(Box::new(Type::Any))
                } else {
                    // Compute union of all element types for heterogeneous lists
                    let elem_types: Vec<_> = items
                        .iter()
                        .map(|e| self.infer_expr_type(*e, module))
                        .collect();
                    let elem_type = Type::normalize_union(elem_types);
                    Type::List(Box::new(elem_type))
                }
            }

            ExprKind::Tuple(items) => {
                let elem_types: Vec<_> = items
                    .iter()
                    .map(|e| self.infer_expr_type(*e, module))
                    .collect();
                Type::Tuple(elem_types)
            }

            ExprKind::Dict(pairs) => {
                if pairs.is_empty() {
                    Type::Dict(Box::new(Type::Any), Box::new(Type::Any))
                } else {
                    // Compute union of all key types and value types for heterogeneous dicts
                    let key_types: Vec<_> = pairs
                        .iter()
                        .map(|(k, _)| self.infer_expr_type(*k, module))
                        .collect();
                    let value_types: Vec<_> = pairs
                        .iter()
                        .map(|(_, v)| self.infer_expr_type(*v, module))
                        .collect();
                    let key_type = Type::normalize_union(key_types);
                    let value_type = Type::normalize_union(value_types);
                    Type::Dict(Box::new(key_type), Box::new(value_type))
                }
            }

            ExprKind::Set(items) => {
                if items.is_empty() {
                    Type::Set(Box::new(Type::Any))
                } else {
                    // Compute union of all element types for heterogeneous sets
                    let elem_types: Vec<_> = items
                        .iter()
                        .map(|e| self.infer_expr_type(*e, module))
                        .collect();
                    let elem_type = Type::normalize_union(elem_types);
                    Type::Set(Box::new(elem_type))
                }
            }

            ExprKind::Index { obj, .. } => {
                let obj_type = self.infer_expr_type(*obj, module);
                match obj_type {
                    Type::List(elem_type) => *elem_type,
                    Type::Dict(_, value_type) => *value_type,
                    Type::Tuple(types) => {
                        // For tuple indexing, return first type (or Any for dynamic)
                        types.into_iter().next().unwrap_or(Type::Any)
                    }
                    Type::Str => Type::Str,
                    _ => Type::Any,
                }
            }

            ExprKind::Slice { obj, .. } => {
                // Slicing preserves the type
                self.infer_expr_type(*obj, module)
            }

            ExprKind::MethodCall {
                obj, method, args, ..
            } => self.infer_method_call_type(*obj, *method, args, module),

            ExprKind::Attribute { obj, attr } => {
                let obj_type = self.infer_expr_type(*obj, module);
                self.infer_attribute_type(&obj_type, *attr)
            }

            // TypeRef is used in isinstance() - it's a type expression, not a value
            ExprKind::TypeRef(_) => Type::Any,

            // Closure is a callable object - return Any since we don't have a proper Callable type
            // This allows decorators that return closures to work correctly
            ExprKind::Closure { .. } => Type::Any,

            // Yield expression - in Python, yield returns the value sent via .send()
            // For simplicity, we return Any since generators are typically used via iteration
            ExprKind::Yield(value) => {
                // The type of the yield expression is what gets sent back via .send()
                // For now, return Any. The yielded value determines the generator's element type.
                let _ = value; // Analyze the yielded value
                Type::Any
            }

            // SuperCall returns the parent method's return type
            // For simplicity, return Any since we don't track parent methods here
            ExprKind::SuperCall { .. } => Type::Any,

            // ImportedRef - type will be resolved after multi-module merging
            ExprKind::ImportedRef { .. } => Type::Any,

            // ModuleAttr - type will be resolved after multi-module merging
            ExprKind::ModuleAttr { .. } => Type::Any,

            // StdlibAttr - stdlib attribute access (type from definition)
            ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),

            // StdlibCall - stdlib function call (type from definition)
            ExprKind::StdlibCall { func, .. } => typespec_to_type(&func.return_type),

            // StdlibConst - compile-time constant (type from definition)
            ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),

            // BuiltinRef - reference to a first-class builtin function (stored as function pointer)
            ExprKind::BuiltinRef(_) => Type::Int,
        }
    }
}
