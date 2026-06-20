//! Function inlining over direct `Call`s — the microgpt lever: hot autograd
//! dunders (`__add__`/`__mul__`) are tiny functions whose call overhead and
//! box/unbox churn dominate.
//!
//! ## Eligibility (v1)
//!
//! A direct `Call { func: F }` splices iff:
//! * F's body is ≤ `max_insts` instructions;
//! * F has no protected blocks (`MirBlock::handler` annotations) and no
//!   `MakeGenerator` / `GenOpInst` (handler-block remapping and generator
//!   state machines — a v1 refusal, not a soundness limit);
//! * F is not the caller and not in the caller's call-graph SCC (no
//!   self/mutual recursion).
//!
//! A `Raise` in the callee is fine: spliced blocks inherit the call site's
//! handler annotation, so an inlined raise lands in exactly the handler the
//! replaced call would have unwound to.
//!
//! ## Order
//!
//! Functions are processed callees-first (Tarjan SCC emission order over the
//! direct-call graph is reverse-topological), so a callee's body is FINAL
//! when its callers splice it — transitive bottom-up inlining in one pass.
//! Blocks copied from a callee are not re-scanned (any call still in them
//! was ineligible when the callee itself was processed).
//!
//! ## Splice
//!
//! Callee locals/blocks append with `+L` / `+B` remaps. The call block is
//! split at the call: parameters become materialized Noop-`Coerce` moves
//! (exact repr equality is verifier-guaranteed; moves, not substitution —
//! Python parameters are assignable), each callee `Return(op)` becomes a
//! Noop-`Coerce` into the call's `dst` plus a `Jump` to the continuation
//! block (which inherits the call block's suffix + original terminator).
//!
//! Functions are NEVER deleted: `FuncId`s are dense indices held by
//! `MakeClosure` / vtables / dunder tables / generator dispatch; dead bodies
//! are the linker's problem (`-dead_strip` / `--gc-sections`).

use pyaot_mir::{
    CoerceInst, Const, LocalDecl, MirBlock, MirFunction, MirInst, MirProgram, MirTerminator,
    Operand,
};
use pyaot_types::{RawKind, Repr};
use pyaot_utils::{BlockId, LocalId};

use crate::OptimizationPass;

pub struct Inline {
    pub max_insts: usize,
}

impl Default for Inline {
    fn default() -> Self {
        // Tuned on bench_calls / microgpt (9C.4). A standalone autograd-style
        // dunder (`__add__`: MakeInstance + MakeClosure + the box/unbox at
        // its own call boundaries — real code until the call is spliced and
        // the round-trips cancel) measures ~50-65 MIR instructions; 16 and 36
        // both missed the whole family, 64 catches it while leaving real
        // multi-block bodies (~100+) out.
        Self { max_insts: 64 }
    }
}

impl OptimizationPass for Inline {
    fn name(&self) -> &'static str {
        "inline"
    }

    fn run(&self, program: &mut MirProgram) {
        let scc_of = call_graph_sccs(&program.funcs);

        // Eligibility is judged on the AS-WRITTEN size, snapshotted before
        // any splice: bottom-up processing grows a callee's body with its own
        // inlined callees before its callers look at it, and judging that
        // grown (pre-cleanup) size would refuse exactly the small functions
        // this pass exists for. A grown body is still capped (8x) against
        // pathological chains.
        let original_ok: Vec<bool> = program
            .funcs
            .iter()
            .map(|f| inlineable(f, self.max_insts))
            .collect();

        // Functions that are the target of some `MakeClosure` are the uniform
        // value-call thunks (under the uniform convention, slot 0 of every
        // closure IS its arity-generic thunk). A thunk is a thin dispatch shim
        // for the COLD fallback path: inlining its specialized callee `F` into it
        // would both bloat that fallback AND sever the thunk→`F` link the `devirt`
        // pass (which runs right after) needs to rewrite a monomorphic
        // value-position call back to a direct `Call(F, …)`. So we never inline
        // INTO a thunk — keeping it a thin `bind args → Call F` shim.
        let is_thunk = makeclosure_targets(&program.funcs);

        // Callees-first: Tarjan emits an SCC only after every SCC it can
        // reach (its callees), so ascending emission index is bottom-up.
        let mut order: Vec<usize> = (0..program.funcs.len()).collect();
        order.sort_by_key(|&i| scc_of[i]);

        for caller in order {
            if is_thunk[caller] {
                continue;
            }
            inline_into(program, caller, &scc_of, &original_ok, self.max_insts);
        }
    }
}

