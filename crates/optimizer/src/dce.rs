//! Dead-code elimination: use-count fixpoint over side-effect-free
//! instructions, plus blanking of unreachable blocks.
//!
//! An instruction is removable iff [`pyaot_mir::MirInst::has_side_effects`]
//! is false AND its `dst` has zero reads. Removal decrements the read counts
//! of the instruction's operands, which can expose the next link of a dead
//! chain — hence the fixpoint.
//!
//! Unreachable blocks (typically a `Branch` arm constfold rewrote into a
//! `Jump`) are **blanked** (`insts.clear()`, `term = Unreachable`), never
//! removed: `BlockId`s are dense indices held by every other terminator, and
//! a remap pass is exactly the churn blanking avoids. Codegen emits a lone
//! trap for them.

use pyaot_mir::{MirFunction, MirProgram, MirTerminator, Operand};

use crate::analysis::{read_counts, reachable_blocks};
use crate::OptimizationPass;

pub struct Dce;

impl OptimizationPass for Dce {
    fn name(&self) -> &'static str {
        "dce"
    }

    fn run(&self, program: &mut MirProgram) {
        for func in &mut program.funcs {
            run_func(func);
        }
    }
}

fn run_func(f: &mut MirFunction) {
    blank_unreachable_blocks(f);

    let mut reads = read_counts(f);
    // Fixpoint: each sweep removes every currently-dead instruction and
    // decrements its operands' counts; repeat while something was removed
    // (a removed reader may zero its producer's count).
    loop {
        let mut changed = false;
        for block in &mut f.blocks {
            let locals = &f.locals;
            // retain() in evaluation order; count decrements inside the
            // closure are visible to later instructions in the same sweep.
            block.insts.retain(|inst| {
                let dead = !inst.has_side_effects(locals)
                    && inst
                        .dst()
                        .is_some_and(|d| reads[d.index()] == 0);
                if dead {
                    inst.for_each_operand(|op| {
                        let Operand::Local(id) = op;
                        reads[id.index()] -= 1;
                    });
                    changed = true;
                }
                !dead
            });
        }
        if !changed {
            break;
        }
    }
}

/// Blank every unreachable block in place (stable `BlockId`s, no remap).
fn blank_unreachable_blocks(f: &mut MirFunction) {
    let reachable = reachable_blocks(f);
    for (i, block) in f.blocks.iter_mut().enumerate() {
        let already_blank =
            block.insts.is_empty() && matches!(block.term, MirTerminator::Unreachable);
        if !reachable[i] && !already_blank {
            block.insts.clear();
            block.term = MirTerminator::Unreachable;
        }
    }
}

#[cfg(test)]
mod tests {
    use pyaot_mir::{BinOp, CmpOp, Const, MirInst, MirTerminator};
    use pyaot_types::{RawKind, Repr};
    use pyaot_utils::{BlockId, LocalId};

    use super::run_func;
    use crate::testutil::{l, op, single_block, verify_ok};

    #[test]
    fn removes_dead_pure_chain() {
        // l0 = 1; l1 = 2; l2 = l0 + l1 (raw, pure) — nothing read; the whole
        // chain dies across fixpoint sweeps (l2 first, then its operands).
        let mut f = single_block(
            vec![Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I64)],
            vec![
                MirInst::Const { dst: l(0), val: Const::Int(1) },
                MirInst::Const { dst: l(1), val: Const::Int(2) },
                MirInst::BinOp { dst: l(2), op: BinOp::Add, l: op(0), r: op(1) },
            ],
            MirTerminator::Return(None),
        );
        run_func(&mut f);
        assert!(f.blocks[0].insts.is_empty());
        verify_ok(&f);
    }

    #[test]
    fn keeps_impure_with_unread_dst() {
        // Tagged + Tagged dispatches through the runtime (may raise) — must
        // survive even though l2 is never read.
        let mut f = single_block(
            vec![Repr::Tagged, Repr::Tagged, Repr::Tagged],
            vec![
                MirInst::Const { dst: l(0), val: Const::Int(1) },
                MirInst::Const { dst: l(1), val: Const::Int(2) },
                MirInst::BinOp { dst: l(2), op: BinOp::Add, l: op(0), r: op(1) },
            ],
            MirTerminator::Return(None),
        );
        run_func(&mut f);
        assert_eq!(f.blocks[0].insts.len(), 3);
        verify_ok(&f);
    }

    #[test]
    fn keeps_values_read_by_terminator() {
        // The Branch condition and the Return operand are reads.
        let mut f = single_block(
            vec![Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I8)],
            vec![
                MirInst::Const { dst: l(0), val: Const::Int(1) },
                MirInst::Const { dst: l(1), val: Const::Int(2) },
                MirInst::Compare { dst: l(2), op: CmpOp::Lt, l: op(0), r: op(1) },
            ],
            MirTerminator::Branch { cond: op(2), then: BlockId::new(0), else_: BlockId::new(0) },
        );
        run_func(&mut f);
        assert_eq!(f.blocks[0].insts.len(), 3);
    }

    #[test]
    fn dead_chain_through_intermediate_reader() {
        // l1 = l0 (coerce noop); l1 unread -> dies -> l0's count drops to 0
        // -> the Const dies on the next sweep. Proves the fixpoint matters.
        let mut f = single_block(
            vec![Repr::Raw(RawKind::I64), Repr::Raw(RawKind::I64)],
            vec![
                MirInst::Const { dst: l(0), val: Const::Int(7) },
                MirInst::Coerce(
                    pyaot_mir::CoerceInst::new(
                        l(1),
                        op(0),
                        Repr::Raw(RawKind::I64),
                        Repr::Raw(RawKind::I64),
                    )
                    .expect("identity coercion is legal"),
                ),
            ],
            MirTerminator::Return(None),
        );
        run_func(&mut f);
        assert!(f.blocks[0].insts.is_empty());
        verify_ok(&f);
    }

    #[test]
    fn blanks_unreachable_block() {
        // Block 1 is never targeted: it must be blanked (insts cleared, term
        // Unreachable), NOT removed — BlockIds stay stable.
        let mut f = crate::testutil::function(
            vec![Repr::Tagged],
            vec![
                (vec![], MirTerminator::Return(None)),
                (
                    vec![MirInst::Const { dst: LocalId::new(0), val: Const::Int(3) }],
                    MirTerminator::Jump(BlockId::new(0)),
                ),
            ],
        );
        run_func(&mut f);
        assert_eq!(f.blocks.len(), 2);
        assert!(f.blocks[1].insts.is_empty());
        assert!(matches!(f.blocks[1].term, MirTerminator::Unreachable));
        verify_ok(&f);
    }

    #[test]
    fn checked_coerce_survives() {
        // checked Coerce raises TypeError on a bad tag — an effect.
        let mut f = single_block(
            vec![Repr::Tagged, Repr::Raw(RawKind::F64)],
            vec![
                MirInst::Const { dst: l(0), val: Const::None },
                MirInst::Coerce(
                    pyaot_mir::CoerceInst::new_checked(
                        l(1),
                        op(0),
                        Repr::Tagged,
                        Repr::Raw(RawKind::F64),
                    )
                    .expect("checked unbox shape"),
                ),
            ],
            MirTerminator::Return(None),
        );
        run_func(&mut f);
        assert_eq!(f.blocks[0].insts.len(), 2);
    }
}
