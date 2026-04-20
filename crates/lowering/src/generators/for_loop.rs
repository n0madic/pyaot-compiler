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

fn yield_expr_from_stmt(stmt: &hir::Stmt, hir_module: &hir::Module) -> Option<Option<hir::ExprId>> {
    if let hir::StmtKind::Expr(expr_id) = &stmt.kind {
        if let hir::ExprKind::Yield(val) = &hir_module.exprs[*expr_id].kind {
            return Some(*val);
        }
    }
    None
}

/// Detect a for-loop generator pattern:
/// `for x in iterable: yield expr` or with a filter `if cond: yield expr`.
///
/// Pure HIR analysis — no Lowering state needed.
pub(crate) fn detect_for_loop_generator(
    func: &hir::Function,
    hir_module: &hir::Module,
) -> Option<ForLoopGenerator> {
    let entry = func.blocks.get(&func.entry_block)?;
    if entry.stmts.len() != 1 {
        return None;
    }

    let iter_expr = match &hir_module.stmts[entry.stmts[0]].kind {
        hir::StmtKind::IterSetup { iter } => *iter,
        _ => return None,
    };
    let header_bb = match entry.terminator {
        hir::HirTerminator::Jump(target) => target,
        _ => return None,
    };

    let header = func.blocks.get(&header_bb)?;
    if !header.stmts.is_empty() {
        return None;
    }
    let (body_bb, exit_bb) = match header.terminator {
        hir::HirTerminator::Branch {
            cond,
            then_bb,
            else_bb,
        } => match &hir_module.exprs[cond].kind {
            hir::ExprKind::IterHasNext(iter) if *iter == iter_expr => (then_bb, else_bb),
            _ => return None,
        },
        _ => return None,
    };

    let body = func.blocks.get(&body_bb)?;
    if body.stmts.is_empty() {
        return None;
    }
    let (target, advance_iter) = match &hir_module.stmts[body.stmts[0]].kind {
        hir::StmtKind::IterAdvance { iter, target } => (target.clone(), *iter),
        _ => return None,
    };
    if advance_iter != iter_expr {
        return None;
    }

    let (yield_expr, filter_cond) = if body.stmts.len() == 2 {
        let yield_stmt = &hir_module.stmts[body.stmts[1]];
        let yield_expr = yield_expr_from_stmt(yield_stmt, hir_module)?;
        match body.terminator {
            hir::HirTerminator::Jump(target) if target == header_bb => (yield_expr, None),
            _ => return None,
        }
    } else if body.stmts.len() == 1 {
        let (cond, then_bb, else_bb) = match body.terminator {
            hir::HirTerminator::Branch {
                cond,
                then_bb,
                else_bb,
            } => (cond, then_bb, else_bb),
            _ => return None,
        };
        let then_block = func.blocks.get(&then_bb)?;
        let else_block = func.blocks.get(&else_bb)?;
        let merge_bb = match (then_block.terminator.clone(), else_block.terminator.clone()) {
            (hir::HirTerminator::Jump(then_merge), hir::HirTerminator::Jump(else_merge))
                if then_merge == else_merge && else_block.stmts.is_empty() =>
            {
                then_merge
            }
            _ => return None,
        };
        let merge = func.blocks.get(&merge_bb)?;
        if !merge.stmts.is_empty() {
            return None;
        }
        match merge.terminator {
            hir::HirTerminator::Jump(target) if target == header_bb => {}
            _ => return None,
        }
        if then_block.stmts.len() != 1 {
            return None;
        }
        let yield_expr = yield_expr_from_stmt(&hir_module.stmts[then_block.stmts[0]], hir_module)?;
        (yield_expr, Some(cond))
    } else {
        return None;
    };

    let mut trailing_yields = Vec::new();
    let exit = func.blocks.get(&exit_bb)?;
    for &stmt_id in &exit.stmts {
        trailing_yields.push(yield_expr_from_stmt(&hir_module.stmts[stmt_id], hir_module)?);
    }
    if !matches!(exit.terminator, hir::HirTerminator::Return(None)) {
        return None;
    }

    Some(ForLoopGenerator {
        target,
        iter_expr,
        yield_expr,
        filter_cond,
        trailing_yields,
    })
}
