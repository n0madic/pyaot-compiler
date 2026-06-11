//! Cold-block classification (Phase 9C.5) — a pure CFG analysis, not an
//! optimizer pass (no MIR rewrite; the [`roots_needed`](crate::liveness)
//! precedent). Codegen feeds the result to Cranelift's `set_cold_block`, so
//! exception handlers and raise paths move out of the hot instruction
//! stream.

use crate::{MirFunction, MirInst, MirTerminator};
use pyaot_utils::BlockId;

/// `true` per block iff it is COLD: not reachable from the entry through
/// normal control flow (a `TryEnter`'s `handler` edge does not count — the
/// handler only runs on a raise), or it diverges into a raise
/// (`Raise`/`AssertFail` + `Unreachable` — the verifier-enforced shape).
pub fn cold_blocks(f: &MirFunction) -> Vec<bool> {
    // Warm = reachable from entry, TryEnter contributing only `normal`.
    let mut warm = vec![false; f.blocks.len()];
    let mut work = vec![f.entry];
    while let Some(b) = work.pop() {
        if warm[b.index()] {
            continue;
        }
        warm[b.index()] = true;
        let mut push = |t: BlockId| work.push(t);
        match &f.blocks[b.index()].term {
            MirTerminator::Jump(t) => push(*t),
            MirTerminator::Branch { then, else_, .. } => {
                push(*then);
                push(*else_);
            }
            MirTerminator::TryEnter { normal, .. } => push(*normal),
            MirTerminator::Return(_) | MirTerminator::Unreachable => {}
        }
    }

    let mut cold: Vec<bool> = f
        .blocks
        .iter()
        .enumerate()
        .map(|(i, block)| {
            if !warm[i] {
                return true;
            }
            // A warm block that diverges into a raise is still cold.
            matches!(block.term, MirTerminator::Unreachable)
                && matches!(
                    block.insts.last(),
                    Some(MirInst::Raise(_)) | Some(MirInst::AssertFail)
                )
        })
        .collect();
    // Cranelift forbids a cold entry block (an always-raising function's
    // entry would otherwise qualify) — and there is nowhere to move it anyway.
    cold[f.entry.index()] = false;
    cold
}

#[cfg(test)]
mod tests {
    use super::cold_blocks;
    use crate::{LocalDecl, MirBlock, MirFunction, MirInst, MirRaise, MirTerminator};
    use pyaot_types::Repr;
    use pyaot_utils::{BlockId, StringInterner};

    fn function(blocks: Vec<(Vec<MirInst>, MirTerminator)>) -> MirFunction {
        MirFunction {
            name: StringInterner::new().intern("f"),
            params: Vec::new(),
            ret: Repr::Tagged,
            locals: Vec::<LocalDecl>::new(),
            blocks: blocks
                .into_iter()
                .map(|(insts, term)| MirBlock { insts, term })
                .collect(),
            entry: BlockId::new(0),
        }
    }

    #[test]
    fn handler_is_cold_normal_is_warm() {
        let f = function(vec![
            (vec![], MirTerminator::TryEnter { normal: BlockId::new(1), handler: BlockId::new(2) }),
            (vec![], MirTerminator::Return(None)), // normal path
            (vec![], MirTerminator::Return(None)), // handler
        ]);
        assert_eq!(cold_blocks(&f), vec![false, false, true]);
    }

    #[test]
    fn raise_block_is_cold_even_when_warm_reachable() {
        let f = function(vec![
            (vec![], MirTerminator::Jump(BlockId::new(1))),
            (vec![MirInst::Raise(MirRaise::Reraise)], MirTerminator::Unreachable),
        ]);
        assert_eq!(cold_blocks(&f), vec![false, true]);
    }

    #[test]
    fn always_raising_entry_stays_warm() {
        // Cranelift rejects a cold entry block.
        let f = function(vec![(
            vec![MirInst::Raise(MirRaise::Reraise)],
            MirTerminator::Unreachable,
        )]);
        assert_eq!(cold_blocks(&f), vec![false]);
    }

    #[test]
    fn unreachable_block_is_cold() {
        let f = function(vec![
            (vec![], MirTerminator::Return(None)),
            (vec![], MirTerminator::Return(None)), // never targeted
        ]);
        assert_eq!(cold_blocks(&f), vec![false, true]);
    }
}
