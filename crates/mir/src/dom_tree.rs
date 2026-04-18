//! Dominator tree + dominance frontier for MIR functions.
//!
//! Implements the classical Cooper–Harvey–Kennedy algorithm
//! ("A Simple, Fast Dominance Algorithm", SPLC 2001): a fix-point over
//! reverse-post-order using an `intersect(b1, b2)` walk along the idom
//! tree. See `ARCHITECTURE_REFACTOR.md` §1.2 (session S1.4).
//!
//! `DomTree::compute` is `O(N · D)` where `N` is the block count and `D` is
//! the dominator-tree depth — linear for well-structured control flow and
//! still competitive on irreducible graphs. A function's dominator tree is
//! cached via `OnceCell` on `mir::Function`; passes that mutate the CFG
//! call `invalidate_dom_tree()` on the function to drop the cache.
//!
//! This module owns the canonical `terminator_successors` helper; every
//! other place that needs CFG successors consumes it from here (previously
//! duplicated inside `ssa_check`).
//!
//! Unreachable blocks (no path from `entry_block`) receive no idom and
//! appear neither in `reverse_post_order()` nor in dominance queries as
//! the dominated side — `dominates(a, unreachable)` returns `false`.

use std::collections::HashMap;

use pyaot_utils::BlockId;
use smallvec::{smallvec, SmallVec};

use crate::{Function, Terminator};

/// Successors exposed by a terminator, in the fixed order used throughout
/// the MIR crate (Goto → one successor; Branch → [then, else]; TrySetjmp →
/// [try_body, handler_entry]; raise / return / unreachable → none).
pub fn terminator_successors(t: &Terminator) -> SmallVec<[BlockId; 2]> {
    match t {
        Terminator::Goto(b) => smallvec![*b],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => smallvec![*then_block, *else_block],
        Terminator::TrySetjmp {
            try_body,
            handler_entry,
            ..
        } => smallvec![*try_body, *handler_entry],
        Terminator::Return(_)
        | Terminator::Unreachable
        | Terminator::Raise { .. }
        | Terminator::RaiseCustom { .. }
        | Terminator::Reraise
        | Terminator::RaiseInstance { .. } => SmallVec::new(),
    }
}

/// Precomputed dominator tree + dominance frontier.
#[derive(Debug, Clone)]
pub struct DomTree {
    /// Blocks reachable from `entry`, in reverse post-order (entry first,
    /// deeper blocks later). Unreachable blocks are omitted.
    rpo: Vec<BlockId>,
    /// `rpo_index[b]` = position of `b` in `rpo`. Smaller index == closer
    /// to entry. Used by `intersect()` to pick the "deeper" finger.
    rpo_index: HashMap<BlockId, usize>,
    /// Immediate dominator. `idom[entry] == entry` (sentinel). Blocks not
    /// present here are unreachable from entry.
    idom: HashMap<BlockId, BlockId>,
    /// Dominance frontier — the set of blocks at which dominance of each
    /// block "expires" (i.e. the merge points where other paths join).
    df: HashMap<BlockId, SmallVec<[BlockId; 4]>>,
    entry: BlockId,
}

impl DomTree {
    /// Build the dominator tree for `func` starting at `func.entry_block`.
    pub fn compute(func: &Function) -> Self {
        let entry = func.entry_block;
        let rpo = reverse_post_order(func, entry);
        let rpo_index: HashMap<BlockId, usize> =
            rpo.iter().enumerate().map(|(i, b)| (*b, i)).collect();

        let preds = compute_predecessors(func, &rpo_index);
        let idom = compute_idoms(entry, &rpo, &rpo_index, &preds);
        let df = compute_dominance_frontier(&rpo, &preds, &idom, entry);

        Self {
            rpo,
            rpo_index,
            idom,
            df,
            entry,
        }
    }

    /// Blocks visited in reverse post-order starting at the entry block.
    /// Unreachable blocks are omitted.
    pub fn reverse_post_order(&self) -> &[BlockId] {
        &self.rpo
    }

