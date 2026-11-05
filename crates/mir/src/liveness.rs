//! Backward liveness over the MIR CFG, computing which locals actually need a
//! GC root slot (PITFALLS B15, narrowed from "every rootable local" to real
//! dataflow).
//!
//! A local needs a root slot iff, at some instruction `I` that may trigger a
//! collection ([`MirInst::may_allocate`]), the local is either
//!
//! * **used by `I` itself** — the runtime does not root its own arguments, so
//!   a GC inside `rt_list_push(list, v)` must still see `list` and `v` through
//!   the caller's frame; or
//! * **live after `I`** (excluding `I`'s own def — the allocation precedes the
//!   definition, so the not-yet-written dst cannot be stale).
//!
//! plus the **handler rule**: a local live-in at any handler block
//! ([`crate::MirBlock::handler`]) is rooted. An unwind can leave a protected
//! block from *any* raising instruction in it — including one before a
//! mid-block redefinition — so ordinary CFG liveness through a block-level
//! handler edge under-approximates; rooting every handler live-in (root slot
//! + store-on-def) is sound and is exactly the old frame-stack behavior.
//!
//! The result only ever *narrows* the `Repr::is_gc_root()` set — rootness
//! itself stays derived from `Repr` (Invariant 5); a `Raw` local is never
//! rooted no matter what this analysis says. Codegen calls this on the final
//! post-optimizer MIR (a terminal forward consumer — no feedback into earlier
//! passes).
//!
//! The use/def tables are exhaustive matches with NO catch-all arm: a new
//! `MirInst`/`MirTerminator` variant is a compile error here, never a
//! silently-missed use (and so never a prematurely-collected value).

use pyaot_utils::LocalId;

use crate::{MirFunction, MirInst, MirTerminator, Operand};

/// A dense bitset over local indices (word-parallel dataflow).
#[derive(Clone, PartialEq, Eq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Self {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn get(&self, i: usize) -> bool {
        (self.0[i / 64] >> (i % 64)) & 1 == 1
    }
    fn set(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }
    fn clear(&mut self, i: usize) {
        self.0[i / 64] &= !(1 << (i % 64));
    }
    fn union_with(&mut self, o: &Bits) {
        for (a, b) in self.0.iter_mut().zip(&o.0) {
            *a |= *b;
        }
    }
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(w, &word)| {
            let mut bits = word;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let t = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(w * 64 + t)
            })
        })
    }
}

