//! Peephole cleanups:
//!
//! * **Coerce round-trip**: `b = Coerce(a)[r1→r2]` followed (same block, no
//!   intervening redefinition of `a` or `b`) by `c = Coerce(b)[r2→r1]`
//!   rewrites to `c = Coerce(a)[r1→r1]` (a Noop move). This kills the
//!   box→unbox churn around call boundaries; DCE then removes the first
//!   coerce if nothing else reads it. `checked` coerces never participate —
//!   eliding one would skip its TypeError.
//! * **Jump threading**: any terminator target pointing at an empty block
//!   whose terminator is `Jump(C)` retargets to `C` (loop-safe via a visited
//!   set). The empty forwarder is left for DCE to blank.
//! * `Branch { then == else_ }` → `Jump(then)`.

use std::collections::HashMap;

use pyaot_mir::{CoerceInst, MirFunction, MirInst, MirProgram, MirTerminator, Operand};
use pyaot_types::Repr;
use pyaot_utils::{BlockId, LocalId};

use crate::OptimizationPass;

pub struct Peephole;

impl OptimizationPass for Peephole {
    fn name(&self) -> &'static str {
        "peephole"
    }

    fn run(&self, program: &mut MirProgram) {
        for func in &mut program.funcs {
            coerce_round_trips(func);
            thread_jumps(func);
            collapse_same_target_branches(func);
        }
    }
}

/// `(src local, from, to)` of a live unchecked Coerce, keyed by its dst.
type CoerceMap = HashMap<LocalId, (LocalId, Repr, Repr)>;

fn coerce_round_trips(f: &mut MirFunction) {
    for block in &mut f.blocks {
        let mut seen: CoerceMap = HashMap::new();
        // Reverse index: source local → the tracked coerce dsts reading it, so
        // a write invalidates its dependents without scanning all of `seen`.
        let mut by_src: HashMap<LocalId, Vec<LocalId>> = HashMap::new();
        for inst in &mut block.insts {
            // Try the rewrite BEFORE recording this instruction's own write.
            // A CHECKED narrowing folds too: its source provably came out of
            // `to` itself (`X → Tagged → checked X`), so the guard cannot fire.
            if let MirInst::Coerce(c) = inst {
                {
                    let Operand::Local(src) = *c.src();
                    if let Some((orig, from1, to1)) = seen.get(&src) {
                        if c.from() == to1 && c.to() == from1 {
                            let noop = CoerceInst::new(
                                c.dst(),
                                Operand::Local(*orig),
                                from1.clone(),
                                from1.clone(),
                            )
                            .expect("identity coercion is always legal");
                            *inst = MirInst::Coerce(noop);
                        }
                    }
                }
            }

            // Any write kills entries it invalidates (the dst itself, and any
            // tracked coerce whose source it overwrites). A stale `by_src`
            // entry is harmless: removal re-checks the recorded source.
            if let Some(dst) = inst.dst() {
                seen.remove(&dst);
                for dep in by_src.remove(&dst).unwrap_or_default() {
                    if seen.get(&dep).is_some_and(|(src, _, _)| *src == dst) {
                        seen.remove(&dep);
                    }
                }
            }

            // Record a fresh unchecked coerce AFTER the kill (its own dst).
            if let MirInst::Coerce(c) = inst {
                if !c.checked() && c.from() != c.to() {
                    let Operand::Local(src) = *c.src();
                    seen.insert(c.dst(), (src, c.from().clone(), c.to().clone()));
                    by_src.entry(src).or_default().push(c.dst());
                }
            }
        }
    }
}

