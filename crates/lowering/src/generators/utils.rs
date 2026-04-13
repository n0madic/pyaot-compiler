//! Helper functions for generator analysis
//!
//! Provides utilities for collecting yield information from generator bodies.

use pyaot_hir as hir;
use pyaot_utils::VarId;

use super::YieldInfo;

/// Collect yield information from the function body (in order).
/// Returns YieldInfo for each yield, including assignment targets.
/// Pure HIR analysis — no Lowering state needed.
pub(crate) fn collect_yield_info(body: &[hir::StmtId], hir_module: &hir::Module) -> Vec<YieldInfo> {
    let mut yields = Vec::new();
    for stmt_id in body {
        collect_yields_from_stmt_with_target(*stmt_id, hir_module, &mut yields);
    }
    yields
}

fn collect_yields_from_stmt_with_target(
    stmt_id: hir::StmtId,
    hir_module: &hir::Module,
    yields: &mut Vec<YieldInfo>,
) {
    let stmt = &hir_module.stmts[stmt_id];
    match &stmt.kind {
        hir::StmtKind::Expr(expr_id) => {
            collect_yields_from_expr_with_target(*expr_id, None, hir_module, yields);
        }
        hir::StmtKind::Bind { target, value, .. } => {
            let value_expr = &hir_module.exprs[*value];
            // Only record a yield assignment target for plain Var bindings;
            // tuple-pattern bindings of a yield are not a standard generator idiom.
            let yield_target = if let hir::BindingTarget::Var(var_id) = target {
                if matches!(value_expr.kind, hir::ExprKind::Yield(_)) {
                    Some(*var_id)
                } else {
                    None
                }
            } else {
                None
            };
            collect_yields_from_expr_with_target(*value, yield_target, hir_module, yields);
        }
        hir::StmtKind::Return(Some(expr_id)) => {
            collect_yields_from_expr_with_target(*expr_id, None, hir_module, yields);
        }
        hir::StmtKind::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
            for s in then_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
            for s in else_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
        }
        hir::StmtKind::While {
            cond,
            body,
            else_block,
        } => {
            collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
            for s in body {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
            for s in else_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
        }
        hir::StmtKind::ForBind {
            iter,
            body,
            else_block,
            ..
        } => {
            collect_yields_from_expr_with_target(*iter, None, hir_module, yields);
            for s in body {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
            for s in else_block {
                collect_yields_from_stmt_with_target(*s, hir_module, yields);
            }
        }
        _ => {}
    }
}

fn collect_yields_from_expr_with_target(
    expr_id: hir::ExprId,
    assignment_target: Option<VarId>,
    hir_module: &hir::Module,
    yields: &mut Vec<YieldInfo>,
) {
    let expr = &hir_module.exprs[expr_id];
    match &expr.kind {
        hir::ExprKind::Yield(value) => {
            yields.push(YieldInfo {
                yield_value: *value,
                assignment_target,
            });
        }
        hir::ExprKind::BinOp { left, right, .. } => {
            collect_yields_from_expr_with_target(*left, None, hir_module, yields);
            collect_yields_from_expr_with_target(*right, None, hir_module, yields);
        }
        hir::ExprKind::UnOp { operand, .. } => {
            collect_yields_from_expr_with_target(*operand, None, hir_module, yields);
        }
        hir::ExprKind::Call { func, args, .. } => {
            collect_yields_from_expr_with_target(*func, None, hir_module, yields);
            for a in args {
                let arg_id = match a {
                    hir::CallArg::Regular(id) | hir::CallArg::Starred(id) => id,
                };
                collect_yields_from_expr_with_target(*arg_id, None, hir_module, yields);
            }
        }
        hir::ExprKind::IfExpr {
            cond,
            then_val,
            else_val,
        } => {
            collect_yields_from_expr_with_target(*cond, None, hir_module, yields);
            collect_yields_from_expr_with_target(*then_val, None, hir_module, yields);
            collect_yields_from_expr_with_target(*else_val, None, hir_module, yields);
        }
        _ => {}
    }
}