/// For each local (by index): must it get a GC root slot? `true` only for
/// locals whose `Repr::is_gc_root()` holds AND whose liveness crosses an
/// allocation point (see module docs).
///
/// Complexity: the dataflow runs on word-parallel bitsets over a worklist with
/// O(1) membership; the rooting pass is linear in the instruction count (plus
/// one sweep of the live set per block), using an allocation *epoch* counter
/// instead of scanning every local at every allocation point.
pub fn roots_needed(f: &MirFunction) -> Vec<bool> {
    let nlocals = f.locals.len();
    let nblocks = f.blocks.len();

    // ── per-block use/def summaries ──
    // block_use: read before any write within the block (backward scan);
    // block_def: written anywhere in the block.
    let mut block_use = vec![Bits::new(nlocals); nblocks];
    let mut block_def = vec![Bits::new(nlocals); nblocks];
    for (bi, b) in f.blocks.iter().enumerate() {
        let (use_b, def_b) = (&mut block_use[bi], &mut block_def[bi]);
        // Backward: a use before an earlier (in scan order: later in program
        // order) def stays a use; a def kills uses below it.
        term_uses(&b.term, |l| use_b.set(l.index()));
        for inst in b.insts.iter().rev() {
            if let Some(d) = inst.dst() {
                use_b.clear(d.index());
                def_b.set(d.index());
            }
            inst_uses(inst, |l| use_b.set(l.index()));
        }
    }

    // ── classic backward worklist: live_in/live_out per block ──
    // A protected block's handler is a CFG successor (control can transfer
    // there from any raising instruction in the block).
    let succs: Vec<Vec<usize>> = f.blocks.iter().map(successors).collect();
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nblocks];
    for (bi, ss) in succs.iter().enumerate() {
        for &s in ss {
            preds[s].push(bi);
        }
    }
    let mut live_in = vec![Bits::new(nlocals); nblocks];
    let mut live_out = vec![Bits::new(nlocals); nblocks];
    let mut work: Vec<usize> = (0..nblocks).collect();
    let mut in_work = vec![true; nblocks];
    while let Some(bi) = work.pop() {
        in_work[bi] = false;
        // live_out = ∪ live_in[succ]
        let mut out = Bits::new(nlocals);
        for &s in &succs[bi] {
            out.union_with(&live_in[s]);
        }
        // live_in = use ∪ (out \ def)
        let mut inn = block_use[bi].clone();
        for ((i, o), d) in inn.0.iter_mut().zip(&out.0).zip(&block_def[bi].0) {
            *i |= *o & !*d;
        }
        live_out[bi] = out;
        if inn != live_in[bi] {
            live_in[bi] = inn;
            for &p in &preds[bi] {
                if !in_work[p] {
                    in_work[p] = true;
                    work.push(p);
                }
            }
        }
    }

    // ── second backward pass: root locals live across / used by allocations ──
    // `epoch` counts the allocation points passed so far (backward) and
    // `since[l]` is the epoch at which `l` most recently became live. A local
    // whose live range (ending, backward, at its def or the block entry) saw
    // the epoch advance was live after some allocating instruction.
    let mut needed = vec![false; nlocals];
    let mut since = vec![0u64; nlocals];
    let mut epoch: u64 = 0;
    for (bi, b) in f.blocks.iter().enumerate() {
        // `live` tracks liveness AFTER the instruction under the cursor.
        let mut live = live_out[bi].clone();
        // Terminators never allocate (branch/return), so start below them:
        // fold their uses into `live` and walk the instructions.
        term_uses(&b.term, |l| live.set(l.index()));
        for l in live.iter() {
            since[l] = epoch;
        }
        for inst in b.insts.iter().rev() {
            // The def ends its live range here. The allocation (if any)
            // precedes the definition, so it never counts for `def` itself.
            if let Some(d) = inst.dst() {
                let d = d.index();
                if live.get(d) {
                    if epoch > since[d] {
                        needed[d] = true;
                    }
                    live.clear(d);
                }
            }
            let allocates = inst.may_allocate(&f.locals);
            if allocates {
                // needs_root(l) ⇐ l ∈ live_after(I) \ {def(I)}: every local
                // live at this point crosses the new epoch.
                epoch += 1;
            }
            inst_uses(inst, |l| {
                let l = l.index();
                // The runtime does not root its own arguments.
                if allocates {
                    needed[l] = true;
                }
                if !live.get(l) {
                    live.set(l);
                    since[l] = epoch;
                }
            });
        }
        // The block entry ends every remaining live range.
        for l in live.iter() {
            if epoch > since[l] {
                needed[l] = true;
            }
        }
    }

    // ── handler rule (see module docs) ──
    for b in &f.blocks {
        if let Some(handler) = b.handler {
            for l in live_in[handler.index()].iter() {
                needed[l] = true;
            }
        }
    }

    // Rootness is derived from `Repr` (Invariant 5) — liveness only narrows.
    for (l, n) in needed.iter_mut().enumerate() {
        *n &= f.locals[l].repr.is_gc_root();
    }
    needed
}

/// The CFG successors of a block: its terminator's targets plus its handler
/// edge, if protected (exhaustive over terminators — no catch-all).
fn successors(b: &crate::MirBlock) -> Vec<usize> {
    let mut succ = match &b.term {
        MirTerminator::Return(_) | MirTerminator::Unreachable => vec![],
        MirTerminator::Jump(t) => vec![t.index()],
        MirTerminator::Branch {
            cond: _,
            then,
            else_,
        } => vec![then.index(), else_.index()],
    };
    if let Some(h) = b.handler {
        succ.push(h.index());
    }
    succ
}

/// The locals a terminator reads (exhaustive — no catch-all).
fn term_uses(t: &MirTerminator, mut f: impl FnMut(LocalId)) {
    match t {
        MirTerminator::Return(op) => {
            if let Some(Operand::Local(l)) = op {
                f(*l);
            }
        }
        MirTerminator::Branch {
            cond: Operand::Local(l),
            then: _,
            else_: _,
        } => f(*l),
        MirTerminator::Jump(_) | MirTerminator::Unreachable => {}
    }
}

/// The locals an instruction reads — the def-vs-use complement of the canonical
/// [`MirInst::dst`] (used directly above). Adapts [`MirInst::for_each_operand`]
/// by mapping each `Operand` to its `LocalId`; both canonical sources are
/// exhaustive (no catch-all), so a new instruction variant is forced through GC
/// liveness at compile time.
fn inst_uses(inst: &MirInst, mut f: impl FnMut(LocalId)) {
    inst.for_each_operand(|op| {
        let Operand::Local(id) = op;
        f(*id);
    });
}

