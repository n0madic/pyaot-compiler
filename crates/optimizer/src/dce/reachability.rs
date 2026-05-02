//! Unreachable block elimination
//!
//! Removes basic blocks that are not reachable from the function's entry block
//! via CFG edges. This commonly occurs after inlining when conditional branches
//! are simplified or when exception handlers become dead.

use std::collections::VecDeque;

use indexmap::IndexSet;
use pyaot_mir::{terminator_successors, Function, InstructionKind};
use pyaot_utils::BlockId;

/// Remove blocks not reachable from the entry block.
/// Returns true if any blocks were removed.
pub fn eliminate_unreachable_blocks(func: &mut Function) -> bool {
    let reachable = compute_reachable_blocks(func);
    let before = func.blocks.len();
    func.blocks.retain(|id, _| reachable.contains(id));
    let removed = func.blocks.len() < before;
    if removed {
        func.invalidate_dom_tree();
        // Phi sources may still reference removed predecessors. Without this
        // cleanup the SSA invariant checker fires (PhiArityMismatch).
        prune_phi_sources(func, &reachable);
    }
    removed
}

/// Remove phi sources whose pred BlockId is no longer in `reachable`.
/// Called after `func.blocks` has been pruned to the reachable set.
fn prune_phi_sources(func: &mut Function, reachable: &IndexSet<BlockId>) {
    for block in func.blocks.values_mut() {
        for inst in &mut block.instructions {
            if let InstructionKind::Phi { sources, .. } = &mut inst.kind {
                sources.retain(|(pred, _)| reachable.contains(pred));
            }
        }
    }
}

/// BFS from entry_block to find all reachable blocks.
fn compute_reachable_blocks(func: &Function) -> IndexSet<BlockId> {
    let mut reachable = IndexSet::new();
    let mut worklist = VecDeque::new();

    reachable.insert(func.entry_block);
    worklist.push_back(func.entry_block);

    while let Some(block_id) = worklist.pop_front() {
        if let Some(block) = func.blocks.get(&block_id) {
            for succ in terminator_successors(&block.terminator) {
                if reachable.insert(succ) {
                    worklist.push_back(succ);
                }
            }
        }
    }

    reachable
}
