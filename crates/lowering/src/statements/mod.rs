//! Statement lowering from HIR to MIR
//!
//! This module handles lowering of all statement types from HIR to MIR.
//! It is organized into submodules by statement category:
//! - `assign`: Assign, UnpackAssign, IndexAssign, FieldAssign
//! - `control_flow`: Return, If, While, Break, Continue, Pass
//! - `loops`: For (range and iterable iteration)
//! - `assert`: Assert

mod assert;
mod assign;
mod control_flow;
mod loops;
mod match_stmt;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Main entry point for lowering a statement.
    /// Dispatches to appropriate submodule based on statement kind.
    pub(crate) fn lower_stmt(
        &mut self,
        stmt: &hir::Stmt,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        self.codegen.current_span = Some(stmt.span);
        match &stmt.kind {
            // Expression statement
            hir::StmtKind::Expr(expr_id) => {
                let expr = &hir_module.exprs[*expr_id];
                self.lower_expr(expr, hir_module, mir_func)?;
            }

            // Control flow (control_flow.rs)
            hir::StmtKind::Return(value_expr) => {
                self.lower_return(value_expr.as_ref(), hir_module, mir_func)?;
            }
            hir::StmtKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.lower_if(*cond, then_block, else_block, hir_module, mir_func)?;
            }
            hir::StmtKind::While {
                cond,
                body,
                else_block,
            } => {
                self.lower_while(*cond, body, else_block, hir_module, mir_func)?;
            }
            hir::StmtKind::Break => {
                self.lower_break();
            }
            hir::StmtKind::Continue => {
                self.lower_continue();
            }
            hir::StmtKind::Pass => {
                // No-op
            }

            // Delete (del obj[key])
            hir::StmtKind::IndexDelete { obj, index } => {
                self.lower_index_delete(*obj, *index, hir_module, mir_func)?;
            }

            // Assert (assert.rs)
            hir::StmtKind::Assert { cond, msg } => {
                self.lower_assert(*cond, msg.as_ref(), hir_module, mir_func)?;
            }

            // Match statement (match_stmt.rs)
            hir::StmtKind::Match { subject, cases } => {
                self.lower_match(*subject, cases, hir_module, mir_func)?;
            }

            // Exceptions (exceptions.rs - already separate)
            hir::StmtKind::Raise { exc, cause } => {
                self.lower_raise(exc, cause, hir_module, mir_func)?;
            }
            hir::StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                self.lower_try(
                    body,
                    handlers,
                    else_block,
                    finally_block,
                    hir_module,
                    mir_func,
                )?;
            }

            // Unified binding: assign/bind.rs
            hir::StmtKind::Bind {
                target,
                value,
                type_hint,
            } => {
                // For plain variable targets, delegate to lower_assign which handles all
                // the special cases: globals, cell variables, union boxing, FuncRef/Closure
                // tracking, dict in-place update, and bidirectional type propagation.
                if let hir::BindingTarget::Var(var_id) = target {
                    self.lower_assign(*var_id, *value, type_hint.clone(), hir_module, mir_func)?;
                } else {
                    let value_expr = &hir_module.exprs[*value];
                    let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;
                    let value_type = self.get_type_of_expr_id(*value, hir_module);
                    self.lower_binding_target(
                        target,
                        value_operand,
                        &value_type,
                        hir_module,
                        mir_func,
                    )?;
                }
            }
            hir::StmtKind::ForBind {
                target,
                iter,
                body,
                else_block,
            } => {
                self.lower_for_bind(target, *iter, body, else_block, hir_module, mir_func)?;
            }
        }
        Ok(())
    }
}