#[cfg(test)]
mod tests {

    fn interned_file() -> pyaot_utils::InternedString {
        pyaot_utils::StringInterner::new().intern("test.py")
    }
    use pyaot_types::{HeapShape, RawKind, Repr};
    use pyaot_utils::{BlockId, InternedString, LocalId, StringInterner};

    use super::*;
    use crate::{Const, LocalDecl, MirBlock, PrintKind};

    const STR: Repr = Repr::Heap(HeapShape::Str);
    const I64: Repr = Repr::Raw(RawKind::I64);

    fn interned(s: &str) -> InternedString {
        StringInterner::new().intern(s)
    }

    fn func(locals: Vec<Repr>, blocks: Vec<MirBlock>) -> MirFunction {
        MirFunction {
            name: interned("f"),
            file: interned_file(),
            params: Vec::new(),
            ret: Repr::Tagged,
            locals: locals.into_iter().map(|repr| LocalDecl { repr }).collect(),
            blocks,
            entry: BlockId::new(0),
        }
    }

    fn lid(i: u32) -> LocalId {
        LocalId::new(i)
    }

    fn op(i: u32) -> Operand {
        Operand::Local(lid(i))
    }

    /// `Const Str` — the canonical allocating instruction in these tests.
    fn alloc_str(dst: u32) -> MirInst {
        MirInst::Const {
            dst: lid(dst),
            val: Const::Str(interned("x")),
        }
    }

    /// `Print StrObj` — an allocating USER of its operand.
    fn print_str(arg: u32) -> MirInst {
        MirInst::Print {
            kind: PrintKind::StrObj,
            arg: Some(op(arg)),
        }
    }

    /// A non-allocating use (SetField value into an existing instance held in
    /// local `base`).
    fn quiet_use(base: u32, value: u32) -> MirInst {
        MirInst::SetField {
            base: op(base),
            slot: 0,
            value: op(value),
        }
    }

    #[test]
    fn dead_before_allocation_is_not_rooted() {
        // s0 := "a" (alloc); n2 := None; consume s0 (no alloc); s1 := "b"
        // (alloc); print s1. s0 and n2 are dead at the second allocation and
        // cross no allocation while live → no roots. s1 is used by the
        // allocating print → rooted.
        // Local 2 is a statically-known instance: a field store into it is
        // a plain store (a `Tagged` base would be shape-checked and could
        // raise, i.e. allocate).
        let inst = Repr::Heap(HeapShape::Class(pyaot_utils::ClassId::new(0)));
        let f = func(
            vec![STR, STR, inst],
            vec![MirBlock {
                insts: vec![
                    alloc_str(0),
                    MirInst::Const {
                        dst: lid(2),
                        val: Const::None,
                    },
                    quiet_use(2, 0),
                    alloc_str(1),
                    print_str(1),
                ],
                term: MirTerminator::Return(None),
                handler: None,
            }],
        );
        let needed = roots_needed(&f);
        assert!(
            !needed[0],
            "consumed before the next allocation -> not rooted"
        );
        assert!(needed[1], "used by the allocating print -> rooted");
        assert!(!needed[2], "dead before the allocation -> not rooted");
    }

    #[test]
    fn live_across_allocation_is_rooted() {
        // s0 := "a"; s1 := "b" (alloc with s0 live-after); print s0.
        let f = func(
            vec![STR, STR],
            vec![MirBlock {
                insts: vec![alloc_str(0), alloc_str(1), print_str(0)],
                term: MirTerminator::Return(None),
                handler: None,
            }],
        );
        let needed = roots_needed(&f);
        assert!(needed[0], "live across the allocation -> rooted");
        assert!(!needed[1], "dead right after its own def -> not rooted");
    }

    #[test]
    fn argument_of_allocating_call_is_rooted_even_if_dead_after() {
        // print s0 (allocating) where s0 dies at that use: the uses(I) rule.
        let f = func(
            vec![STR],
            vec![MirBlock {
                insts: vec![alloc_str(0), print_str(0)],
                term: MirTerminator::Return(None),
                handler: None,
            }],
        );
        assert!(
            roots_needed(&f)[0],
            "argument of an allocating inst must be rooted"
        );
    }

