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
use pyaot_utils::VarId;

use super::{WhileLoopGenerator, YieldSection};

/// If `stmt` is a yield statement — either a bare `yield expr` (`Expr(Yield)`)
/// or a plain-variable assignment `var = yield expr` (`Bind { Var, Yield }`) —
/// return `(yield_value, assignment_target)`. Tuple-pattern targets and yields
/// nested inside larger expressions are intentionally not matched.
fn match_yield_stmt(
    stmt: &hir::Stmt,
    hir_module: &hir::Module,
) -> Option<(Option<hir::ExprId>, Option<VarId>)> {
    match &stmt.kind {
        hir::StmtKind::Expr(expr_id) => match &hir_module.exprs[*expr_id].kind {
            hir::ExprKind::Yield(val) => Some((*val, None)),
            _ => None,
        },
        hir::StmtKind::Bind { target, value, .. } => match &hir_module.exprs[*value].kind {
            hir::ExprKind::Yield(val) => match target {
                hir::BindingTarget::Var(var_id) => Some((*val, Some(*var_id))),
                // A tuple-pattern binding of a yield isn't a standard
                // generator idiom — leave it to the generic resumer.
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Detect if the generator body follows the pattern:
/// [init_stmts...] while cond: yield val; [update_stmts...]
///
/// Pure HIR analysis — no Lowering state needed.
pub(crate) fn detect_while_loop_generator(
    func: &hir::Function,
    hir_module: &hir::Module,
) -> Option<WhileLoopGenerator> {
    let entry = func.blocks.get(&func.entry_block)?;
    let init_stmts = entry.stmts.clone();
    let header_bb = match entry.terminator {
        hir::HirTerminator::Jump(target) => target,
        _ => return None,
    };
    let header = func.blocks.get(&header_bb)?;
    if !header.stmts.is_empty() {
        return None;
    }
    let (cond, body_bb, exit_bb) = match header.terminator {
        hir::HirTerminator::Branch {
            cond,
            then_bb,
            else_bb,
        } => (cond, then_bb, else_bb),
        _ => return None,
    };
    let body = func.blocks.get(&body_bb)?;
    match body.terminator {
        hir::HirTerminator::Jump(target) if target == header_bb => {}
        _ => return None,
    }
    let exit = func.blocks.get(&exit_bb)?;
    if !exit.stmts.is_empty() || !matches!(exit.terminator, hir::HirTerminator::Return(None)) {
        return None;
    }

    let mut yield_sections = Vec::new();
    let mut current_stmts = Vec::new();

    for stmt_id in &body.stmts {
        let stmt = &hir_module.stmts[*stmt_id];
        // Recognize both `yield expr` and `var = yield expr` as yield
        // boundaries; the latter records its assignment target so the resume
        // state machine can deliver the sent value (`send()` round-trip).
        if let Some((yield_expr, assignment_target)) = match_yield_stmt(stmt, hir_module) {
            yield_sections.push(YieldSection {
                stmts_before: current_stmts.clone(),
                yield_expr,
                assignment_target,
            });
            current_stmts.clear();
        } else {
            current_stmts.push(*stmt_id);
        }
    }

    // Statements after last yield become update section
    let update_stmts = current_stmts;

    if yield_sections.is_empty() {
        return None;
    }

    // Extract an optional pre-loop yield from `init_stmts`. A generator like
    // `r = yield 0; while True: r = yield r` has its first yield *before* the
    // loop; that yield becomes a dedicated init state. Only the single
    // trailing-yield shape is supported — any other yield placement in init
    // bails to the generic resumer.
    let (init_stmts, init_yield) = extract_init_yield(init_stmts, hir_module)?;

    // The init-yield state machine (CASE 2) currently handles exactly one
    // in-loop yield section with no statements before it. Anything richer
    // would need a full CFG state-machine; leave it to the generic resumer.
    // TODO: support pre-loop yield combined with multiple in-loop yields or
    // pre-yield body statements.
    if init_yield.is_some()
        && (yield_sections.len() != 1 || !yield_sections[0].stmts_before.is_empty())
    {
        return None;
    }

    Some(WhileLoopGenerator {
        init_stmts,
        init_yield,
        cond,
        yield_sections,
        update_stmts,
    })
}

/// Split `init_stmts` into the statements that run before any pre-loop yield
/// and an optional trailing pre-loop yield section. Returns `None` (caller
/// bails to the generic resumer) if init contains a yield that is not the
/// single last statement, or a yield with an unsupported (tuple) target.
fn extract_init_yield(
    mut init_stmts: Vec<hir::StmtId>,
    hir_module: &hir::Module,
) -> Option<(Vec<hir::StmtId>, Option<YieldSection>)> {
    let yield_positions: Vec<usize> = init_stmts
        .iter()
        .enumerate()
        .filter_map(|(i, sid)| match_yield_stmt(&hir_module.stmts[*sid], hir_module).map(|_| i))
        .collect();

    if yield_positions.is_empty() {
        return Some((init_stmts, None));
    }
    // Only `[...non-yield stmts] <single trailing yield>` is supported.
    if yield_positions.len() != 1 || yield_positions[0] != init_stmts.len() - 1 {
        return None;
    }

    let last = init_stmts.pop().expect("yield_positions non-empty");
    let (yield_expr, assignment_target) = match_yield_stmt(&hir_module.stmts[last], hir_module)?;
    Some((
        init_stmts,
        Some(YieldSection {
            stmts_before: Vec::new(),
            yield_expr,
            assignment_target,
        }),
    ))
}
