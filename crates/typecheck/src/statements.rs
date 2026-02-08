//! Statement type checking

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{ExprId, Module, StmtId, StmtKind};
use pyaot_types::Type;

use crate::context::TypeChecker;

impl<'a> TypeChecker<'a> {
    /// Check a list of statements
    pub(crate) fn check_stmts(&mut self, stmts: &[StmtId], module: &Module) -> Result<()> {
        for &stmt_id in stmts {
            self.check_stmt(stmt_id, module)?;
        }
        Ok(())
    }

    /// Check a single statement
    fn check_stmt(&mut self, stmt_id: StmtId, module: &Module) -> Result<()> {
        let stmt = &module.stmts[stmt_id];
        let span = stmt.span;

        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                type_hint,
            } => {
                let value_type = self.infer_expr_type(*value, module);

                // If there's a type hint, check compatibility
                if let Some(hint) = type_hint {
                    if !value_type.is_subtype_of(hint) && value_type != Type::Any {
                        let expr_span = module.exprs[*value].span;
                        return Err(CompilerError::type_error(
                            format!(
                                "cannot assign value of type '{}' to variable of type '{}'",
                                value_type, hint
                            ),
                            expr_span,
                        ));
                    }
                    self.var_types.insert(*target, hint.clone());
                } else {
                    self.var_types.insert(*target, value_type);
                }
            }

            StmtKind::NestedUnpackAssign { targets, value } => {
                let value_type = self.infer_expr_type(*value, module);
                self.check_nested_unpack_types(targets, &value_type, *value, module)?;
            }

            StmtKind::UnpackAssign {
                before_star,
                starred,
                after_star,
                value,
            } => {
                let value_type = self.infer_expr_type(*value, module);
                let total_fixed = before_star.len() + after_star.len();

                // Check that value is a tuple or list with matching length
                match &value_type {
                    Type::Tuple(elem_types) => {
                        if starred.is_some() {
                            // With starred: need at least total_fixed elements
                            if elem_types.len() < total_fixed {
                                let expr_span = module.exprs[*value].span;
                                return Err(CompilerError::type_error(
                                    format!(
                                        "not enough values to unpack (expected at least {}, got {})",
                                        total_fixed,
                                        elem_types.len()
                                    ),
                                    expr_span,
                                ));
                            }
                            // Assign types to before_star elements
                            for (i, target) in before_star.iter().enumerate() {
                                self.var_types.insert(*target, elem_types[i].clone());
                            }
                            // Starred variable gets list of middle elements
                            if let Some(starred_var) = starred {
                                let middle_start = before_star.len();
                                let middle_end = elem_types.len() - after_star.len();
                                // Compute union of middle element types
                                let middle_types: Vec<_> =
                                    elem_types[middle_start..middle_end].to_vec();
                                let elem_type = if middle_types.is_empty() {
                                    Type::Any
                                } else {
                                    Type::normalize_union(middle_types)
                                };
                                self.var_types
                                    .insert(*starred_var, Type::List(Box::new(elem_type)));
                            }
                            // Assign types to after_star elements
                            for (i, target) in after_star.iter().enumerate() {
                                let idx = elem_types.len() - after_star.len() + i;
                                self.var_types.insert(*target, elem_types[idx].clone());
                            }
                        } else {
                            // Without starred: exact match required
                            if elem_types.len() != total_fixed {
                                let expr_span = module.exprs[*value].span;
                                return Err(CompilerError::type_error(
                                    format!(
                                        "cannot unpack tuple of {} elements into {} targets",
                                        elem_types.len(),
                                        total_fixed
                                    ),
                                    expr_span,
                                ));
                            }
                            for (target, elem_type) in before_star.iter().zip(elem_types.iter()) {
                                self.var_types.insert(*target, elem_type.clone());
                            }
                        }
                    }
                    Type::List(elem_type) => {
                        // List unpacking - all fixed targets get the element type
                        for target in before_star {
                            self.var_types.insert(*target, (**elem_type).clone());
                        }
                        // Starred variable gets list type
                        if let Some(starred_var) = starred {
                            self.var_types
                                .insert(*starred_var, Type::List(elem_type.clone()));
                        }
                        for target in after_star {
                            self.var_types.insert(*target, (**elem_type).clone());
                        }
                    }
                    Type::Any => {
                        // Any type - all targets get Any
                        for target in before_star {
                            self.var_types.insert(*target, Type::Any);
                        }
                        if let Some(starred_var) = starred {
                            self.var_types
                                .insert(*starred_var, Type::List(Box::new(Type::Any)));
                        }
                        for target in after_star {
                            self.var_types.insert(*target, Type::Any);
                        }
                    }
                    _ => {
                        let expr_span = module.exprs[*value].span;
                        return Err(CompilerError::type_error(
                            format!("cannot unpack value of type '{}'", value_type),
                            expr_span,
                        ));
                    }
                }
            }