    /// Immediate dominator of `block`, or `None` for the entry block and
    /// for any block unreachable from the entry.
    pub fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        if block == self.entry {
            return None;
        }
        let parent = self.idom.get(&block).copied()?;
        if parent == block {
            // Sentinel self-loop used only for the entry; treat as "no idom".
            None
        } else {
            Some(parent)
        }
    }

    /// `true` iff `a` dominates `b`, i.e. every path from the entry to `b`
    /// passes through `a`. A block dominates itself. Unreachable `b`
    /// returns `false`.
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if !self.idom.contains_key(&b) && b != self.entry {
            return false;
        }
        let mut cur = b;
        loop {
            if cur == a {
                return true;
            }
            match self.idom.get(&cur).copied() {
                // Walking past the entry sentinel: `a` is not an ancestor.
                Some(parent) if parent == cur => return false,
                Some(parent) => cur = parent,
                None => return false,
            }
        }
    }

    /// Dominance frontier of `block` — the blocks where `block`'s
    /// dominance ends. Iterator yields each block at most once.
    pub fn dominance_frontier(&self, block: BlockId) -> impl Iterator<Item = BlockId> + '_ {
        self.df
            .get(&block)
            .into_iter()
            .flat_map(|v| v.iter().copied())
    }

    /// Position of `block` in the reverse post-order list, or `None` if
    /// the block is unreachable from entry. Used by RPO-driven passes
    /// (e.g. Cytron's φ-insertion in S1.6) that need a stable block order.
    pub fn rpo_index(&self, block: BlockId) -> Option<usize> {
        self.rpo_index.get(&block).copied()
    }
}

fn reverse_post_order(func: &Function, entry: BlockId) -> Vec<BlockId> {
    // Iterative post-order DFS — returns blocks in post-order, then reverses.
    // Using an explicit work stack keeps recursion depth off the OS stack
    // for deeply-nested functions.
    if !func.blocks.contains_key(&entry) {
        return Vec::new();
    }
    let mut visited = HashMap::new();
    let mut post_order: Vec<BlockId> = Vec::new();
    // Stack entries: (block, iter-index-into-successors).
    let mut stack: Vec<(BlockId, usize)> = vec![(entry, 0)];
    visited.insert(entry, ());

    while let Some(&(bid, idx)) = stack.last() {
        let block = match func.blocks.get(&bid) {
            Some(b) => b,
            // Dangling successor pointing at a missing block — treat as leaf.
            None => {
                stack.pop();
                post_order.push(bid);
                continue;
            }
        };
        let succs = terminator_successors(&block.terminator);
        if idx < succs.len() {
            // Advance this frame before descending, so when we return we
            // continue with the next successor.
            let next = succs[idx];
            if let Some(last) = stack.last_mut() {
                last.1 = idx + 1;
            }
            if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(next) {
                e.insert(());
                stack.push((next, 0));
            }
        } else {
            stack.pop();
            post_order.push(bid);
        }
    }

    post_order.reverse();
    post_order
}

fn compute_predecessors(
    func: &Function,
    reachable: &HashMap<BlockId, usize>,
) -> HashMap<BlockId, Vec<BlockId>> {
    let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for (&bid, block) in &func.blocks {
        if !reachable.contains_key(&bid) {
            continue;
        }
        for succ in terminator_successors(&block.terminator) {
            preds.entry(succ).or_default().push(bid);
        }
    }
    preds
}

fn compute_idoms(
    entry: BlockId,
    rpo: &[BlockId],
    rpo_index: &HashMap<BlockId, usize>,
    preds: &HashMap<BlockId, Vec<BlockId>>,
) -> HashMap<BlockId, BlockId> {
    // Cooper–Harvey–Kennedy: idom[entry] = entry (sentinel); all others start
    // unset. Iterate RPO (skipping entry) to fix-point, intersecting the
    // already-processed predecessors.
    let mut idom: HashMap<BlockId, BlockId> = HashMap::new();
    idom.insert(entry, entry);

    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().skip(1) {
            let empty: Vec<BlockId> = Vec::new();
            let block_preds = preds.get(&b).unwrap_or(&empty);

            let mut new_idom: Option<BlockId> = None;
            for &p in block_preds {
                if !idom.contains_key(&p) {
                    // Predecessor not yet processed — skip for this pass.
                    continue;
                }
                new_idom = Some(match new_idom {
                    None => p,
                    Some(existing) => intersect(p, existing, rpo_index, &idom),
                });
            }

            if let Some(new_idom) = new_idom {
                if idom.get(&b).copied() != Some(new_idom) {
                    idom.insert(b, new_idom);
                    changed = true;
                }
            }
        }
    }

    idom
}

