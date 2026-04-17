//! Variable collection for generator functions
//!
//! This module provides functions to collect all variables that need to persist
//! across yields in a generator function.

use std::collections::HashSet;

use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use super::for_loop::detect_for_loop_generator;
use super::GeneratorVar;

/// Collect all variables used in a generator function
pub(super) fn collect_generator_vars(
    func: &hir::Function,
    hir_module: &hir::Module,
) -> Vec<GeneratorVar> {
    let mut vars: Vec<GeneratorVar> = Vec::new();
    let mut var_set: HashSet<VarId> = HashSet::new();
    // Reserve slot 0 for the for-loop iterator (see `build_creator_body` /
    // resume builders in `desugaring.rs` — they hardcode slot 0 for the
    // iter). Without this reservation, the first generator param would
    // collide with the iter slot.
    let mut next_idx = if detect_for_loop_generator(&func.body, hir_module).is_some() {
        1u32
    } else {
        0u32
    };

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
        hir::StmtKind::Bind {
            target, type_hint, ..
        } => {
            target.for_each_var(&mut |var_id| {
                if !var_set.contains(&var_id) {
                    vars.push(GeneratorVar {
                        var_id,
                        gen_local_idx: *next_idx,
                        ty: type_hint.clone().unwrap_or(Type::Any),
                        is_param: false,
                    });
                    var_set.insert(var_id);
                    *next_idx += 1;
                }
            });
        }
        hir::StmtKind::ForBind { target, body, .. } => {
            target.for_each_var(&mut |var_id| {
                if !var_set.contains(&var_id) {
                    vars.push(GeneratorVar {
                        var_id,
                        gen_local_idx: *next_idx,
                        ty: Type::Any,
                        is_param: false,
                    });
                    var_set.insert(var_id);
                    *next_idx += 1;
                }
            });
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