            StmtKind::Return(expr) => {
                let return_type = expr
                    .map(|e| self.infer_expr_type(e, module))
                    .unwrap_or(Type::None);

                if let Some(expected) = &self.expected_return_type {
                    if !return_type.is_subtype_of(expected) && return_type != Type::Any {
                        return Err(CompilerError::type_error(
                            format!(
                                "return type '{}' does not match declared return type '{}'",
                                return_type, expected
                            ),
                            span,
                        ));
                    }
                }
            }

            StmtKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.check_condition(*cond, module)?;
                self.check_stmts(then_block, module)?;
                self.check_stmts(else_block, module)?;
            }

            StmtKind::While {
                cond,
                body,
                else_block,
            } => {
                self.check_condition(*cond, module)?;
                self.check_stmts(body, module)?;
                self.check_stmts(else_block, module)?;
            }

            StmtKind::For {
                iter,
                body,
                else_block,
                ..
            }
            | StmtKind::ForUnpack {
                iter,
                body,
                else_block,
                ..
            }
            | StmtKind::ForUnpackStarred {
                iter,
                body,
                else_block,
                ..
            } => {
                // Check iterator expression (don't need to validate type strictly)
                let _ = self.infer_expr_type(*iter, module);
                self.check_stmts(body, module)?;
                self.check_stmts(else_block, module)?;
            }

            StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                self.check_stmts(body, module)?;
                for handler in handlers {
                    // If handler has a name binding (except Exception as e:),
                    // register the variable with type Str (exception message)
                    if let Some(var_id) = handler.name {
                        self.var_types.insert(var_id, Type::Str);
                    }
                    self.check_stmts(&handler.body, module)?;
                }
                self.check_stmts(else_block, module)?;
                self.check_stmts(finally_block, module)?;
            }

            StmtKind::Assert { cond, msg } => {
                self.check_condition(*cond, module)?;
                if let Some(msg_id) = msg {
                    let msg_type = self.infer_expr_type(*msg_id, module);
                    if msg_type != Type::Str && msg_type != Type::Any {
                        let msg_span = module.exprs[*msg_id].span;
                        return Err(CompilerError::type_error(
                            format!("assert message must be str, got '{}'", msg_type),
                            msg_span,
                        ));
                    }
                }
            }

            StmtKind::IndexAssign { obj, index, value } => {
                let obj_type = self.infer_expr_type(*obj, module);
                let index_type = self.infer_expr_type(*index, module);
                let value_type = self.infer_expr_type(*value, module);

                match &obj_type {
                    Type::List(elem_type) => {
                        if index_type != Type::Int && index_type != Type::Any {
                            let idx_span = module.exprs[*index].span;
                            return Err(CompilerError::type_error(
                                format!("list indices must be int, got '{}'", index_type),
                                idx_span,
                            ));
                        }
                        if !value_type.is_subtype_of(elem_type) && value_type != Type::Any {
                            let val_span = module.exprs[*value].span;
                            return Err(CompilerError::type_error(
                                format!(
                                    "cannot assign '{}' to list of '{}'",
                                    value_type, elem_type
                                ),
                                val_span,
                            ));
                        }
                    }
                    Type::Dict(key_type, val_type) => {
                        if !index_type.is_subtype_of(key_type) && index_type != Type::Any {
                            let idx_span = module.exprs[*index].span;
                            return Err(CompilerError::type_error(
                                format!(
                                    "dict key type mismatch: expected '{}', got '{}'",
                                    key_type, index_type
                                ),
                                idx_span,
                            ));
                        }
                        if !value_type.is_subtype_of(val_type) && value_type != Type::Any {
                            let val_span = module.exprs[*value].span;
                            return Err(CompilerError::type_error(
                                format!(
                                    "dict value type mismatch: expected '{}', got '{}'",
                                    val_type, value_type
                                ),
                                val_span,
                            ));
                        }
                    }
                    Type::Any => {}
                    Type::Class { .. } => {
                        // Allow item assignment for classes with __setitem__
                        // Actual dispatch is handled in lowering
                    }
                    _ => {
                        let obj_span = module.exprs[*obj].span;
                        return Err(CompilerError::type_error(
                            format!("'{}' does not support item assignment", obj_type),
                            obj_span,
                        ));
                    }
                }
            }

            StmtKind::FieldAssign { obj, field, value } => {
                let obj_type = self.infer_expr_type(*obj, module);
                let value_type = self.infer_expr_type(*value, module);

                if let Type::Class { class_id, name } = &obj_type {
                    if let Some(class_info) = self.class_info.get(class_id) {
                        // Check for property first
                        if let Some(prop_type) = class_info.properties.get(field) {
                            // Check if property has a setter
                            if !class_info
                                .property_setters
                                .get(field)
                                .copied()
                                .unwrap_or(false)
                            {
                                let field_name = self.interner.resolve(*field);
                                return Err(CompilerError::type_error(
                                    format!("property '{}' has no setter", field_name),
                                    span,
                                ));
                            }
                            // Type check the value
                            if !value_type.is_subtype_of(prop_type) && value_type != Type::Any {
                                let val_span = module.exprs[*value].span;
                                let field_name = self.interner.resolve(*field);
                                return Err(CompilerError::type_error(
                                    format!(
                                        "cannot assign '{}' to property '{}' of type '{}'",
                                        value_type, field_name, prop_type
                                    ),
                                    val_span,
                                ));
                            }
                        } else if let Some(field_type) = class_info.fields.get(field) {
                            if !value_type.is_subtype_of(field_type) && value_type != Type::Any {
                                let val_span = module.exprs[*value].span;
                                let field_name = self.interner.resolve(*field);
                                return Err(CompilerError::type_error(
                                    format!(
                                        "cannot assign '{}' to field '{}' of type '{}'",
                                        value_type, field_name, field_type
                                    ),
                                    val_span,
                                ));
                            }
                        } else {
                            let field_name = self.interner.resolve(*field);
                            let class_name = self.interner.resolve(*name);
                            return Err(CompilerError::type_error(
                                format!("class '{}' has no field '{}'", class_name, field_name),
                                span,
                            ));
                        }
                    }
                } else if obj_type != Type::Any {
                    let obj_span = module.exprs[*obj].span;
                    return Err(CompilerError::type_error(
                        format!("'{}' has no fields", obj_type),
                        obj_span,
                    ));
                }
            }

            StmtKind::Expr(expr_id) => {
                let _ = self.infer_expr_type(*expr_id, module);
            }

            StmtKind::Raise { exc, cause } => {
                if let Some(expr_id) = exc {
                    let _ = self.infer_expr_type(*expr_id, module);
                }
                if let Some(cause_id) = cause {
                    let _ = self.infer_expr_type(*cause_id, module);
                }
            }

            StmtKind::Break | StmtKind::Continue | StmtKind::Pass => {}

            StmtKind::ClassAttrAssign {
                class_id,
                attr,
                value,
            } => {
                let _ = self.infer_expr_type(*value, module);
                // Type is determined by the class attribute definition
                let _ = (class_id, attr);
            }

            StmtKind::Match { subject, cases } => {
                // Check subject expression
                let _ = self.infer_expr_type(*subject, module);

                // Check each case
                for case in cases {
                    // Check guard if present
                    if let Some(guard) = case.guard {
                        self.check_condition(guard, module)?;
                    }
                    // Check body statements
                    for &stmt_id in &case.body {
                        self.check_stmt(stmt_id, module)?;
                    }
                }
            }

            StmtKind::IndexDelete { obj, index } => {
                // Check object and index expressions
                let _ = self.infer_expr_type(*obj, module);
                let _ = self.infer_expr_type(*index, module);
            }
        }

        Ok(())
    }

    /// Check that an expression can be used as a boolean condition
    fn check_condition(&mut self, expr_id: ExprId, module: &Module) -> Result<()> {
        let cond_type = self.infer_expr_type(expr_id, module);

        // In Python, any type can be used as a condition (truthy/falsy)
        // But we emit a warning for some suspicious cases
        // For now, just allow everything (Python semantics)
        let _ = cond_type;
        Ok(())
    }
}
