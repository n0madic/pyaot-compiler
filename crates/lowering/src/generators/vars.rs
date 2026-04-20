//! Variable collection for generator functions
//!
//! This module provides functions to collect all variables that need to persist
//! across yields in a generator function.
//!
//! §1.17b-d — migrated to walk `func.blocks` CFG. Previously recursed through
//! tree-shape `StmtKind::{If, While, ForBind, Try}` variants; now iterates
//! blocks in IndexMap order (bridge's pre-order DFS) and extracts bound
//! variables from straight-line `Bind` and `IterAdvance` stmts.

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
    // collide with the iter slot. `detect_for_loop_generator` is still
    // tree-shape (operates on `func.body`); migrating it is a separate
    // piece since its pattern-matching on `StmtKind::ForBind` is specific
    // to single-for-loop generators.
    let mut next_idx = if detect_for_loop_generator(func, hir_module).is_some() {
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

    // Then collect all assigned variables via CFG block iteration.
    // Bridge allocates blocks in pre-order DFS of the source tree, so
    // the iteration order matches what the tree-walker saw.
    for block in func.blocks.values() {
        for &stmt_id in &block.stmts {
            collect_vars_from_flat_stmt(
                stmt_id,
                hir_module,
                &mut vars,
                &mut var_set,
                &mut next_idx,
            );
        }
    }

    vars
}

/// Extract bound-var info from a single straight-line statement.
/// Post-bridge, `HirBlock.stmts` only contains non-control-flow variants:
/// `Bind`, `IterAdvance`, `Expr`, `Return`, `Assert`, `Pass`, `Raise`,
/// `Break`, `Continue`, `IndexDelete`, `IterSetup`. Of these, only
/// `Bind` and `IterAdvance` introduce new variable bindings.
fn collect_vars_from_flat_stmt(
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
        // §1.17b-c — `IterAdvance { iter, target }` replaces tree-form
        // `ForBind` inside CFG body blocks. Binds the next iter value
        // to the for-loop target var. Element type is Any (matches the
        // former ForBind arm which also used Any — per-element typing
        // happens elsewhere via prescan).
        hir::StmtKind::IterAdvance { target, .. } => {
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
        }
        // All other stmts don't introduce new variable bindings.
        _ => {}
    }
}
