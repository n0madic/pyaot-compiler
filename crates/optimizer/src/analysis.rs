//! Shared per-function analyses for the Phase 9 passes: read counts,
//! block reachability, and the block-local constant lattice.

use std::collections::HashMap;

use pyaot_mir::{Const, MirFunction, MirTerminator, Operand};
use pyaot_utils::{BlockId, LocalId};

/// How many times each local is READ (instruction operands + terminator
/// operands — `Branch` conditions and `Return` values). `dst` writes do not
/// count. Indexed by `LocalId::index()`.
pub fn read_counts(f: &MirFunction) -> Vec<u32> {
    let mut counts = vec![0u32; f.locals.len()];
    let mut bump = |op: &Operand| {
        let Operand::Local(id) = op;
        counts[id.index()] += 1;
    };
    for block in &f.blocks {
        for inst in &block.insts {
            inst.for_each_operand(&mut bump);
        }
        match &block.term {
            MirTerminator::Return(Some(op)) => bump(op),
            MirTerminator::Branch { cond, .. } => bump(cond),
            MirTerminator::Return(None) | MirTerminator::Jump(_) | MirTerminator::Unreachable => {}
        }
    }
    counts
}

/// Which blocks are reachable from `entry` — a worklist over `Jump` /
/// `Branch` edges plus each reachable block's handler edge (a protected
/// block keeps its handler alive). Indexed by `BlockId::index()`.
pub fn reachable_blocks(f: &MirFunction) -> Vec<bool> {
    let mut reachable = vec![false; f.blocks.len()];
    let mut work = vec![f.entry];
    while let Some(b) = work.pop() {
        if reachable[b.index()] {
            continue;
        }
        reachable[b.index()] = true;
        let block = &f.blocks[b.index()];
        let mut push = |t: BlockId| work.push(t);
        if let Some(h) = block.handler {
            push(h);
        }
        match &block.term {
            MirTerminator::Jump(t) => push(*t),
            MirTerminator::Branch { then, else_, .. } => {
                push(*then);
                push(*else_);
            }
            MirTerminator::Return(_) | MirTerminator::Unreachable => {}
        }
    }
    reachable
}

/// A block-local forward constant environment: `LocalId → Const` while
/// scanning a block top-to-bottom. Any redefinition of a local KILLS its
/// entry (locals are mutable slots, not SSA). State never crosses block
/// boundaries — joins are someone else's problem, deliberately out of scope.
#[derive(Default)]
pub struct ConstLattice {
    consts: HashMap<LocalId, Const>,
}

impl ConstLattice {
    pub fn new() -> Self {
        Self::default()
    }

    /// The constant currently known for `local`, if any.
    pub fn get(&self, local: LocalId) -> Option<&Const> {
        self.consts.get(&local)
    }

    /// The constant known for an operand's local, if any.
    pub fn get_operand(&self, op: &Operand) -> Option<&Const> {
        let Operand::Local(id) = op;
        self.get(*id)
    }

    /// Record that `local` now holds `val` (a `Const` instruction or a fold).
    pub fn set(&mut self, local: LocalId, val: Const) {
        self.consts.insert(local, val);
    }

    /// `local` was overwritten with something non-constant — kill it.
    pub fn kill(&mut self, local: LocalId) {
        self.consts.remove(&local);
    }
}
