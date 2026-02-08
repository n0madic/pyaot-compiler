//! Exception handling: raise, try/except/finally

use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_types::{BuiltinExceptionKind, Type};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

/// Convert a built-in exception name to its Type variant.
/// Uses BuiltinExceptionKind from the unified exception system.
/// Returns None if the name is not a built-in exception.
fn builtin_exception_name_to_type(name: &str) -> Option<Type> {
    BuiltinExceptionKind::from_name(name).map(Type::BuiltinException)
}

impl AstToHir {
    /// Resolve a single exception type from an AST expression.
    /// Handles built-in exceptions and user-defined exception classes.
    fn resolve_exception_type(&mut self, expr: &py::Expr) -> Type {
        match expr {
            py::Expr::Name(name) => {
                let name_str = name.id.as_str();
                // First check if it's a built-in exception
                if let Some(exc_ty) = builtin_exception_name_to_type(name_str) {
                    exc_ty
                } else {
                    // Check if it's a user-defined exception class
                    let class_name = self.interner.intern(&name.id);
                    if let Some(&class_id) = self.class_map.get(&class_name) {
                        if let Some(class_def) = self.module.class_defs.get(&class_id) {
                            if class_def.is_exception_class {
                                return Type::Class {
                                    class_id,
                                    name: class_name,
                                };
                            }
                        }
                    }
                    // Fallback to base Exception
                    Type::BuiltinException(BuiltinExceptionKind::Exception)
                }
            }
            _ => Type::BuiltinException(BuiltinExceptionKind::Exception),
        }
    }

    pub(crate) fn convert_raise(
        &mut self,
        raise_stmt: py::StmtRaise,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Handle raise statement: raise, raise Exception("message"), or raise X from Y
        let exc = if let Some(exc_expr) = raise_stmt.exc {
            Some(self.convert_expr(*exc_expr)?)
        } else {
            None
        };
        let cause = if let Some(cause_expr) = raise_stmt.cause {
            Some(self.convert_expr(*cause_expr)?)
        } else {
            None
        };

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Raise { exc, cause },
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_try(&mut self, try_stmt: py::StmtTry, stmt_span: Span) -> Result<StmtId> {
        // Convert try body
        let mut body = Vec::new();
        for stmt in try_stmt.body {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            body.extend(pending);
            body.push(stmt_id);
        }

        // Convert exception handlers
        let mut handlers = Vec::new();
        for handler in try_stmt.handlers {
            let py::ExceptHandler::ExceptHandler(h) = handler;

            // Convert exception variable name (if specified) BEFORE converting body
            // so the variable is available when expressions in the body reference it
            let name = if let Some(name_str) = &h.name {
                let interned = self.interner.intern(name_str);
                let var_id = if let Some(&id) = self.var_map.get(&interned) {
                    id
                } else {
                    let id = self.alloc_var_id();
                    self.var_map.insert(interned, id);
                    id
                };
                Some(var_id)
            } else {
                None
            };

            // Convert handler body (now the exception variable is in scope)
            let mut handler_body = Vec::new();
            for stmt in h.body {
                let stmt_id = self.convert_stmt(stmt)?;
                let pending = self.take_pending_stmts();
                handler_body.extend(pending);
                handler_body.push(stmt_id);
            }

            // Convert exception type (if specified)
            // For tuple of types: except (ValueError, TypeError) as e:
            // we expand into multiple handlers sharing the same body and name
            if let Some(type_expr) = h.type_ {
                match type_expr.as_ref() {
                    py::Expr::Tuple(tuple) => {
                        // Multiple exception types: except (ValueError, TypeError) as e:
                        // Expand into multiple handlers with the same body and name
                        for elt in &tuple.elts {
                            let exc_type = self.resolve_exception_type(elt);
                            handlers.push(ExceptHandler {
                                ty: Some(exc_type),
                                name,
                                body: handler_body.clone(),
                            });
                        }
                    }
                    _ => {
                        let exc_type = self.resolve_exception_type(&type_expr);
                        handlers.push(ExceptHandler {
                            ty: Some(exc_type),
                            name,
                            body: handler_body,
                        });
                    }
                }
            } else {
                handlers.push(ExceptHandler {
                    ty: None,
                    name,
                    body: handler_body,
                });
            }
        }

        // Convert else block (runs if no exception raised in try body)
        let mut else_block = Vec::new();
        for stmt in try_stmt.orelse {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            else_block.extend(pending);
            else_block.push(stmt_id);
        }

        // Convert finally block
        let mut finally_block = Vec::new();
        for stmt in try_stmt.finalbody {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            finally_block.extend(pending);
            finally_block.push(stmt_id);
        }

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            },
            span: stmt_span,
        }))
    }
}