/// The cloned parts of a callee needed for one splice.
struct CalleeBody {
    params: Vec<Repr>,
    locals: Vec<LocalDecl>,
    blocks: Vec<MirBlock>,
    entry: BlockId,
}

fn inline_into(
    program: &mut MirProgram,
    caller: usize,
    scc_of: &[usize],
    original_ok: &[bool],
    max_insts: usize,
) {
    // Worklist of caller-origin blocks to scan; continuation blocks created
    // by a splice join it, callee-copied blocks do not.
    let mut work: Vec<usize> = (0..program.funcs[caller].blocks.len()).collect();

    while let Some(bi) = work.pop() {
        // Find the first eligible call in this block (immutable scan).
        let site = {
            let funcs = &program.funcs;
            funcs[caller].blocks[bi].insts.iter().enumerate().find_map(|(i, inst)| {
                if let MirInst::Call { func, .. } = inst {
                    let callee = func.index();
                    let grown: usize = real_size(&funcs[callee]);
                    if callee != caller
                        && scc_of[callee] != scc_of[caller]
                        && original_ok[callee]
                        && grown <= max_insts * 8
                    {
                        return Some((i, callee));
                    }
                    if std::env::var_os("PYAOT_INLINE_DEBUG").is_some() {
                        eprintln!(
                            "inline refuse: caller={caller} callee={callee} scc={}/{} ok={} grown={grown}",
                            scc_of[callee], scc_of[caller], original_ok[callee],
                        );
                    }
                }
                None
            })
        };
        let Some((inst_idx, callee)) = site else {
            continue;
        };

        let body = clone_body(&program.funcs[callee]);
        let cont = splice(&mut program.funcs[caller], bi, inst_idx, &body);
        // The continuation holds the rest of this block — keep scanning it.
        work.push(cont);
    }
}

/// The set of functions that are the target of some `MakeClosure` (the uniform
/// value-call thunks). Indexed by `FuncId::index()`.
fn makeclosure_targets(funcs: &[MirFunction]) -> Vec<bool> {
    let mut is_target = vec![false; funcs.len()];
    for f in funcs {
        for block in &f.blocks {
            for inst in &block.insts {
                if let MirInst::MakeClosure { func, .. } = inst {
                    if func.index() < is_target.len() {
                        is_target[func.index()] = true;
                    }
                }
            }
        }
    }
    is_target
}

/// Instruction count for inlining decisions: `LineMarker`s are compile-time
/// metadata (no machine code) and must not shift the tuned size thresholds.
fn real_size(f: &MirFunction) -> usize {
    f.blocks
        .iter()
        .map(|b| {
            b.insts
                .iter()
                .filter(|i| !matches!(i, MirInst::LineMarker(_)))
                .count()
        })
        .sum()
}

/// May this function be spliced into a caller at all (size + v1 refusals)?
fn inlineable(f: &MirFunction, max_insts: usize) -> bool {
    if real_size(f) > max_insts {
        return false;
    }
    for block in &f.blocks {
        // A callee with its own protected region would need handler-block
        // remapping AND nesting into the call site's handler — refused (the
        // Phase-9 v1 rule, carried over to handler annotations).
        if block.handler.is_some() {
            return false;
        }
        for inst in &block.insts {
            if matches!(
                inst,
                MirInst::MakeGenerator { .. } | MirInst::GenOpInst { .. }
            ) {
                return false;
            }
        }
    }
    true
}

fn clone_body(f: &MirFunction) -> CalleeBody {
    CalleeBody {
        params: f.params.clone(),
        locals: f.locals.clone(),
        blocks: f
            .blocks
            .iter()
            .map(|b| MirBlock {
                insts: b.insts.clone(),
                term: b.term.clone(),
                handler: b.handler,
            })
            .collect(),
        entry: f.entry,
    }
}