/// Resolve every block through chains of empty `Jump`-only blocks, in one
/// memoized pass (each block is walked once overall). Loop-safe: a block on a
/// cycle of empty forwarders resolves to itself, and a chain leading into such
/// a cycle resolves to the block where it enters the cycle.
fn resolve_targets(f: &MirFunction) -> Vec<BlockId> {
    const UNSEEN: u8 = 0;
    const ON_PATH: u8 = 1;
    const DONE: u8 = 2;
    let n = f.blocks.len();
    let mut state = vec![UNSEEN; n];
    let mut resolved: Vec<BlockId> = (0..n).map(|i| BlockId::new(i as u32)).collect();
    let mut path: Vec<usize> = Vec::new();
    for start in 0..n {
        if state[start] == DONE {
            continue;
        }
        let mut cur = start;
        // Walk forward until the chain ends, rejoins a resolved block, or
        // loops back onto the current path.
        let target = loop {
            match state[cur] {
                DONE => break resolved[cur],
                ON_PATH => {
                    // A cycle: its members resolve to themselves; the path
                    // prefix leading into it resolves to `cur`.
                    let at = path.iter().position(|&b| b == cur).expect("on path");
                    for &b in &path[at..] {
                        state[b] = DONE;
                    }
                    path.truncate(at);
                    break BlockId::new(cur as u32);
                }
                _ => {}
            }
            let block = &f.blocks[cur];
            match block.term {
                MirTerminator::Jump(next) if block.insts.is_empty() => {
                    state[cur] = ON_PATH;
                    path.push(cur);
                    cur = next.index();
                }
                _ => {
                    state[cur] = DONE;
                    break BlockId::new(cur as u32);
                }
            }
        };
        for b in path.drain(..) {
            state[b] = DONE;
            resolved[b] = target;
        }
    }
    resolved
}

fn thread_jumps(f: &mut MirFunction) {
    let resolved = resolve_targets(f);
    let r = |t: BlockId| resolved[t.index()];
    for block in &mut f.blocks {
        // Handler annotations thread the same way: an empty handler block
        // that just jumps on contains nothing an unwind could observe.
        if let Some(h) = &mut block.handler {
            *h = r(*h);
        }
        match &mut block.term {
            MirTerminator::Jump(t) => *t = r(*t),
            MirTerminator::Branch { then, else_, .. } => {
                *then = r(*then);
                *else_ = r(*else_);
            }
            MirTerminator::Return(_) | MirTerminator::Unreachable => {}
        }
    }
}