fn intersect(
    mut b1: BlockId,
    mut b2: BlockId,
    rpo_index: &HashMap<BlockId, usize>,
    idom: &HashMap<BlockId, BlockId>,
) -> BlockId {
    while b1 != b2 {
        // Walk the "deeper" pointer (higher RPO index) up the idom tree
        // until both fingers meet. This finds the nearest common
        // ancestor in the partially-constructed idom tree.
        while rpo_index.get(&b1).copied().unwrap_or(usize::MAX)
            > rpo_index.get(&b2).copied().unwrap_or(usize::MAX)
        {
            b1 = match idom.get(&b1).copied() {
                Some(p) if p != b1 => p,
                _ => return b2,
            };
        }
        while rpo_index.get(&b2).copied().unwrap_or(usize::MAX)
            > rpo_index.get(&b1).copied().unwrap_or(usize::MAX)
        {
            b2 = match idom.get(&b2).copied() {
                Some(p) if p != b2 => p,
                _ => return b1,
            };
        }
    }
    b1
}

fn compute_dominance_frontier(
    rpo: &[BlockId],
    preds: &HashMap<BlockId, Vec<BlockId>>,
    idom: &HashMap<BlockId, BlockId>,
    entry: BlockId,
) -> HashMap<BlockId, SmallVec<[BlockId; 4]>> {
    // Cooper–Harvey–Kennedy Figure 5: for each join point `b` (|preds| >= 2),
    // each predecessor walks up the idom tree, adding `b` to its DF, until
    // it reaches idom[b].
    let mut df: HashMap<BlockId, SmallVec<[BlockId; 4]>> = HashMap::new();
    for &b in rpo {
        let empty: Vec<BlockId> = Vec::new();
        let block_preds = preds.get(&b).unwrap_or(&empty);
        if block_preds.len() < 2 {
            continue;
        }
        let Some(&b_idom) = idom.get(&b) else {
            continue;
        };
        for &p in block_preds {
            let mut runner = p;
            while runner != b_idom {
                let entry_set = df.entry(runner).or_default();
                if !entry_set.contains(&b) {
                    entry_set.push(b);
                }
                runner = match idom.get(&runner).copied() {
                    // Entry sentinel (`idom[entry] == entry`): stop walking
                    // to avoid infinite loop at the root.
                    Some(parent) if parent == runner => {
                        // Only keep walking if we haven't reached b_idom yet,
                        // and we're at the sentinel — break defensively.
                        if runner == entry {
                            break;
                        }
                        parent
                    }
                    Some(parent) => parent,
                    None => break,
                };
                if runner == b_idom {
                    break;
                }
            }
        }
    }
    df
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BasicBlock, Function, Operand, Terminator};
    use pyaot_types::Type;
    use pyaot_utils::FuncId;

    fn fresh_func() -> Function {
        // Function::new creates block 0 as the entry, terminated Unreachable.
        let mut func = Function::new(
            FuncId::from(0u32),
            "test".to_string(),
            Vec::new(),
            Type::None,
            None,
        );
        // Reset entry block's terminator to Return, since tests below
        // build custom CFGs on top.
        func.block_mut(BlockId::from(0u32)).terminator = Terminator::Return(None);
        func
    }

    fn add_block(func: &mut Function, id: u32, terminator: Terminator) -> BlockId {
        let bid = BlockId::from(id);
        func.blocks.insert(
            bid,
            BasicBlock {
                id: bid,
                instructions: Vec::new(),
                terminator,
            },
        );
        bid
    }

    fn set_term(func: &mut Function, bid: BlockId, t: Terminator) {
        func.block_mut(bid).terminator = t;
    }

    #[test]
    fn linear_chain_entry_dominates_everything() {
        //   entry(0) -> b1 -> b2 -> ret
        let mut func = fresh_func();
        let b1 = add_block(&mut func, 1, Terminator::Unreachable);
        let b2 = add_block(&mut func, 2, Terminator::Return(None));
        set_term(&mut func, BlockId::from(0u32), Terminator::Goto(b1));
        set_term(&mut func, b1, Terminator::Goto(b2));

        let dom = DomTree::compute(&func);
        let entry = BlockId::from(0u32);
        assert!(dom.dominates(entry, b1));
        assert!(dom.dominates(entry, b2));
        assert!(dom.dominates(b1, b2));
        assert!(!dom.dominates(b2, b1));
        assert_eq!(dom.immediate_dominator(b1), Some(entry));
        assert_eq!(dom.immediate_dominator(b2), Some(b1));
        assert_eq!(dom.immediate_dominator(entry), None);
    }

    #[test]
    fn diamond_merge_is_in_dominance_frontier_of_branches() {
        //          entry
        //         /     \
        //       then    else
        //         \     /
        //          merge
        let mut func = fresh_func();
        let then_bb = add_block(&mut func, 1, Terminator::Unreachable);
        let else_bb = add_block(&mut func, 2, Terminator::Unreachable);
        let merge = add_block(&mut func, 3, Terminator::Return(None));
        let entry = BlockId::from(0u32);
        set_term(
            &mut func,
            entry,
            Terminator::Branch {
                cond: Operand::Constant(crate::Constant::Bool(true)),
                then_block: then_bb,
                else_block: else_bb,
            },
        );
        set_term(&mut func, then_bb, Terminator::Goto(merge));
        set_term(&mut func, else_bb, Terminator::Goto(merge));

        let dom = DomTree::compute(&func);
        // entry dominates both arms and the merge.
        assert!(dom.dominates(entry, then_bb));
        assert!(dom.dominates(entry, else_bb));
        assert!(dom.dominates(entry, merge));
        // Neither arm dominates the merge (both reach it).
        assert!(!dom.dominates(then_bb, merge));
        assert!(!dom.dominates(else_bb, merge));
        // idom of the merge is the entry (nearest common dominator).
        assert_eq!(dom.immediate_dominator(merge), Some(entry));

        // Dominance frontier: each arm's DF = {merge}.
        let then_df: Vec<BlockId> = dom.dominance_frontier(then_bb).collect();
        let else_df: Vec<BlockId> = dom.dominance_frontier(else_bb).collect();
        assert_eq!(then_df, vec![merge]);
        assert_eq!(else_df, vec![merge]);
    }

    #[test]
    fn while_loop_header_dominates_body_and_exit_is_df_of_body() {
        //   entry -> header -+
        //            ^  \    |
        //            |   \   v
        //            body    exit
        let mut func = fresh_func();
        let header = add_block(&mut func, 1, Terminator::Unreachable);
        let body = add_block(&mut func, 2, Terminator::Unreachable);
        let exit = add_block(&mut func, 3, Terminator::Return(None));
        let entry = BlockId::from(0u32);
        set_term(&mut func, entry, Terminator::Goto(header));
        set_term(
            &mut func,
            header,
            Terminator::Branch {
                cond: Operand::Constant(crate::Constant::Bool(true)),
                then_block: body,
                else_block: exit,
            },
        );
        set_term(&mut func, body, Terminator::Goto(header));

        let dom = DomTree::compute(&func);
        assert!(dom.dominates(header, body));
        assert!(dom.dominates(header, exit));
        assert_eq!(dom.immediate_dominator(body), Some(header));
        assert_eq!(dom.immediate_dominator(exit), Some(header));
        // Body jumps back to header → header is in DF(body). Body is also its own DF only indirectly.
        let body_df: Vec<BlockId> = dom.dominance_frontier(body).collect();
        assert_eq!(body_df, vec![header]);
    }

    #[test]
    fn unreachable_block_is_not_in_rpo_and_does_not_dominate() {
        // entry -> b1 -> ret ;  b2 is unreachable
        let mut func = fresh_func();
        let b1 = add_block(&mut func, 1, Terminator::Return(None));
        let b2 = add_block(&mut func, 2, Terminator::Return(None));
        let entry = BlockId::from(0u32);
        set_term(&mut func, entry, Terminator::Goto(b1));

        let dom = DomTree::compute(&func);
        assert!(dom.reverse_post_order().contains(&entry));
        assert!(dom.reverse_post_order().contains(&b1));
        assert!(!dom.reverse_post_order().contains(&b2));
        assert_eq!(dom.immediate_dominator(b2), None);
        // entry does not dominate the unreachable block.
        assert!(!dom.dominates(entry, b2));
    }

    #[test]
    fn block_dominates_itself() {
        let mut func = fresh_func();
        let entry = BlockId::from(0u32);
        let dom = DomTree::compute(&func);
        let _ = &mut func; // silence unused mut
        assert!(dom.dominates(entry, entry));
    }

    #[test]
    fn terminator_successors_basics() {
        use crate::{Constant, Operand};
        let goto = Terminator::Goto(BlockId::from(7u32));
        let br = Terminator::Branch {
            cond: Operand::Constant(Constant::Bool(true)),
            then_block: BlockId::from(8u32),
            else_block: BlockId::from(9u32),
        };
        let ret = Terminator::Return(None);
        assert_eq!(&*terminator_successors(&goto), &[BlockId::from(7u32)]);
        assert_eq!(
            &*terminator_successors(&br),
            &[BlockId::from(8u32), BlockId::from(9u32)]
        );
        assert!(terminator_successors(&ret).is_empty());
    }
}
