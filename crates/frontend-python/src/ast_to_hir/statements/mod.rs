//! Statement conversion from Python AST to HIR
//!
//! Organized into submodules by statement category:
//! - `assign`: Assign, AnnAssign, AugAssign
//! - `control_flow`: If, While, Return, Break, Continue, Pass
//! - `loops`: For, ForUnpack
//! - `nested_functions`: FunctionDef (nested closures)
//! - `imports`: ImportFrom, Import
//! - `exceptions`: Raise, Try
//! - `scope`: Global, Nonlocal
//! - `context_managers`: With

mod assign;
mod context_managers;
mod control_flow;
mod exceptions;
mod imports;
mod loops;
mod match_stmt;
mod nested_functions;
mod scope;

use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{cfg_builder::CfgStmt, *};
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_stmt(&mut self, stmt: py::Stmt) -> Result<CfgStmt> {
        let stmt_span = Self::span_from(&stmt);
        let simple_stmt = match stmt {
            // Inline simple statements
            py::Stmt::Expr(expr_stmt) => {
                // Special case: yield from as a statement — desugar directly to For loop
                // This avoids creating a trailing Expr(None) that would break
                // generator pattern detection (which requires exactly 1 statement in body)
                if let py::Expr::YieldFrom(yield_from) = *expr_stmt.value {
                    self.scope.current_func_is_generator = true;

                    // Convert the iterable expression
                    let iter_expr_id = self.convert_expr(*yield_from.value)?;

                    // Create a temp variable for the loop target
                    let temp_var = self.ids.alloc_var();

                    // Create yield expression: yield __v
                    let var_ref = self.module.exprs.alloc(Expr {
                        kind: ExprKind::Var(temp_var),
                        ty: None,
                        span: stmt_span,
                    });
                    let yield_expr_id = self.module.exprs.alloc(Expr {
                        kind: ExprKind::Yield(Some(var_ref)),
                        ty: None,
                        span: stmt_span,
                    });

                    // Wrap yield in an expression statement
                    let yield_stmt = self.module.stmts.alloc(Stmt {
                        kind: StmtKind::Expr(yield_expr_id),
                        span: stmt_span,
                    });

                    return Ok(CfgStmt::For {
                        target: BindingTarget::Var(temp_var),
                        iter: iter_expr_id,
                        body: vec![CfgStmt::stmt(yield_stmt)],
                        else_body: vec![],
                        span: stmt_span,
                    });
                } else {
                    let expr_id = self.convert_expr(*expr_stmt.value)?;
                    StmtKind::Expr(expr_id)
                }
            }
            py::Stmt::Assert(assert_stmt) => {
                let cond = self.convert_expr(*assert_stmt.test)?;
                let msg = if let Some(msg_expr) = assert_stmt.msg {
                    Some(self.convert_expr(*msg_expr)?)
                } else {
                    None
                };
                StmtKind::Assert { cond, msg }
            }

            // Dispatch to submodules
            py::Stmt::Assign(assign) => {
                return Ok(CfgStmt::stmt(self.convert_assign(assign, stmt_span)?));
            }
            py::Stmt::AnnAssign(ann_assign) => {
                return Ok(CfgStmt::stmt(
                    self.convert_ann_assign(ann_assign, stmt_span)?,
                ));
            }
            py::Stmt::AugAssign(aug_assign) => {
                return Ok(CfgStmt::stmt(
                    self.convert_aug_assign(aug_assign, stmt_span)?,
                ));
            }
            py::Stmt::Return(ret) => {
                return Ok(CfgStmt::stmt(self.convert_return(ret, stmt_span)?));
            }
            py::Stmt::If(if_stmt) => {
                return self.convert_if(if_stmt, stmt_span);
            }
            py::Stmt::While(while_stmt) => {
                return self.convert_while(while_stmt, stmt_span);
            }
            py::Stmt::Break(_) => {
                return Ok(CfgStmt::stmt(self.convert_break(stmt_span)?));
            }
            py::Stmt::Continue(_) => {
                return Ok(CfgStmt::stmt(self.convert_continue(stmt_span)?));
            }
            py::Stmt::Pass(_) => {
                return Ok(CfgStmt::stmt(self.convert_pass(stmt_span)?));
            }
            py::Stmt::For(for_stmt) => {
                return self.convert_for(for_stmt, stmt_span);
            }
            py::Stmt::FunctionDef(func_def) => {
                return self.convert_nested_function_def(func_def, stmt_span);
            }
            py::Stmt::ImportFrom(import_from) => {
                return Ok(CfgStmt::stmt(
                    self.convert_import_from(import_from, stmt_span)?,
                ));
            }
            py::Stmt::Import(import_stmt) => {
                return Ok(CfgStmt::stmt(self.convert_import(import_stmt, stmt_span)?));
            }
            py::Stmt::Raise(raise_stmt) => {
                return Ok(CfgStmt::stmt(self.convert_raise(raise_stmt, stmt_span)?));
            }
            py::Stmt::Try(try_stmt) => {
                return self.convert_try(try_stmt, stmt_span);
            }
            py::Stmt::Global(global_stmt) => {
                return Ok(CfgStmt::stmt(self.convert_global(global_stmt, stmt_span)?));
            }
            py::Stmt::Nonlocal(nonlocal_stmt) => {
                return Ok(CfgStmt::stmt(
                    self.convert_nonlocal(nonlocal_stmt, stmt_span)?,
                ));
            }
            py::Stmt::With(with_stmt) => {
                return self.convert_with(with_stmt, stmt_span);
            }
            py::Stmt::Match(match_stmt) => {
                return self.convert_match(match_stmt, stmt_span);
            }
            py::Stmt::Delete(delete_stmt) => {
                return Ok(CfgStmt::stmt(self.convert_delete(delete_stmt, stmt_span)?));
            }
            py::Stmt::TypeAlias(ta) => {
                // PEP 695: type MyType = int (inside function scope)
                self.convert_type_alias_stmt(ta, stmt_span)?;
                return Ok(CfgStmt::stmt(self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Pass,
                    span: stmt_span,
                })));
            }
            _ => {
                return Err(CompilerError::parse_error(
                    format!("Unsupported statement: {:?}", stmt),
                    stmt_span,
                ))
            }
        };

        Ok(CfgStmt::stmt(self.module.stmts.alloc(Stmt {
            kind: simple_stmt,
            span: stmt_span,
        })))
    }
}