fn collapse_same_target_branches(f: &mut MirFunction) {
    for block in &mut f.blocks {
        if let MirTerminator::Branch { then, else_, .. } = &block.term {
            if then == else_ {
                block.term = MirTerminator::Jump(*then);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pyaot_mir::{CoerceInst, Const, MirInst, MirTerminator};
    use pyaot_types::{RawKind, Repr};
    use pyaot_utils::BlockId;

    use super::{coerce_round_trips, collapse_same_target_branches, thread_jumps};
    use crate::testutil::{function, l, op, single_block, verify_ok};

    #[test]
    fn coerce_round_trip_becomes_noop_move() {
        // l0: F64; l1 = box(l0); l2 = unbox(l1) → l2 = Coerce(l0)[F64→F64].
        let mut f = single_block(
            vec![
                Repr::Raw(RawKind::F64),
                Repr::Tagged,
                Repr::Raw(RawKind::F64),
            ],
            vec![
                MirInst::Const {
                    dst: l(0),
                    val: Const::Float(1.0),
                },
                MirInst::Coerce(
                    CoerceInst::new(l(1), op(0), Repr::Raw(RawKind::F64), Repr::Tagged).unwrap(),
                ),
                MirInst::Coerce(
                    CoerceInst::new(l(2), op(1), Repr::Tagged, Repr::Raw(RawKind::F64)).unwrap(),
                ),
            ],
            MirTerminator::Return(None),
        );
        coerce_round_trips(&mut f);
        match &f.blocks[0].insts[2] {
            MirInst::Coerce(c) => {
                assert_eq!(c.from(), c.to(), "must be the identity coercion");
                let pyaot_mir::Operand::Local(src) = *c.src();
                assert_eq!(src, l(0), "must read the original local");
            }
            other => panic!("expected Coerce, got {other:?}"),
        }
        verify_ok(&f);
    }

    #[test]
    fn round_trip_blocked_by_redefinition() {
        // The original local is overwritten between the two coerces — the
        // round-trip must NOT rewrite (it would read the new value).
        let mut f = single_block(
            vec![
                Repr::Raw(RawKind::F64),
                Repr::Tagged,
                Repr::Raw(RawKind::F64),
            ],
            vec![
                MirInst::Const {
                    dst: l(0),
                    val: Const::Float(1.0),
                },
                MirInst::Coerce(
                    CoerceInst::new(l(1), op(0), Repr::Raw(RawKind::F64), Repr::Tagged).unwrap(),
                ),
                MirInst::Const {
                    dst: l(0),
                    val: Const::Float(2.0),
                },
                MirInst::Coerce(
                    CoerceInst::new(l(2), op(1), Repr::Tagged, Repr::Raw(RawKind::F64)).unwrap(),
                ),
            ],
            MirTerminator::Return(None),
        );
        coerce_round_trips(&mut f);
        match &f.blocks[0].insts[3] {
            MirInst::Coerce(c) => {
                let pyaot_mir::Operand::Local(src) = *c.src();
                assert_eq!(src, l(1), "must still read the boxed value");
            }
            other => panic!("expected Coerce, got {other:?}"),
        }
        verify_ok(&f);
    }

    #[test]
    fn threads_through_empty_forwarder() {
        let mut f = function(
            vec![],
            vec![
                (vec![], MirTerminator::Jump(BlockId::new(1))),
                (vec![], MirTerminator::Jump(BlockId::new(2))),
                (vec![], MirTerminator::Return(None)),
            ],
        );
        thread_jumps(&mut f);
        assert!(matches!(f.blocks[0].term, MirTerminator::Jump(t) if t == BlockId::new(2)));
        verify_ok(&f);
    }

    #[test]
    fn jump_threading_is_loop_safe() {
        // 1 → 2 → 1: an empty cycle must not hang resolution.
        let mut f = function(
            vec![],
            vec![
                (vec![], MirTerminator::Jump(BlockId::new(1))),
                (vec![], MirTerminator::Jump(BlockId::new(2))),
                (vec![], MirTerminator::Jump(BlockId::new(1))),
            ],
        );
        thread_jumps(&mut f);
        verify_ok(&f);
    }

    #[test]
    fn memoized_resolution_matches_chain_walk() {
        // 0 → 1 → 2 → 3 (return); 4 → 5 → 6 → 5 (an empty cycle entered at 5);
        // 7 → 2 rejoins the first chain after it is already resolved.
        let mut f = function(
            vec![],
            vec![
                (vec![], MirTerminator::Jump(BlockId::new(1))),
                (vec![], MirTerminator::Jump(BlockId::new(2))),
                (vec![], MirTerminator::Jump(BlockId::new(3))),
                (vec![], MirTerminator::Return(None)),
                (vec![], MirTerminator::Jump(BlockId::new(5))),
                (vec![], MirTerminator::Jump(BlockId::new(6))),
                (vec![], MirTerminator::Jump(BlockId::new(5))),
                (vec![], MirTerminator::Jump(BlockId::new(2))),
            ],
        );
        let r: Vec<u32> = super::resolve_targets(&f)
            .iter()
            .map(|b| b.index() as u32)
            .collect();
        assert_eq!(r, vec![3, 3, 3, 3, 5, 5, 6, 3]);
        thread_jumps(&mut f);
        verify_ok(&f);
    }

    #[test]
    fn same_target_branch_collapses() {
        let mut f = function(
            vec![Repr::Raw(RawKind::I8)],
            vec![
                (
                    vec![MirInst::Const {
                        dst: l(0),
                        val: Const::Int(1),
                    }],
                    MirTerminator::Branch {
                        cond: op(0),
                        then: BlockId::new(1),
                        else_: BlockId::new(1),
                    },
                ),
                (vec![], MirTerminator::Return(None)),
            ],
        );
        collapse_same_target_branches(&mut f);
        assert!(matches!(f.blocks[0].term, MirTerminator::Jump(t) if t == BlockId::new(1)));
        verify_ok(&f);
    }
}