    #[test]
    fn alloc_dst_alone_is_not_rooted() {
        // s0 := "a" (alloc), never used afterwards and not live across
        // anything else → its own def does not root it.
        let f = func(
            vec![STR],
            vec![MirBlock {
                insts: vec![alloc_str(0)],
                term: MirTerminator::Return(None),
                handler: None,
            }],
        );
        assert!(
            !roots_needed(&f)[0],
            "an allocation's own dst is not rooted by that allocation"
        );
    }

    #[test]
    fn raw_local_is_never_rooted() {
        // A Raw(I64) local live across an allocation is not a GC root.
        let f = func(
            vec![I64, STR],
            vec![MirBlock {
                insts: vec![
                    MirInst::Const {
                        dst: lid(0),
                        val: Const::Int(7),
                    },
                    alloc_str(1),
                    quiet_use(1, 0),
                ],
                term: MirTerminator::Return(None),
                handler: None,
            }],
        );
        let needed = roots_needed(&f);
        assert!(
            !needed[0],
            "Raw local: rootness derives from Repr (Invariant 5)"
        );
    }

    #[test]
    fn loop_back_edge_keeps_local_live_through_allocation() {
        // b0: s0 := "a"; jump b1
        // b1: s1 := "b" (alloc); branch(cond raw) -> b1 | b2
        // b2: print s0
        // s0 is live around the loop, across the allocation in b1 -> rooted.
        let f = func(
            vec![STR, STR, Repr::Raw(RawKind::I8)],
            vec![
                MirBlock {
                    insts: vec![alloc_str(0)],
                    term: MirTerminator::Jump(BlockId::new(1)),
                    handler: None,
                },
                MirBlock {
                    insts: vec![alloc_str(1)],
                    term: MirTerminator::Branch {
                        cond: op(2),
                        then: BlockId::new(1),
                        else_: BlockId::new(2),
                    },
                    handler: None,
                },
                MirBlock {
                    insts: vec![print_str(0)],
                    term: MirTerminator::Return(None),
                    handler: None,
                },
            ],
        );
        let needed = roots_needed(&f);
        assert!(needed[0], "live through the loop allocation -> rooted");
    }

    #[test]
    fn branch_only_one_arm_allocates() {
        // b0: s0 := "a"; branch -> b1 | b2
        // b1: alloc; jump b3      (s0 live across -> rooted)
        // b2: jump b3             (no alloc)
        // b3: print s0 — print itself allocates and uses s0, so s0 is rooted
        // regardless; use a QUIET final consumer instead to isolate the arm.
        // b3: quiet_use(s0) ; return
        let f = func(
            vec![STR, STR, Repr::Raw(RawKind::I8), Repr::Tagged],
            vec![
                MirBlock {
                    insts: vec![alloc_str(0)],
                    term: MirTerminator::Branch {
                        cond: op(2),
                        then: BlockId::new(1),
                        else_: BlockId::new(2),
                    },
                    handler: None,
                },
                MirBlock {
                    insts: vec![alloc_str(1)],
                    term: MirTerminator::Jump(BlockId::new(3)),
                    handler: None,
                },
                MirBlock {
                    insts: vec![],
                    term: MirTerminator::Jump(BlockId::new(3)),
                    handler: None,
                },
                MirBlock {
                    insts: vec![quiet_use(3, 0)],
                    term: MirTerminator::Return(None),
                    handler: None,
                },
            ],
        );
        let needed = roots_needed(&f);
        assert!(needed[0], "live across the allocating arm -> rooted");
        assert!(!needed[1], "alloc dst dead after -> not rooted");
    }

    #[test]
    fn handler_live_in_is_rooted() {
        // b0: s0 := "a" (pre-try value printed in the handler); jump b1
        // b1 (protected try body, handler = b2): return
        // b2 (handler): print s0; return
        // The try body itself never allocates with s0 live, but the handler
        // rule must root s0 anyway (an unwind can enter b2 from any raising
        // instruction in b1).
        let f = func(
            vec![STR],
            vec![
                MirBlock {
                    insts: vec![alloc_str(0)],
                    term: MirTerminator::Jump(BlockId::new(1)),
                    handler: None,
                },
                MirBlock {
                    insts: vec![],
                    term: MirTerminator::Return(None),
                    handler: Some(BlockId::new(2)),
                },
                MirBlock {
                    insts: vec![print_str(0)],
                    term: MirTerminator::Return(None),
                    handler: None,
                },
            ],
        );
        assert!(
            roots_needed(&f)[0],
            "live-in at a handler -> rooted (handler rule)"
        );
    }
}