/// Splice `body` over the `Call` at `caller.blocks[bi].insts[inst_idx]`.
/// Returns the continuation block's index.
fn splice(caller: &mut MirFunction, bi: usize, inst_idx: usize, body: &CalleeBody) -> usize {
    let l_off = caller.locals.len() as u32;
    let b_off = caller.blocks.len() as u32;
    let cont = BlockId::new(b_off + body.blocks.len() as u32);
    // Inlining into a protected block: every spliced callee block (and the
    // continuation) inherits the call site's handler — a raise from inlined
    // code must land in exactly the handler the call would have unwound to.
    // (Callee-own handlers cannot occur: `inlineable()` refuses them.)
    let site_handler = caller.blocks[bi].handler;

    // (1) Callee locals append after the caller's (params are locals 0..P).
    caller.locals.extend(body.locals.iter().cloned());

    // (3) Split the call block. The suffix + original terminator move to the
    // continuation block; the call block ends by jumping into the callee.
    let block = &mut caller.blocks[bi];
    let mut suffix = block.insts.split_off(inst_idx);
    let call = suffix.remove(0);
    let MirInst::Call { dst, args, .. } = call else {
        unreachable!("splice target must be a direct Call");
    };
    let orig_term = std::mem::replace(
        &mut block.term,
        MirTerminator::Jump(BlockId::new(b_off + body.entry.index() as u32)),
    );

    // (4) Materialized parameter moves: arg_i → callee local i (now l_off+i).
    // Exact repr equality is verifier-guaranteed at every Call site, so the
    // identity coercion is always constructible.
    for (i, (arg, repr)) in args.into_iter().zip(&body.params).enumerate() {
        let mv = CoerceInst::new(
            LocalId::new(l_off + i as u32),
            arg,
            repr.clone(),
            repr.clone(),
        )
        .expect("identity coercion is always legal");
        caller.blocks[bi].insts.push(MirInst::Coerce(mv));
    }

    // (2) Append remapped callee blocks; (5) rewrite each Return.
    for cb in &body.blocks {
        let mut insts: Vec<MirInst> = cb.insts.clone();
        for inst in &mut insts {
            inst.map_locals(|l| LocalId::new(l_off + l.index() as u32));
        }
        let rb = |b: BlockId| BlockId::new(b_off + b.index() as u32);
        let term = match &cb.term {
            MirTerminator::Return(op) => {
                if let Some(d) = dst {
                    match op {
                        // The return value moves into the call's dst (reprs are
                        // identical: callee.ret == dst's repr, verifier-checked).
                        Some(Operand::Local(src)) => {
                            let src = LocalId::new(l_off + src.index() as u32);
                            let ret_repr = caller.locals[d.index()].repr.clone();
                            let mv =
                                CoerceInst::new(d, Operand::Local(src), ret_repr.clone(), ret_repr)
                                    .expect("identity coercion is always legal");
                            insts.push(MirInst::Coerce(mv));
                        }
                        // A bare `return` / fall-off in a **value-returning** callee
                        // yields its return repr's default — the SAME value codegen's
                        // `default_ret` emits when not inlined (`Tagged → None`,
                        // `Raw(F64) → 0.0`, `Raw(I*) → 0`). Without this write the
                        // call's `dst` is left holding a STALE value across the
                        // inlined None path (a `def f(): if x: return x` then a bare
                        // `return` reads back the previous result, not `None`).
                        None => emit_default_ret(&mut insts, caller, d),
                    }
                }
                MirTerminator::Jump(cont)
            }
            MirTerminator::Jump(t) => MirTerminator::Jump(rb(*t)),
            MirTerminator::Branch { cond, then, else_ } => {
                let Operand::Local(c) = cond;
                MirTerminator::Branch {
                    cond: Operand::Local(LocalId::new(l_off + c.index() as u32)),
                    then: rb(*then),
                    else_: rb(*else_),
                }
            }
            MirTerminator::Unreachable => MirTerminator::Unreachable,
        };
        caller.blocks.push(MirBlock {
            insts,
            term,
            handler: site_handler,
        });
    }

    // The continuation block: the call block's suffix + original terminator.
    caller.blocks.push(MirBlock {
        insts: suffix,
        term: orig_term,
        handler: site_handler,
    });
    cont.index()
}

