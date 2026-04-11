//! While-loop generator pattern detection
//!
//! Detects generators that follow the while-loop pattern:
//! ```python
//! def gen():
//!     i = 0
//!     while i < n:
//!         yield i
//!         i = i + 1
//! ```

use pyaot_hir as hir;

use super::{WhileLoopGenerator, YieldSection};

/// Detect if the generator body follows the pattern:
/// [init_stmts...] while cond: yield val; [update_stmts...]
///
/// Pure HIR analysis — no Lowering state needed.
pub(crate) fn detect_while_loop_generator(
    body: &[hir::StmtId],
    hir_module: &hir::Module,
) -> Option<WhileLoopGenerator> {
    // Find the while loop
    let mut init_stmts = Vec::new();
    let mut while_stmt_idx = None;

    for (i, stmt_id) in body.iter().enumerate() {
        let stmt = &hir_module.stmts[*stmt_id];
        if matches!(stmt.kind, hir::StmtKind::While { .. }) {
            while_stmt_idx = Some(i);
            break;
        }
        init_stmts.push(*stmt_id);
    }

    let while_idx = while_stmt_idx?;
    let while_stmt_id = body[while_idx];
    let while_stmt = &hir_module.stmts[while_stmt_id];

    let (cond, while_body) = match &while_stmt.kind {
        hir::StmtKind::While { cond, body, .. } => (*cond, body),
        _ => return None,
    };

    // Find all yields in while body and split into sections
    let mut yield_sections = Vec::new();
    let mut current_stmts = Vec::new();

    for stmt_id in while_body {
        let stmt = &hir_module.stmts[*stmt_id];
        match &stmt.kind {
            hir::StmtKind::Expr(expr_id) => {
                let expr = &hir_module.exprs[*expr_id];
                if let hir::ExprKind::Yield(val) = &expr.kind {
                    // Found a yield - save current section
                    yield_sections.push(YieldSection {
                        stmts_before: current_stmts.clone(),
                        yield_expr: *val,
                    });
                    current_stmts.clear();
                } else {
                    current_stmts.push(*stmt_id);
                }
            }
            _ => {
                current_stmts.push(*stmt_id);
            }
        }
    }

    // Statements after last yield become update section
    let update_stmts = current_stmts;

    if yield_sections.is_empty() {
        return None;
    }

    // Make sure there's nothing after the while loop
    if while_idx + 1 < body.len() {
        return None;
    }

    Some(WhileLoopGenerator {
        init_stmts,
        cond,
        yield_sections,
        update_stmts,
    })
}
