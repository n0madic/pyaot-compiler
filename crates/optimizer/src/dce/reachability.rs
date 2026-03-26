//! Unreachable block elimination
//!
//! Removes basic blocks that are not reachable from the function's entry block
//! via CFG edges. This commonly occurs after inlining when conditional branches
//! are simplified or when exception handlers become dead.

use std::collections::VecDeque;

use indexmap::IndexSet;
use pyaot_mir::Function;
use pyaot_utils::BlockId;

use super::terminator_successors;

/// Remove blocks not reachable from the entry block.
/// Returns true if any blocks were removed.
pub fn eliminate_unreachable_blocks(func: &mut Function) -> bool {
    let reachable = compute_reachable_blocks(func);
    let before = func.blocks.len();
    func.blocks.retain(|id, _| reachable.contains(id));
    func.blocks.len() < before
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
