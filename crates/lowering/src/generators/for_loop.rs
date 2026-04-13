//! For-loop generator pattern detection
//!
//! Detects generators that follow the for-loop pattern:
//! ```python
//! def gen():
//!     for x in iterable:
//!         yield x
//! ```
//! Or with a filter condition:
//! ```python
//! def gen():
//!     for x in iterable:
//!         if cond:
//!             yield x
//! ```

use pyaot_hir as hir;

use super::ForLoopGenerator;

/// Detect a for-loop generator pattern:
/// `for x in iterable: yield expr` or with a filter `if cond: yield expr`.
///
/// Pure HIR analysis — no Lowering state needed.
pub(crate) fn detect_for_loop_generator(
    body: &[hir::StmtId],
    hir_module: &hir::Module,
) -> Option<ForLoopGenerator> {
    // Body must start with a for loop (at least 1 statement)
    if body.is_empty() {
        return None;
    }

    let stmt = &hir_module.stmts[body[0]];
    // Only handle `ForBind` with a simple variable target.
    // Other `BindingTarget` shapes (tuple, attr, index) need the generic
    // generator path and are intentionally not detected here.
    let (target_var, iter_expr, for_body) = match &stmt.kind {
        hir::StmtKind::ForBind {
            target: hir::BindingTarget::Var(target),
            iter,
            body,
            ..
        } => (*target, *iter, body),
        _ => return None,
    };

    // For body should contain exactly one statement
    if for_body.len() != 1 {
        return None;
    }

    let first_stmt = &hir_module.stmts[for_body[0]];

    let (yield_expr, filter_cond) = {
        // Case 1: Direct yield statement
        if let hir::StmtKind::Expr(expr_id) = &first_stmt.kind {
            let expr = &hir_module.exprs[*expr_id];
            if let hir::ExprKind::Yield(val) = &expr.kind {
                (*val, None)
            } else {
                return None;
            }
        }
        // Case 2: If statement wrapping yield (filter condition)
        else if let hir::StmtKind::If {
            cond,
            then_block,
            else_block,
        } = &first_stmt.kind
        {
            if !else_block.is_empty() || then_block.len() != 1 {
                return None;
            }
            let yield_stmt = &hir_module.stmts[then_block[0]];
            if let hir::StmtKind::Expr(expr_id) = &yield_stmt.kind {
                let expr = &hir_module.exprs[*expr_id];
                if let hir::ExprKind::Yield(val) = &expr.kind {
                    (*val, Some(*cond))
                } else {
                    return None;
                }
            } else {
                return None;
            }
        } else {
            return None;
        }
    };

    // Collect trailing yield statements after the for-loop
    let mut trailing_yields = Vec::new();
    for &stmt_id in &body[1..] {
        let trailing_stmt = &hir_module.stmts[stmt_id];
        if let hir::StmtKind::Expr(expr_id) = &trailing_stmt.kind {
            let expr = &hir_module.exprs[*expr_id];
            if let hir::ExprKind::Yield(val) = &expr.kind {
                trailing_yields.push(*val);
            } else {
                // Non-yield statement after for-loop — can't use this pattern
                return None;
            }
        } else {
            return None;
        }
    }

    Some(ForLoopGenerator {
        target_var,
        iter_expr,
        yield_expr,
        filter_cond,
        trailing_yields,
    })
}
