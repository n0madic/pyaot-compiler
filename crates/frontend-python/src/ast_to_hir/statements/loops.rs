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
        let target = self.bind_target(&for_stmt.target)?;
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
        self.scope.pending_stmts = iter_pending;

        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::ForBind {
                target,
                iter,
                body,
                else_block,
            },
            span: stmt_span,
        }))
    }
}
