//! Loop statements: For (with unpacking support)

use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Convert orelse block for for/while loops
    fn convert_loop_else(&mut self, orelse: Vec<py::Stmt>) -> Result<Vec<StmtId>> {
        let mut else_block = Vec::new();
        for stmt in orelse {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            else_block.extend(pending);
            else_block.push(stmt_id);
        }
        Ok(else_block)
    }

    pub(crate) fn convert_for(&mut self, for_stmt: py::StmtFor, stmt_span: Span) -> Result<StmtId> {
        // Check if target is a tuple (for tuple unpacking: for a, b in items)
        if let py::Expr::Tuple(tuple) = &*for_stmt.target {
            // Check if any element is starred (for a, *rest in items)
            let has_starred = tuple
                .elts
                .iter()
                .any(|elt| matches!(elt, py::Expr::Starred(_)));

            if has_starred {
                // Use parse_unpack_pattern to handle starred expressions
                let (before_star, starred, after_star) =
                    self.parse_unpack_pattern(&tuple.elts, stmt_span)?;

                let iter = self.convert_expr(*for_stmt.iter)?;
                let iter_pending = self.take_pending_stmts();

                let mut body = Vec::new();
                for stmt in for_stmt.body {
                    let stmt_id = self.convert_stmt(stmt)?;
                    let pending = self.take_pending_stmts();
                    body.extend(pending);
                    body.push(stmt_id);
                }

                let else_block = self.convert_loop_else(for_stmt.orelse)?;

                self.pending_stmts = iter_pending;

                Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::ForUnpackStarred {
                        before_star,
                        starred,
                        after_star,
                        iter,
                        body,
                        else_block,
                    },
                    span: stmt_span,
                }))
            } else {
                // No starred expressions - use regular unpacking
                let mut targets = Vec::new();
                for elt in &tuple.elts {
                    targets.push(self.get_or_create_var_from_expr(elt)?);
                }
                let iter = self.convert_expr(*for_stmt.iter)?;
                let iter_pending = self.take_pending_stmts();

                let mut body = Vec::new();
                for stmt in for_stmt.body {
                    let stmt_id = self.convert_stmt(stmt)?;
                    let pending = self.take_pending_stmts();
                    body.extend(pending);
                    body.push(stmt_id);
                }

                let else_block = self.convert_loop_else(for_stmt.orelse)?;

                self.pending_stmts = iter_pending;

                Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::ForUnpack {
                        targets,
                        iter,
                        body,
                        else_block,
                    },
                    span: stmt_span,
                }))
            }
        } else {
            let target_var = self.get_or_create_var_from_expr(&for_stmt.target)?;
            let iter = self.convert_expr(*for_stmt.iter)?;
            // Save pending statements from iterable (will be prepended by parent)
            let iter_pending = self.take_pending_stmts();

            let mut body = Vec::new();
            for stmt in for_stmt.body {
                let stmt_id = self.convert_stmt(stmt)?;
                let pending = self.take_pending_stmts();
                body.extend(pending);
                body.push(stmt_id);
            }

            let else_block = self.convert_loop_else(for_stmt.orelse)?;

            // Restore iterable's pending statements for parent to handle
            self.pending_stmts = iter_pending;

            Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::For {
                    target: target_var,
                    iter,
                    body,
                    else_block,
                },
                span: stmt_span,
            }))
        }
    }
}
