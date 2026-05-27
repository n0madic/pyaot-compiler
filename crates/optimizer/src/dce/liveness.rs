//! Dead instruction and dead local elimination
//!
//! Removes pure instructions whose results are never used, and cleans up
//! local variable entries that are no longer referenced by any instruction.

use indexmap::IndexSet;
use pyaot_mir::Function;
use pyaot_utils::LocalId;

use super::instruction_is_pure;

/// Eliminate dead instructions: pure instructions whose dest is never used.
/// Returns true if any instructions were removed.
pub fn eliminate_dead_instructions(func: &mut Function) -> bool {
    let used_locals = compute_used_locals(func);

    let mut changed = false;
    for block in func.blocks.values_mut() {
        let before = block.instructions.len();
        block.instructions.retain(|instr| {
            if let Some(dest) = instr.kind.def() {
                if instruction_is_pure(&instr.kind) && !used_locals.contains(&dest) {
                    return false;
                }
            }
            true
        });
        if block.instructions.len() < before {
            changed = true;
        }
    }

    changed
}

/// Remove locals from func.locals that are not referenced by any instruction or terminator.
/// Returns true if any locals were removed.
pub fn eliminate_dead_locals(func: &mut Function) -> bool {
    let mut referenced = IndexSet::new();

    for block in func.blocks.values() {
        for instr in &block.instructions {
            if let Some(dest) = instr.kind.def() {
                referenced.insert(dest);
            }
            instr.kind.for_each_use(|id| {
                referenced.insert(id);
            });
        }
        block.terminator.for_each_use(|id| {
            referenced.insert(id);
        });
    }

    // Keep parameters even if unused (they are part of the function signature)
    for param in &func.params {
        referenced.insert(param.id);
    }

    let before = func.locals.len();
    func.locals.retain(|id, _| referenced.contains(id));
    func.locals.len() < before
}

/// Compute the set of all locals that are *read* by any instruction or terminator.
fn compute_used_locals(func: &Function) -> IndexSet<LocalId> {
    let mut used = IndexSet::new();

    for block in func.blocks.values() {
        for instr in &block.instructions {
            instr.kind.for_each_use(|id| {
                used.insert(id);
            });
        }
        block.terminator.for_each_use(|id| {
            used.insert(id);
        });
    }

    used
}
