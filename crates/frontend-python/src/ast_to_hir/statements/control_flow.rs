//! Control flow statements: If, While, Return, Break, Continue, Pass, Delete

use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_return(
        &mut self,
        ret: py::StmtReturn,
        stmt_span: Span,
    ) -> Result<StmtId> {
        let val = if let Some(v) = ret.value {
            Some(self.convert_expr(*v)?)
        } else {
            None
        };

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Return(val),
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_if(&mut self, if_stmt: py::StmtIf, stmt_span: Span) -> Result<StmtId> {
        let cond = self.convert_expr(*if_stmt.test)?;
        // Save pending statements from condition (will be prepended by parent)
        let cond_pending = self.take_pending_stmts();

        let mut then_block = Vec::new();
        for stmt in if_stmt.body {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            then_block.extend(pending);
            then_block.push(stmt_id);
        }

        let mut else_block = Vec::new();
        for stmt in if_stmt.orelse {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            else_block.extend(pending);
            else_block.push(stmt_id);
        }

        // Restore condition's pending statements for parent to handle
        self.pending_stmts = cond_pending;

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::If {
                cond,
                then_block,
                else_block,
            },
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_while(
        &mut self,
        while_stmt: py::StmtWhile,
        stmt_span: Span,
    ) -> Result<StmtId> {
        let cond = self.convert_expr(*while_stmt.test)?;
        // Save pending statements from condition (will be prepended by parent)
        let cond_pending = self.take_pending_stmts();

        let mut body = Vec::new();
        for stmt in while_stmt.body {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            body.extend(pending);
            body.push(stmt_id);
        }

        let mut else_block = Vec::new();
        for stmt in while_stmt.orelse {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            else_block.extend(pending);
            else_block.push(stmt_id);
        }

        // Restore condition's pending statements for parent to handle
        self.pending_stmts = cond_pending;

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::While {
                cond,
                body,
                else_block,
            },
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_break(&mut self, stmt_span: Span) -> Result<StmtId> {
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Break,
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_continue(&mut self, stmt_span: Span) -> Result<StmtId> {
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Continue,
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_pass(&mut self, stmt_span: Span) -> Result<StmtId> {
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Pass,
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_delete(
        &mut self,
        delete_stmt: py::StmtDelete,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Handle del statement: del obj[key], del obj[index]
        // For multiple targets (del a, b, c), generate pending stmts for all but last
        let targets = delete_stmt.targets;

        if targets.is_empty() {
            return Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::Pass,
                span: stmt_span,
            }));
        }

        // Generate IndexDelete for each subscript target
        for target in targets.iter().take(targets.len().saturating_sub(1)) {
            let stmt = self.convert_delete_target(target, stmt_span)?;
            self.pending_stmts.push(stmt);
        }

        // Last target is returned directly
        let last = targets.last().expect("targets is non-empty");
        self.convert_delete_target(last, stmt_span)
    }

    fn convert_delete_target(&mut self, target: &py::Expr, stmt_span: Span) -> Result<StmtId> {
        match target {
            py::Expr::Subscript(sub) => {
                let obj_expr = self.convert_expr(*sub.value.clone())?;
                let index_expr = self.convert_expr(*sub.slice.clone())?;
                Ok(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::IndexDelete {
                        obj: obj_expr,
                        index: index_expr,
                    },
                    span: stmt_span,
                }))
            }
            _ => Err(CompilerError::parse_error(
                "del is only supported for indexed targets (del obj[key])",
                stmt_span,
            )),
        }
    }
}
