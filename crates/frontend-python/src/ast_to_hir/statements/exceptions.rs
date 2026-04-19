//! Exception handling: raise, try/except/finally

use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_types::{BuiltinExceptionKind, Type};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Resolve a single exception type from an AST expression.
    /// Handles built-in exceptions, scoped stdlib exceptions imported by the
    /// user, and user-defined exception classes.
    fn resolve_exception_type(&mut self, expr: &py::Expr) -> Type {
        match expr {
            py::Expr::Name(name) => {
                let name_str = name.id.as_str();
                let interned = self.interner.intern(&name.id);

                // Python built-in exceptions resolve globally by name.
                // Stdlib exceptions from submodules (e.g. HTTPError) are NOT
                // here — they're registered as synthetic classes by
                // `imports.rs::register_stdlib_exception` and resolved
                // through the class_map path below, matching CPython's
                // requirement that they be explicitly imported.
                if let Some(kind) = BuiltinExceptionKind::from_name(name_str) {
                    return Type::BuiltinException(kind);
                }

                // Class-bound lookup covers both user-defined exception
                // classes AND imported stdlib exception classes (same
                // mechanism).
                if let Some(&class_id) = self.symbols.class_map.get(&interned) {
                    if let Some(class_def) = self.module.class_defs.get(&class_id) {
                        if class_def.is_exception_class {
                            return Type::Class {
                                class_id,
                                name: interned,
                            };
                        }
                    }
                }

                // Fallback to base Exception
                Type::BuiltinException(BuiltinExceptionKind::Exception)
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
                let var_id = if let Some(&id) = self.symbols.var_map.get(&interned) {
                    id
                } else {
                    let id = self.ids.alloc_var();
                    self.symbols.var_map.insert(interned, id);
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
                                entry_block: pyaot_utils::HirBlockId::new(0),
                            });
                        }
                    }
                    _ => {
                        let exc_type = self.resolve_exception_type(&type_expr);
                        handlers.push(ExceptHandler {
                            ty: Some(exc_type),
                            name,
                            body: handler_body,
                            entry_block: pyaot_utils::HirBlockId::new(0),
                        });
                    }
                }
            } else {
                handlers.push(ExceptHandler {
                    ty: None,
                    name,
                    body: handler_body,
                    entry_block: pyaot_utils::HirBlockId::new(0),
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
