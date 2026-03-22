//! Variable collection for generator functions
//!
//! This module provides functions to collect all variables that need to persist
//! across yields in a generator function.

use std::collections::HashSet;

use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use super::GeneratorVar;

/// Collect all variables used in a generator function
pub(super) fn collect_generator_vars(
    func: &hir::Function,
    hir_module: &hir::Module,
) -> Vec<GeneratorVar> {
    let mut vars: Vec<GeneratorVar> = Vec::new();
    let mut var_set: HashSet<VarId> = HashSet::new();
    let mut next_idx = 0u32;

    // First add all parameters
    for param in &func.params {
        if !var_set.contains(&param.var) {
            vars.push(GeneratorVar {
                var_id: param.var,
                gen_local_idx: next_idx,
                ty: param.ty.clone().unwrap_or(Type::Any),
                is_param: true,
            });
            var_set.insert(param.var);
            next_idx += 1;
        }
    }

    // Then collect all assigned variables in the body
    for stmt_id in &func.body {
        collect_vars_from_stmt(*stmt_id, hir_module, &mut vars, &mut var_set, &mut next_idx);
    }

    vars
}

fn collect_vars_from_stmt(
    stmt_id: hir::StmtId,
    hir_module: &hir::Module,
    vars: &mut Vec<GeneratorVar>,
    var_set: &mut HashSet<VarId>,
    next_idx: &mut u32,
) {
    let stmt = &hir_module.stmts[stmt_id];
    match &stmt.kind {
        hir::StmtKind::Assign {
            target, type_hint, ..
        } => {
            if !var_set.contains(target) {
                vars.push(GeneratorVar {
                    var_id: *target,
                    gen_local_idx: *next_idx,
                    ty: type_hint.clone().unwrap_or(Type::Any),
                    is_param: false,
                });
                var_set.insert(*target);
                *next_idx += 1;
            }
        }
        hir::StmtKind::For { target, body, .. } => {
            if !var_set.contains(target) {
                vars.push(GeneratorVar {
                    var_id: *target,
                    gen_local_idx: *next_idx,
                    ty: Type::Any, // Loop variable may hold any type (int, str, etc.)
                    is_param: false,
                });
                var_set.insert(*target);
                *next_idx += 1;
            }
            for s in body {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
        }
        hir::StmtKind::ForUnpack { targets, body, .. } => {
            for target in targets {
                if !var_set.contains(target) {
                    vars.push(GeneratorVar {
                        var_id: *target,
                        gen_local_idx: *next_idx,
                        ty: Type::Any,
                        is_param: false,
                    });
                    var_set.insert(*target);
                    *next_idx += 1;
                }
            }
            for s in body {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
        }
        hir::StmtKind::If {
            then_block,
            else_block,
            ..
        } => {
            for s in then_block {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
            for s in else_block {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
        }
        hir::StmtKind::While { body, .. } => {
            for s in body {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
        }
        hir::StmtKind::Try {
            body,
            handlers,
            else_block,
            finally_block,
        } => {
            for s in body {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
            for handler in handlers {
                for s in &handler.body {
                    collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
                }
            }
            for s in else_block {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
            for s in finally_block {
                collect_vars_from_stmt(*s, hir_module, vars, var_set, next_idx);
            }
        }
        _ => {}
    }
}
