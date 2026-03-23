//! Assignment statements: Assign, AnnAssign, AugAssign

use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_types::Type;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_assign(
        &mut self,
        assign: py::StmtAssign,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Chained assignment: a = b = value → b = value; a = value
        // We process right-to-left (Python semantics: value is evaluated once)
        if assign.targets.len() > 1 {
            return self.convert_chained_assign(assign, stmt_span);
        }

        let target = &assign.targets[0];

        // Check if this is an attribute assignment: obj.field = value or ClassName.attr = value
        if let py::Expr::Attribute(attr) = target {
            // Check if the base is a class name (class attribute assignment)
            if let py::Expr::Name(base_name) = &*attr.value {
                let base_str = self.interner.intern(&base_name.id);
                if let Some(&class_id) = self.class_map.get(&base_str) {
                    // This is a class attribute assignment: ClassName.attr = value
                    let attr_name = self.interner.intern(&attr.attr);
                    let value = self.convert_expr(*assign.value)?;

                    return Ok(self.module.stmts.alloc(Stmt {
                        kind: StmtKind::ClassAttrAssign {
                            class_id,
                            attr: attr_name,
                            value,
                        },
                        span: stmt_span,
                    }));
                }
            }

            // Regular instance field assignment
            let obj_expr = self.convert_expr(*attr.value.clone())?;
            let field_name = self.interner.intern(&attr.attr);
            let value = self.convert_expr(*assign.value)?;

            return Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::FieldAssign {
                    obj: obj_expr,
                    field: field_name,
                    value,
                },
                span: stmt_span,
            }));
        // Check if this is an indexed assignment: obj[index] = value
        } else if let py::Expr::Subscript(sub) = target {
            let obj_expr = self.convert_expr(*sub.value.clone())?;
            let index_expr = self.convert_expr(*sub.slice.clone())?;
            let value = self.convert_expr(*assign.value)?;

            return Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::IndexAssign {
                    obj: obj_expr,
                    index: index_expr,
                    value,
                },
                span: stmt_span,
            }));
        } else if let py::Expr::Tuple(tuple) = target {
            // Check if pattern has nested tuples/lists or starred expressions
            let has_nested = tuple
                .elts
                .iter()
                .any(|elt| matches!(elt, py::Expr::Tuple(_) | py::Expr::List(_)));
            let has_starred = tuple
                .elts
                .iter()
                .any(|elt| matches!(elt, py::Expr::Starred(_)));

            if has_nested && !has_starred {
                // Nested unpacking: (a, (b, c)) = value
                self.mark_vars_initialized(&tuple.elts);
                let targets = self.parse_nested_unpack_pattern(&tuple.elts)?;
                let value = self.convert_expr(*assign.value)?;

                return Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::NestedUnpackAssign { targets, value },
                    span: stmt_span,
                }));
            } else {
                // Simple tuple unpacking or starred unpacking: a, b, c = (1, 2, 3) or a, *rest = (1, 2, 3)
                self.mark_vars_initialized(&tuple.elts);
                let (before_star, starred, after_star) =
                    self.parse_unpack_pattern(&tuple.elts, stmt_span)?;
                let value = self.convert_expr(*assign.value)?;

                return Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::UnpackAssign {
                        before_star,
                        starred,
                        after_star,
                        value,
                    },
                    span: stmt_span,
                }));
            }
        } else if let py::Expr::List(list) = target {
            // Check if pattern has nested tuples/lists or starred expressions
            let has_nested = list
                .elts
                .iter()
                .any(|elt| matches!(elt, py::Expr::Tuple(_) | py::Expr::List(_)));
            let has_starred = list
                .elts
                .iter()
                .any(|elt| matches!(elt, py::Expr::Starred(_)));

            if has_nested && !has_starred {
                // Nested unpacking: [a, [b, c]] = value
                self.mark_vars_initialized(&list.elts);
                let targets = self.parse_nested_unpack_pattern(&list.elts)?;
                let value = self.convert_expr(*assign.value)?;

                return Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::NestedUnpackAssign { targets, value },
                    span: stmt_span,
                }));
            } else {
                // List unpacking: [a, b, c] = [1, 2, 3] or [a, *rest] = [1, 2, 3]
                self.mark_vars_initialized(&list.elts);
                let (before_star, starred, after_star) =
                    self.parse_unpack_pattern(&list.elts, stmt_span)?;
                let value = self.convert_expr(*assign.value)?;

                return Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::UnpackAssign {
                        before_star,
                        starred,
                        after_star,
                        value,
                    },
                    span: stmt_span,
                }));
            }
        }

        // Simple variable assignment
        self.mark_var_initialized(target);
        let target_var = self.get_or_create_var_from_expr(target)?;
        let value = self.convert_expr(*assign.value)?;

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: target_var,
                value,
                type_hint: None,
            },
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_ann_assign(
        &mut self,
        ann_assign: py::StmtAnnAssign,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Annotated assignment: target: Type = value
        self.mark_var_initialized(&ann_assign.target);
        let target_var = self.get_or_create_var_from_expr(&ann_assign.target)?;
        let type_hint = self.convert_type_annotation(&ann_assign.annotation)?;
        let value = if let Some(val) = ann_assign.value {
            let value_id = self.convert_expr(*val)?;
            // Propagate type hint to the value expression for empty collection literals
            // This allows type inference to use the annotation for empty list/dict/set
            let value_expr = &mut self.module.exprs[value_id];
            if value_expr.ty.is_none() {
                value_expr.ty = Some(type_hint.clone());
            }
            value_id
        } else {
            // Just declaration without value
            self.module.exprs.alloc(Expr {
                kind: ExprKind::None,
                ty: Some(Type::None),
                span: stmt_span,
            })
        };

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: target_var,
                value,
                type_hint: Some(type_hint),
            },
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_aug_assign(
        &mut self,
        aug_assign: py::StmtAugAssign,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Augmented assignment: target op= value → target = target op value
        // Examples: x += 5 → x = x + 5
        //           obj.field -= 1 → obj.field = obj.field - 1
        //           list[i] *= 2 → list[i] = list[i] * 2

        let binop = self.convert_binop(&aug_assign.op, stmt_span)?;
        let target_ref = &*aug_assign.target;

        // Handle different target types
        if let py::Expr::Name(name) = target_ref {
            // Simple variable: x += 5 → x = x + 5
            let var_name = self.interner.intern(&name.id);
            let target_var = if let Some(&id) = self.var_map.get(&var_name) {
                id
            } else {
                return Err(CompilerError::parse_error(
                    format!(
                        "Augmented assignment to undefined variable: {}",
                        self.interner.resolve(var_name)
                    ),
                    stmt_span,
                ));
            };

            // Create target expr (read the current value)
            let target_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(target_var),
                ty: None,
                span: stmt_span,
            });

            // Convert the RHS value
            let value_expr = self.convert_expr(*aug_assign.value)?;

            // Create the BinOp expression: target op value
            let binop_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::BinOp {
                    op: binop,
                    left: target_expr,
                    right: value_expr,
                },
                ty: None,
                span: stmt_span,
            });

            Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::Assign {
                    target: target_var,
                    value: binop_expr,
                    type_hint: None,
                },
                span: stmt_span,
            }))
        } else if let py::Expr::Attribute(attr) = target_ref {
            // Check if this is a class attribute augmented assignment: ClassName.attr += 5
            if let py::Expr::Name(base_name) = &*attr.value {
                let base_str = self.interner.intern(&base_name.id);
                if let Some(&class_id) = self.class_map.get(&base_str) {
                    // This is a class attribute augmented assignment
                    let attr_name = self.interner.intern(&attr.attr);

                    // Create class attr read for current value
                    let attr_read = self.module.exprs.alloc(Expr {
                        kind: ExprKind::ClassAttrRef {
                            class_id,
                            attr: attr_name,
                        },
                        ty: None,
                        span: stmt_span,
                    });

                    // Convert the RHS value
                    let value_expr = self.convert_expr(*aug_assign.value.clone())?;

                    // Create the BinOp expression
                    let binop_expr = self.module.exprs.alloc(Expr {
                        kind: ExprKind::BinOp {
                            op: binop,
                            left: attr_read,
                            right: value_expr,
                        },
                        ty: None,
                        span: stmt_span,
                    });

                    return Ok(self.module.stmts.alloc(Stmt {
                        kind: StmtKind::ClassAttrAssign {
                            class_id,
                            attr: attr_name,
                            value: binop_expr,
                        },
                        span: stmt_span,
                    }));
                }
            }

            // Regular instance field augmented assignment: obj.field += 5 → obj.field = obj.field + 5
            let obj_expr = self.convert_expr(*attr.value.clone())?;
            let field_name = self.interner.intern(&attr.attr);

            // Create attribute access for reading current value
            let attr_read = self.module.exprs.alloc(Expr {
                kind: ExprKind::Attribute {
                    obj: obj_expr,
                    attr: field_name,
                },
                ty: None,
                span: stmt_span,
            });

            // Convert the RHS value
            let value_expr = self.convert_expr(*aug_assign.value)?;

            // Create the BinOp expression
            let binop_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::BinOp {
                    op: binop,
                    left: attr_read,
                    right: value_expr,
                },
                ty: None,
                span: stmt_span,
            });

            Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::FieldAssign {
                    obj: obj_expr,
                    field: field_name,
                    value: binop_expr,
                },
                span: stmt_span,
            }))
        } else if let py::Expr::Subscript(sub) = target_ref {
            // Indexed: list[i] += 5 → list[i] = list[i] + 5
            let obj_expr = self.convert_expr(*sub.value.clone())?;
            let index_expr = self.convert_expr(*sub.slice.clone())?;

            // Create index access for reading current value
            let index_read = self.module.exprs.alloc(Expr {
                kind: ExprKind::Index {
                    obj: obj_expr,
                    index: index_expr,
                },
                ty: None,
                span: stmt_span,
            });

            // Convert the RHS value
            let value_expr = self.convert_expr(*aug_assign.value)?;

            // Create the BinOp expression
            let binop_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::BinOp {
                    op: binop,
                    left: index_read,
                    right: value_expr,
                },
                ty: None,
                span: stmt_span,
            });

            Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::IndexAssign {
                    obj: obj_expr,
                    index: index_expr,
                    value: binop_expr,
                },
                span: stmt_span,
            }))
        } else {
            Err(CompilerError::parse_error(
                format!(
                    "Unsupported augmented assignment target: {:?}",
                    aug_assign.target
                ),
                stmt_span,
            ))
        }
    }

    /// Create an assignment statement for any target type (Name, Attribute, Subscript).
    /// Used by chained assignments and other multi-target scenarios.
    fn assign_to_target(
        &mut self,
        target: &py::Expr,
        value_expr: ExprId,
        span: Span,
    ) -> Result<StmtId> {
        match target {
            py::Expr::Name(_) => {
                self.mark_var_initialized(target);
                let target_var = self.get_or_create_var_from_expr(target)?;
                Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Assign {
                        target: target_var,
                        value: value_expr,
                        type_hint: None,
                    },
                    span,
                }))
            }
            py::Expr::Attribute(attr) => {
                // Check for class attribute assignment: ClassName.attr = value
                if let py::Expr::Name(base_name) = &*attr.value {
                    let base_str = self.interner.intern(&base_name.id);
                    if let Some(&class_id) = self.class_map.get(&base_str) {
                        let attr_name = self.interner.intern(&attr.attr);
                        return Ok(self.module.stmts.alloc(Stmt {
                            kind: StmtKind::ClassAttrAssign {
                                class_id,
                                attr: attr_name,
                                value: value_expr,
                            },
                            span,
                        }));
                    }
                }
                // Regular field assignment: obj.field = value
                let obj_expr = self.convert_expr(*attr.value.clone())?;
                let field_name = self.interner.intern(&attr.attr);
                Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::FieldAssign {
                        obj: obj_expr,
                        field: field_name,
                        value: value_expr,
                    },
                    span,
                }))
            }
            py::Expr::Subscript(sub) => {
                let obj_expr = self.convert_expr(*sub.value.clone())?;
                let index_expr = self.convert_expr(*sub.slice.clone())?;
                Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::IndexAssign {
                        obj: obj_expr,
                        index: index_expr,
                        value: value_expr,
                    },
                    span,
                }))
            }
            _ => Err(CompilerError::parse_error(
                "Unsupported chained assignment target",
                span,
            )),
        }
    }

    /// Convert chained assignment: a = b = value
    /// Python evaluates the value once, then assigns right-to-left.
    /// We generate: tmp = value; b = tmp; a = tmp
    fn convert_chained_assign(
        &mut self,
        assign: py::StmtAssign,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Evaluate value once and assign to a temporary variable
        let value = self.convert_expr(*assign.value)?;
        let temp_var = self.alloc_var_id();
        let temp_assign = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: temp_var,
                value,
                type_hint: None,
            },
            span: stmt_span,
        });
        self.pending_stmts.push(temp_assign);

        // Assign to each target right-to-left
        // Python: a = b = 42 → targets = [a, b], value = 42
        // We assign right-to-left: b = temp, then a = temp
        // All except the first (leftmost) become pending statements
        let targets: Vec<_> = assign.targets;
        for target in targets[1..].iter().rev() {
            let temp_ref = self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(temp_var),
                ty: None,
                span: stmt_span,
            });
            let assign_stmt = self.assign_to_target(target, temp_ref, stmt_span)?;
            self.pending_stmts.push(assign_stmt);
        }

        // First (leftmost) target is the returned statement
        let first_target = &targets[0];
        let temp_ref = self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(temp_var),
            ty: None,
            span: stmt_span,
        });
        self.assign_to_target(first_target, temp_ref, stmt_span)
    }
}
