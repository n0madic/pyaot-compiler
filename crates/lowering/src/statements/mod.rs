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
        match &stmt.kind {
            // Expression statement
            hir::StmtKind::Expr(expr_id) => {
                let expr = &hir_module.exprs[*expr_id];
                self.lower_expr(expr, hir_module, mir_func)?;
            }

            // Assignments (assign.rs)
            hir::StmtKind::Assign {
                target,
                value,
                type_hint,
            } => {
                self.lower_assign(*target, *value, type_hint.clone(), hir_module, mir_func)?;
            }
            hir::StmtKind::UnpackAssign {
                before_star,
                starred,
                after_star,
                value,
            } => {
                self.lower_unpack_assign(
                    before_star,
                    starred.as_ref(),
                    after_star,
                    *value,
                    hir_module,
                    mir_func,
                )?;
            }
            hir::StmtKind::NestedUnpackAssign { targets, value } => {
                self.lower_nested_unpack_assign(targets, *value, hir_module, mir_func)?;
            }
            hir::StmtKind::IndexAssign { obj, index, value } => {
                self.lower_index_assign(*obj, *index, *value, hir_module, mir_func)?;
            }
            hir::StmtKind::FieldAssign { obj, field, value } => {
                self.lower_field_assign(*obj, *field, *value, hir_module, mir_func)?;
            }
            hir::StmtKind::ClassAttrAssign {
                class_id,
                attr,
                value,
            } => {
                self.lower_class_attr_assign(*class_id, *attr, *value, hir_module, mir_func)?;
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

            // Loops (loops.rs)
            hir::StmtKind::For {
                target,
                iter,
                body,
                else_block,
            } => {
                self.lower_for(*target, *iter, body, else_block, hir_module, mir_func)?;
            }
            hir::StmtKind::ForUnpack {
                targets,
                iter,
                body,
                else_block,
            } => {
                self.lower_for_unpack(targets, *iter, body, else_block, hir_module, mir_func)?;
            }
            hir::StmtKind::ForUnpackStarred {
                before_star,
                starred,
                after_star,
                iter,
                body,
                else_block,
            } => {
                self.lower_for_unpack_starred_dispatch(
                    before_star,
                    starred.as_ref(),
                    after_star,
                    *iter,
                    body,
                    else_block,
                    hir_module,
                    mir_func,
                )?;
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
        }
        Ok(())
    }
}