/// Write the default return value for `d`'s repr — the value an inlined callee's
/// bare `return` / fall-off (`MirTerminator::Return(None)`) must store into the
/// call's `dst`, mirroring codegen's [`default_ret`] for the non-inlined ABI:
/// `Tagged → None`, `Raw(F64) → 0.0`, `Raw(I*) → 0`. A typed-heap / closure
/// return (rare for a bare return) takes the `None` bits via a `Tagged` temp + the
/// gradual retype — the same `NONE_TAG` codegen returns for an I64-class slot.
fn emit_default_ret(insts: &mut Vec<MirInst>, caller: &mut MirFunction, d: LocalId) {
    let repr = caller.locals[d.index()].repr.clone();
    let val = match &repr {
        Repr::Raw(RawKind::F64) => Const::Float(0.0),
        Repr::Raw(_) => Const::Int(0),
        Repr::Tagged => Const::None,
        _ => {
            let tmp = LocalId::new(caller.locals.len() as u32);
            caller.locals.push(LocalDecl { repr: Repr::Tagged });
            insts.push(MirInst::Const {
                dst: tmp,
                val: Const::None,
            });
            let mv = CoerceInst::new(d, Operand::Local(tmp), Repr::Tagged, repr)
                .expect("Tagged -> heap/closure is the gradual retype");
            insts.push(MirInst::Coerce(mv));
            return;
        }
    };
    insts.push(MirInst::Const { dst: d, val });
}

/// Tarjan SCC over the direct-call graph (iterative). Returns each
/// function's SCC **emission index** — ascending order is callees-first.
fn call_graph_sccs(funcs: &[MirFunction]) -> Vec<usize> {
    let n = funcs.len();
    let succs: Vec<Vec<usize>> = funcs
        .iter()
        .map(|f| {
            let mut out = Vec::new();
            for block in &f.blocks {
                for inst in &block.insts {
                    if let MirInst::Call { func, .. } = inst {
                        out.push(func.index());
                    }
                }
            }
            out
        })
        .collect();

    const UNVISITED: usize = usize::MAX;
    let mut index = vec![UNVISITED; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut scc_of = vec![UNVISITED; n];
    let mut next = 0usize;
    let mut scc_count = 0usize;

    for root in 0..n {
        if index[root] != UNVISITED {
            continue;
        }
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;

        while let Some(frame) = frames.last_mut() {
            let v = frame.0;
            if frame.1 < succs[v].len() {
                let w = succs[v][frame.1];
                frame.1 += 1;
                if index[w] == UNVISITED {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    frames.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                frames.pop();
                if let Some(parent) = frames.last() {
                    let p = parent.0;
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    loop {
                        let w = stack.pop().expect("Tarjan stack underflow");
                        on_stack[w] = false;
                        scc_of[w] = scc_count;
                        if w == v {
                            break;
                        }
                    }
                    scc_count += 1;
                }
            }
        }
    }
    scc_of
}

#[cfg(test)]
mod tests {

    fn interned_file() -> pyaot_utils::InternedString {
        pyaot_utils::StringInterner::new().intern("test.py")
    }
    use pyaot_mir::{
        verify, BinOp, Const, LocalDecl, MirBlock, MirFunction, MirInst, MirProgram, MirTerminator,
        StrPool,
    };
    use pyaot_types::{RawKind, Repr};
    use pyaot_utils::{BlockId, FuncId};

    use super::Inline;
    use crate::testutil::{interned, l, op};
    use crate::OptimizationPass;

    fn func(
        params: Vec<Repr>,
        ret: Repr,
        locals: Vec<Repr>,
        blocks: Vec<(Vec<MirInst>, MirTerminator)>,
    ) -> MirFunction {
        MirFunction {
            name: interned("f"),
            file: interned_file(),
            params,
            ret,
            locals: locals.into_iter().map(|repr| LocalDecl { repr }).collect(),
            blocks: blocks
                .into_iter()
                .map(|(insts, term)| MirBlock {
                    insts,
                    term,
                    handler: None,
                })
                .collect(),
            entry: BlockId::new(0),
        }
    }

    fn program(funcs: Vec<MirFunction>) -> MirProgram {
        MirProgram {
            funcs,
            entry: FuncId::new(0),
            str_pool: StrPool::new(),
            classes: Vec::new(),
            generators: Vec::new(),
        }
    }

    fn verify_all(p: &MirProgram) {
        for f in &p.funcs {
            verify(f, &p.funcs).expect("inlined program must verify");
        }
    }

    /// callee: raw add of its two params.
    fn raw_add_callee() -> MirFunction {
        let i64r = Repr::Raw(RawKind::I64);
        func(
            vec![i64r.clone(), i64r.clone()],
            i64r.clone(),
            vec![i64r.clone(), i64r.clone(), i64r],
            vec![(
                vec![MirInst::BinOp {
                    dst: l(2),
                    op: BinOp::Add,
                    l: op(0),
                    r: op(1),
                }],
                MirTerminator::Return(Some(op(2))),
            )],
        )
    }

    #[test]
    fn simple_splice() {
        let i64r = Repr::Raw(RawKind::I64);
        let caller = func(
            vec![],
            Repr::Tagged,
            vec![i64r.clone(), i64r.clone(), i64r.clone()],
            vec![(
                vec![
                    MirInst::Const {
                        dst: l(0),
                        val: Const::Int(2),
                    },
                    MirInst::Const {
                        dst: l(1),
                        val: Const::Int(3),
                    },
                    MirInst::Call {
                        dst: Some(l(2)),
                        func: FuncId::new(1),
                        args: vec![op(0), op(1)],
                    },
                ],
                MirTerminator::Return(None),
            )],
        );
        let mut p = program(vec![caller, raw_add_callee()]);
        Inline::default().run(&mut p);

        let f = &p.funcs[0];
        // The Call is gone; the call block jumps into the spliced body.
        assert!(
            !f.blocks
                .iter()
                .any(|b| b.insts.iter().any(|i| matches!(i, MirInst::Call { .. }))),
            "direct call must be spliced away"
        );
        assert!(
            f.blocks.len() > 1,
            "splice must add callee + continuation blocks"
        );
        assert_eq!(f.locals.len(), 3 + 3, "callee locals must append");
        verify_all(&p);
    }

    #[test]
    fn recursive_call_refused() {
        let i64r = Repr::Raw(RawKind::I64);
        // f(x) calls itself — same SCC, must survive.
        let f0 = func(
            vec![i64r.clone()],
            i64r.clone(),
            vec![i64r.clone(), i64r.clone()],
            vec![(
                vec![MirInst::Call {
                    dst: Some(l(1)),
                    func: FuncId::new(0),
                    args: vec![op(0)],
                }],
                MirTerminator::Return(Some(op(1))),
            )],
        );
        let mut p = program(vec![f0]);
        Inline::default().run(&mut p);
        assert!(p.funcs[0]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i, MirInst::Call { .. }))));
        verify_all(&p);
    }

    #[test]
    fn mutual_recursion_refused() {
        let i64r = Repr::Raw(RawKind::I64);
        let make = |target: u32| {
            func(
                vec![i64r.clone()],
                i64r.clone(),
                vec![i64r.clone(), i64r.clone()],
                vec![(
                    vec![MirInst::Call {
                        dst: Some(l(1)),
                        func: FuncId::new(target),
                        args: vec![op(0)],
                    }],
                    MirTerminator::Return(Some(op(1))),
                )],
            )
        };
        let mut p = program(vec![make(1), make(0)]);
        Inline::default().run(&mut p);
        for f in &p.funcs {
            assert!(f
                .blocks
                .iter()
                .any(|b| b.insts.iter().any(|i| matches!(i, MirInst::Call { .. }))));
        }
        verify_all(&p);
    }

    #[test]
    fn protected_callee_refused() {
        let i64r = Repr::Raw(RawKind::I64);
        let mut callee = func(
            vec![],
            Repr::Tagged,
            vec![],
            vec![
                (vec![], MirTerminator::Jump(BlockId::new(1))),
                (vec![], MirTerminator::Return(None)),
                (vec![], MirTerminator::Return(None)),
            ],
        );
        // A callee with its own protected region must be refused.
        callee.blocks[1].handler = Some(BlockId::new(2));
        let caller = func(
            vec![],
            Repr::Tagged,
            vec![i64r],
            vec![(
                vec![MirInst::Call {
                    dst: None,
                    func: FuncId::new(1),
                    args: vec![],
                }],
                MirTerminator::Return(None),
            )],
        );
        let mut p = program(vec![caller, callee]);
        Inline::default().run(&mut p);
        assert!(p.funcs[0].blocks[0]
            .insts
            .iter()
            .any(|i| matches!(i, MirInst::Call { .. })));
        verify_all(&p);
    }

    #[test]
    fn multi_return_callee() {
        let i64r = Repr::Raw(RawKind::I64);
        let i8r = Repr::Raw(RawKind::I8);
        // callee(x): if <const cond> return x+1 else return x+2 — two Returns.
        let callee = func(
            vec![i64r.clone()],
            i64r.clone(),
            vec![
                i64r.clone(),
                i8r,
                i64r.clone(),
                i64r.clone(),
                i64r.clone(),
                i64r.clone(),
            ],
            vec![
                (
                    vec![MirInst::Const {
                        dst: l(1),
                        val: Const::Int(1),
                    }],
                    MirTerminator::Branch {
                        cond: op(1),
                        then: BlockId::new(1),
                        else_: BlockId::new(2),
                    },
                ),
                (
                    vec![
                        MirInst::Const {
                            dst: l(2),
                            val: Const::Int(1),
                        },
                        MirInst::BinOp {
                            dst: l(3),
                            op: BinOp::Add,
                            l: op(0),
                            r: op(2),
                        },
                    ],
                    MirTerminator::Return(Some(op(3))),
                ),
                (
                    vec![
                        MirInst::Const {
                            dst: l(4),
                            val: Const::Int(2),
                        },
                        MirInst::BinOp {
                            dst: l(5),
                            op: BinOp::Add,
                            l: op(0),
                            r: op(4),
                        },
                    ],
                    MirTerminator::Return(Some(op(5))),
                ),
            ],
        );
        let caller = func(
            vec![],
            Repr::Tagged,
            vec![i64r.clone(), i64r.clone()],
            vec![(
                vec![
                    MirInst::Const {
                        dst: l(0),
                        val: Const::Int(10),
                    },
                    MirInst::Call {
                        dst: Some(l(1)),
                        func: FuncId::new(1),
                        args: vec![op(0)],
                    },
                ],
                MirTerminator::Return(None),
            )],
        );
        let mut p = program(vec![caller, callee]);
        Inline::default().run(&mut p);
        assert!(!p.funcs[0]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i, MirInst::Call { .. }))));
        verify_all(&p);
    }

    #[test]
    fn void_arity_zero_callee() {
        let callee = func(
            vec![],
            Repr::Tagged,
            vec![Repr::Tagged],
            vec![(
                vec![MirInst::Const {
                    dst: l(0),
                    val: Const::None,
                }],
                MirTerminator::Return(Some(op(0))),
            )],
        );
        // Caller discards the result (dst: None).
        let caller = func(
            vec![],
            Repr::Tagged,
            vec![],
            vec![(
                vec![MirInst::Call {
                    dst: None,
                    func: FuncId::new(1),
                    args: vec![],
                }],
                MirTerminator::Return(None),
            )],
        );
        let mut p = program(vec![caller, callee]);
        Inline::default().run(&mut p);
        assert!(!p.funcs[0]
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i, MirInst::Call { .. }))));
        verify_all(&p);
    }

    #[test]
    fn transitive_bottom_up_inline() {
        let i64r = Repr::Raw(RawKind::I64);
        // h = leaf; g calls h; f calls g. Bottom-up order means f ends up
        // with h's body too (g was final when f spliced it).
        let h = raw_add_callee();
        let g = func(
            vec![i64r.clone(), i64r.clone()],
            i64r.clone(),
            vec![i64r.clone(), i64r.clone(), i64r.clone()],
            vec![(
                vec![MirInst::Call {
                    dst: Some(l(2)),
                    func: FuncId::new(2),
                    args: vec![op(0), op(1)],
                }],
                MirTerminator::Return(Some(op(2))),
            )],
        );
        let f0 = func(
            vec![],
            Repr::Tagged,
            vec![i64r.clone(), i64r.clone(), i64r.clone()],
            vec![(
                vec![
                    MirInst::Const {
                        dst: l(0),
                        val: Const::Int(1),
                    },
                    MirInst::Const {
                        dst: l(1),
                        val: Const::Int(2),
                    },
                    MirInst::Call {
                        dst: Some(l(2)),
                        func: FuncId::new(1),
                        args: vec![op(0), op(1)],
                    },
                ],
                MirTerminator::Return(None),
            )],
        );
        let mut p = program(vec![f0, g, h]);
        Inline::default().run(&mut p);
        for f in &p.funcs {
            assert!(
                !f.blocks
                    .iter()
                    .any(|b| b.insts.iter().any(|i| matches!(i, MirInst::Call { .. }))),
                "every direct call must be gone after bottom-up inlining"
            );
        }
        verify_all(&p);
    }
}
