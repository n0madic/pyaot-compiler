//! Devirtualization of monomorphic value-position closure calls.
//!
//! The uniform value-call convention gives every closure ONE arity-generic ABI:
//! a value-position call `f(a, b)` lowers (in `lowering::lower_indirect_call`) to
//! a `CallIndirect` through [`generic_sig`] that **allocates a positional
//! args-tuple per call** and dispatches through the closure's slot-0 uniform
//! thunk (which re-binds the args and makes a direct call to the specialized
//! function `F`). That is the accepted, sound cost of making a genuinely-`Dyn`
//! callee callable — but it regresses HOF/closure-heavy hot loops.
//!
//! This pass *recovers* the specialized native ABI for **monomorphic** call
//! sites: when the closure's identity is statically known, it rewrites the
//! `CallIndirect` back to a direct [`MirInst::Call`] to `F`, recovering the
//! individual positional args from the args-tuple builder and deleting that now
//! dead builder. The args-tuple allocation, the slot-0 read, the indirect
//! dispatch, and the thunk's re-bind all disappear; the result reproduces
//! exactly the pre-uniform specialized call.
//!
//! It is a clean layer on top of the existing specialized-`Call` ABI that by-name
//! calls already use — never a parallel ABI, never a marker bit (Invariant 3 /
//! PITFALLS A4). The uniform path remains the always-correct fallback: any unmet
//! precondition leaves the `CallIndirect` untouched (graceful degradation). The
//! MIR verifier (debug + release, every pass boundary) and the differential gate
//! are the backstop.
//!
//! ## Where it runs
//!
//! Registered in [`PassManager::phase9`](crate::PassManager::phase9) immediately
//! AFTER `inline::Inline`, so a post-inline `MakeClosure` (e.g. `make_step`
//! inlined into `main`, exposing the `step` closure + the hot-loop `CallIndirect`
//! in one function) is visible to this intraprocedural pass.
//!
//! ## Soundness preconditions
//!
//! For a `CallIndirect { callee: Local(c), args: [args_op, kwargs_op], sig }` with
//! `sig == generic_sig()`:
//! 1. **No keywords:** `kwargs_op`'s single static def is `Const::NullPtr` (the
//!    `lower_indirect_call` shape; a `CallValue` with real kwargs is skipped).
//! 2. **Known closure:** following `c` through single-def bit-identity `Coerce`
//!    links reaches a `MakeClosure { func: thunk_fid }` (monomorphic — any
//!    multi-def or non-`Coerce`/non-`MakeClosure` step bails).
//! 3. **Simple thunk → `F`:** `program.funcs[thunk_fid]` has exactly ONE
//!    `MirInst::Call` (→ `F`), is structurally simple (no varargs/`**kwargs`
//!    machinery — no `TupleFromIter`/`Dict*` container ops, no `CallRuntime`/other
//!    indirect calls), and forwards `env` (local 0) as `F`'s first arg iff its
//!    `Call`'s first arg is `Local(0)`.
//! 4. **Non-escaping args-tuple:** `args_op`'s def is `Coerce(tup:
//!    Heap(TupleVar) -> Tagged)`, `tup`'s def is `CallContainer { TupleNew }`, its
//!    `TupleSet`s fill contiguous slots `0..n`, and `tup`/`args_op` are used by
//!    nothing else (read-count proof).
//! 5. **Exact arity:** `F.params.len() == (pass_env as usize) + n` — combined with
//!    (3) this guarantees the recovered args exactly fill `F`'s positional params.
//!
//! Single static def (`def_count == 1`) is the soundness anchor: one static def
//! always reaches every use, so no dominance analysis is needed.

use std::collections::HashSet;

use pyaot_mir::{
    classify_coercion, CoerceInst, Coercion, Const, ContainerOp, LocalDecl, MirBlock, MirFunction,
    MirInst, MirProgram, Operand,
};
use pyaot_types::{generic_sig, HeapShape, RawKind, Repr};
use pyaot_utils::{FuncId, LocalId};

use crate::analysis::read_counts;
use crate::OptimizationPass;

/// One block's pending rewrite: the set of block-instruction indices to drop
/// (the now-dead args-tuple builders) plus `(call_indirect_index, replacement)`
/// pairs that splice each devirtualized call in place.
type BlockEdit = (HashSet<usize>, Vec<(usize, Vec<MirInst>)>);

/// The specialized direct-call target recovered from a uniform thunk.
struct DevirtTarget {
    /// The specialized function `F` the thunk calls.
    f_fid: FuncId,
    /// Whether the thunk forwards its `env` (local 0) as `F`'s first arg
    /// (nested defs / lambdas pass env; top-level fn-values do not).
    pass_env: bool,
    /// `F`'s parameter reprs (cloned so the rewrite never re-borrows `program`).
    f_params: Vec<Repr>,
    /// `F`'s return repr.
    f_ret: Repr,
}

pub struct Devirt;

impl OptimizationPass for Devirt {
    fn name(&self) -> &'static str {
        "devirt"
    }

    fn run(&self, program: &mut MirProgram) {
        // Classify every function as a potential uniform thunk up front (an
        // immutable read of the whole table), so the per-function rewrite below
        // never needs to borrow `program.funcs` while mutating one function.
        let thunk_targets: Vec<Option<DevirtTarget>> = (0..program.funcs.len())
            .map(|fid| analyze_thunk(FuncId::new(fid as u32), &program.funcs))
            .collect();

        for func in &mut program.funcs {
            run_func(func, &thunk_targets);
        }
    }
}

/// Classify `funcs[fid]` as a simple uniform thunk over a specialized `F`, or
/// `None` if it is not a devirtualizable thunk. A simple positional thunk binds a
/// packed `__args__` tuple to `F`'s parameters and makes exactly ONE direct call
/// to `F`; anything carrying varargs/`**kwargs` machinery (a `TupleFromIter`/
/// `Dict*` container op, a `CallRuntime` — e.g. the `*args` slice — or another
/// indirect call) is refused.
fn analyze_thunk(fid: FuncId, funcs: &[MirFunction]) -> Option<DevirtTarget> {
    let thunk = &funcs[fid.index()];
    let mut found: Option<(FuncId, bool)> = None;
    for block in &thunk.blocks {
        for inst in &block.insts {
            match inst {
                MirInst::Call { func, args, .. } => {
                    if found.is_some() {
                        return None; // more than one direct call → not simple
                    }
                    // `env` is forwarded iff the thunk's call leads with local 0.
                    let pass_env =
                        matches!(args.first(), Some(Operand::Local(l)) if l.index() == 0);
                    found = Some((*func, pass_env));
                }
                // Varargs / `**kwargs` / default-dict machinery markers.
                MirInst::CallContainer { op, .. } if is_machinery_op(*op) => return None,
                // Any other call-like instruction means the body is not a plain
                // positional binder (a `*args` slice lowers to `CallRuntime`).
                MirInst::CallRuntime { .. }
                | MirInst::CallIndirect { .. }
                | MirInst::CallVirtual { .. }
                | MirInst::CallBuiltin { .. } => return None,
                _ => {}
            }
        }
    }
    let (f_fid, pass_env) = found?;
    let f = funcs.get(f_fid.index())?;
    Some(DevirtTarget {
        f_fid,
        pass_env,
        f_params: f.params.clone(),
        f_ret: f.ret.clone(),
    })
}

/// Container ops that appear ONLY in a varargs / `**kwargs` / default-filling
/// thunk — their presence proves the thunk is not a simple positional binder.
fn is_machinery_op(op: ContainerOp) -> bool {
    matches!(
        op,
        ContainerOp::TupleFromIter
            | ContainerOp::ListFromIter
            | ContainerOp::DictFromPairs
            | ContainerOp::DictNew
            | ContainerOp::DictGet
            | ContainerOp::DictGetDefault
            | ContainerOp::DictSet
            | ContainerOp::DictSetdefault
            | ContainerOp::DictUpdate
            | ContainerOp::DictPopM
    )
}

/// Per-function single static-definition map: `def[l]` is the instruction that
/// writes local `l` (cloned), and `count[l]` how many instructions write it. A
/// local is single-def iff `count[l] == 1`; its one static def then reaches every
/// use (the soundness anchor — no dominance analysis needed).
fn single_def(f: &MirFunction) -> (Vec<Option<MirInst>>, Vec<u32>) {
    let n = f.locals.len();
    let mut count = vec![0u32; n];
    let mut def: Vec<Option<MirInst>> = vec![None; n];
    for block in &f.blocks {
        for inst in &block.insts {
            if let Some(d) = inst.dst() {
                count[d.index()] += 1;
                def[d.index()] = Some(inst.clone());
            }
        }
    }
    (def, count)
}

/// Follow `c` through single-def bit-identity `Coerce` links (closure ↔ Tagged ↔
/// heap are all bit-identical) to the `MakeClosure { func }` that produced it.
/// Bails on any multi-def or non-`Coerce`/non-`MakeClosure` step (monomorphic
/// only).
fn resolve_to_makeclosure(c: LocalId, def: &[Option<MirInst>], count: &[u32]) -> Option<FuncId> {
    let mut cur = c;
    // Bounded walk: the coercion chain is short; the cap only guards against a
    // pathological cycle (which single-def MIR cannot actually form).
    for _ in 0..64 {
        if count[cur.index()] != 1 {
            return None;
        }
        match def[cur.index()].as_ref()? {
            MirInst::MakeClosure { func, .. } => return Some(*func),
            MirInst::Coerce(co) if is_bit_identity(co) => {
                let Operand::Local(src) = co.src();
                cur = *src;
            }
            _ => return None,
        }
    }
    None
}

/// If `a0`'s single static def is a bit-identity `Coerce(Heap(TupleVar) ->
/// Tagged)`, return its source tuple local. The shape `emit_container` emits for
/// a `TupleSet`'s coerced container arg (PITFALLS A5).
fn coerce_to_tup(a0: LocalId, def: &[Option<MirInst>], count: &[u32]) -> Option<LocalId> {
    if count[a0.index()] != 1 {
        return None;
    }
    let Some(MirInst::Coerce(co)) = def[a0.index()].as_ref() else {
        return None;
    };
    if co.checked()
        || !matches!(co.from(), Repr::Heap(HeapShape::TupleVar(_)))
        || *co.to() != Repr::Tagged
    {
        return None;
    }
    let Operand::Local(src) = co.src();
    Some(*src)
}

/// Trace a `Raw(I64)` constant local back to its literal value through the
/// `Const::Int → Coerce(Tagged → Raw(I64))` chain that `raw_i64_const` emits in
/// lowering. Returns `None` if the def isn't that constant shape (so the caller
/// bails the rewrite rather than trusting an unverified slot order).
fn raw_i64_const_value(local: LocalId, def: &[Option<MirInst>]) -> Option<i64> {
    let mut cur = local;
    for _ in 0..4 {
        match def[cur.index()].as_ref()? {
            MirInst::Const {
                val: Const::Int(n), ..
            } => return Some(*n),
            MirInst::Coerce(co) => {
                let Operand::Local(src) = co.src();
                cur = *src;
            }
            _ => return None,
        }
    }
    None
}

/// A `Coerce` that re-types a value without reinterpreting its bits — the
/// closure ↔ Tagged ↔ heap-pointer family. Box/unbox/tag/untag (which change the
/// interpreted type) and checked unboxes are excluded.
fn is_bit_identity(co: &CoerceInst) -> bool {
    !co.checked()
        && matches!(
            classify_coercion(co.from(), co.to()),
            Some(Coercion::Noop | Coercion::HeapToTagged | Coercion::TaggedToHeap)
        )
}

fn run_func(f: &mut MirFunction, thunk_targets: &[Option<DevirtTarget>]) {
    let reads = read_counts(f);
    let (def, count) = single_def(f);

    // New locals (arg/env coercion temps, result re-box temps) append after the
    // existing locals; their ids are `base + offset`.
    let base = f.locals.len() as u32;
    let mut new_locals: Vec<LocalDecl> = Vec::new();

    // Per block: collect the rewrite of each devirtualizable `CallIndirect` plus
    // the block-instruction indices its now-dead args-tuple builder occupies.
    let mut block_edits: Vec<Option<BlockEdit>> = vec![None; f.blocks.len()];

    for (bi, block) in f.blocks.iter().enumerate() {
        let mut dead: HashSet<usize> = HashSet::new();
        let mut repls: Vec<(usize, Vec<MirInst>)> = Vec::new();
        for (ci_idx, inst) in block.insts.iter().enumerate() {
            if !matches!(inst, MirInst::CallIndirect { .. }) {
                continue;
            }
            if let Some((plan_dead, replacement)) = try_devirt_ci(
                f,
                block,
                inst,
                &reads,
                &def,
                &count,
                thunk_targets,
                base,
                &mut new_locals,
            ) {
                dead.extend(plan_dead);
                repls.push((ci_idx, replacement));
            }
        }
        if !repls.is_empty() {
            block_edits[bi] = Some((dead, repls));
        }
    }

    f.locals.extend(new_locals);

    for (bi, edit) in block_edits.into_iter().enumerate() {
        let Some((dead, repls)) = edit else { continue };
        let repl_at: std::collections::HashMap<usize, Vec<MirInst>> = repls.into_iter().collect();
        let old = std::mem::take(&mut f.blocks[bi].insts);
        let mut out = Vec::with_capacity(old.len());
        for (i, inst) in old.into_iter().enumerate() {
            if let Some(replacement) = repl_at.get(&i) {
                // Replace the `CallIndirect` with the direct-`Call` sequence.
                out.extend(replacement.iter().cloned());
            } else if dead.contains(&i) {
                // Drop the now-dead args-tuple builder (impure `TupleNew`/
                // `TupleSet`s + the tuple `Coerce`); the leftover `kwargs` /
                // index `Const`s are pure and swept by the trailing DCE.
            } else {
                out.push(inst);
            }
        }
        f.blocks[bi].insts = out;
    }
}

/// Try to devirtualize one `CallIndirect`. Returns the block-instruction indices
/// of its now-dead args-tuple builder and the replacement instruction sequence
/// (the arg/env coercions, the direct `Call`, the optional result re-box), or
/// `None` if any precondition is unmet (leave the `CallIndirect` as-is).
#[allow(clippy::too_many_arguments)]
fn try_devirt_ci(
    f: &MirFunction,
    block: &MirBlock,
    ci: &MirInst,
    reads: &[u32],
    def: &[Option<MirInst>],
    count: &[u32],
    thunk_targets: &[Option<DevirtTarget>],
    base: u32,
    new_locals: &mut Vec<LocalDecl>,
) -> Option<(HashSet<usize>, Vec<MirInst>)> {
    let MirInst::CallIndirect {
        dst: ci_dst,
        callee,
        args,
        sig,
    } = ci
    else {
        return None;
    };
    // Only the simple positional `lower_indirect_call` shape: the carried sig is
    // exactly `generic_sig()` and the operands are `[args_tuple, kwargs]`.
    if *sig != generic_sig() || args.len() != 2 {
        return None;
    }
    let Operand::Local(c) = callee;
    let Operand::Local(args_op) = &args[0];
    let Operand::Local(kwargs_op) = &args[1];
    let (c, args_op, kwargs_op) = (*c, *args_op, *kwargs_op);

    // (1) No keywords: `kwargs` is the null sentinel from a single static def.
    if count[kwargs_op.index()] != 1 {
        return None;
    }
    match def[kwargs_op.index()].as_ref()? {
        MirInst::Const {
            val: Const::NullPtr,
            ..
        } => {}
        _ => return None,
    }

    // (2) Known closure → its uniform thunk → the specialized `F`.
    let thunk_fid = resolve_to_makeclosure(c, def, count)?;
    let target = thunk_targets.get(thunk_fid.index())?.as_ref()?;

    // (4) Recover the positional args from the (non-escaping) args-tuple builder.
    //     `args_op` is `Coerce(tup: Heap(TupleVar) -> Tagged)`.
    if count[args_op.index()] != 1 {
        return None;
    }
    let Some(MirInst::Coerce(co)) = def[args_op.index()].as_ref() else {
        return None;
    };
    if co.checked()
        || !matches!(co.from(), Repr::Heap(HeapShape::TupleVar(_)))
        || *co.to() != Repr::Tagged
    {
        return None;
    }
    let Operand::Local(tup) = co.src();
    let tup = *tup;
    // `tup`'s def is `CallContainer { TupleNew }`.
    if count[tup.index()] != 1 {
        return None;
    }
    if !matches!(
        def[tup.index()].as_ref(),
        Some(MirInst::CallContainer {
            op: ContainerOp::TupleNew,
            ..
        })
    ) {
        return None;
    }

    // Collect the `TupleSet`s into `tup` in textual order (= slot order: lowering
    // emits slot 0, 1, … in sequence). `emit_container` coerces the container arg
    // to `Tagged` (PITFALLS A5), so a `TupleSet`'s arg 0 is a per-slot
    // bit-identity `Coerce(tup -> Tagged)`, not `tup` itself; the recovered value
    // is arg 2 (already `Tagged`). Each such arg-0 coerce is single-use and is
    // deleted with its `TupleSet`.
    let mut vals: Vec<Operand> = Vec::new();
    let mut dead_builder: Vec<usize> = Vec::new();
    for (j, inst) in block.insts.iter().enumerate() {
        let MirInst::CallContainer {
            op: ContainerOp::TupleSet,
            args: ts,
            ..
        } = inst
        else {
            continue;
        };
        let [Operand::Local(a0), Operand::Local(pos), val] = &ts[..] else {
            continue;
        };
        // Does this `TupleSet` write OUR tuple (its arg 0 coerces from `tup`)?
        match coerce_to_tup(*a0, def, count) {
            Some(src) if src == tup => {}
            _ => continue, // a different tuple's TupleSet — skip
        }
        // The slot index must equal this value's textual position (lowering emits
        // slot 0, 1, … in order). Validate it instead of trusting the order
        // silently: a future reorder would otherwise bind args to the wrong F
        // params with no verifier catch (every recovered value is `Tagged`).
        if raw_i64_const_value(*pos, def) != Some(vals.len() as i64) {
            return None;
        }
        // The arg-0 coerce must be single-use (we delete it).
        if reads[a0.index()] != 1 {
            return None;
        }
        let ct_idx = block.insts.iter().position(|i| i.dst() == Some(*a0))?;
        vals.push(val.clone());
        dead_builder.push(j);
        dead_builder.push(ct_idx);
    }
    let n = vals.len();

    // Non-escape proof: `tup` is read by exactly the `n` arg-0 coerces plus the one
    // `args_op` `Coerce` (src); `args_op` is read by only this `CallIndirect`.
    if reads[tup.index()] as usize != n + 1 || reads[args_op.index()] != 1 {
        return None;
    }

    // (5) Exact arity: the recovered args (plus the optional env) exactly fill F.
    let env_off = target.pass_env as usize;
    if target.f_params.len() != env_off + n {
        return None;
    }

    // Locate the remaining dead builder instructions in THIS block (single-def, so
    // `position` finds the unique in-block index): the `TupleNew` and the tuple
    // `Coerce` (the `TupleSet`s + their arg-0 coerces are already in `dead_builder`).
    let tn_idx = block.insts.iter().position(|i| i.dst() == Some(tup))?;
    let co_idx = block.insts.iter().position(|i| i.dst() == Some(args_op))?;
    let mut dead: HashSet<usize> = HashSet::new();
    dead.insert(tn_idx);
    dead.insert(co_idx);
    dead.extend(dead_builder);

    // Build the replacement: arg/env coercions, the direct `Call`, optional re-box.
    let mut fresh = |repr: Repr| -> LocalId {
        let id = LocalId::new(base + new_locals.len() as u32);
        new_locals.push(LocalDecl { repr });
        id
    };
    let mut replacement: Vec<MirInst> = Vec::new();
    let mut call_args: Vec<Operand> = Vec::new();

    // `env` arg (if the thunk forwards it): coerce the closure value to F's env
    // param (a bit-identity Closure -> Tagged Noop).
    if target.pass_env {
        let env_to = &target.f_params[0];
        let c_repr = f.local_repr(c).clone();
        if c_repr == *env_to {
            call_args.push(Operand::Local(c));
        } else {
            let e = fresh(env_to.clone());
            let inst = CoerceInst::new(e, Operand::Local(c), c_repr, env_to.clone())?;
            replacement.push(MirInst::Coerce(inst));
            call_args.push(Operand::Local(e));
        }
    }

    // Positional args: each recovered value is `Tagged` (verifier-guaranteed at
    // the `TupleSet`). Coerce to F's param repr — the CHECKED seam for a `Raw`
    // float/bool/int param (mirrors the thunk's `bind_arg_checked`), the unchecked
    // gradual retype otherwise (`Tagged` Noop, `Tagged -> Heap` trust).
    for (i, val) in vals.into_iter().enumerate() {
        let to = target.f_params[env_off + i].clone();
        if to == Repr::Tagged {
            call_args.push(val);
        } else {
            let a = fresh(to.clone());
            let inst = make_arg_coerce(a, val, &to)?;
            replacement.push(MirInst::Coerce(inst));
            call_args.push(Operand::Local(a));
        }
    }

    // Result repr: the `CallIndirect.dst` is `Tagged`. If `F.ret == Tagged`, use it
    // directly; otherwise call into a fresh `r: F.ret` and re-box it into `dst`
    // (the same box the thunk's return terminator emitted — no NEW heap alloc).
    let call_dst = match ci_dst {
        None => None,
        Some(d) => {
            if target.f_ret == Repr::Tagged {
                Some(*d)
            } else {
                let r = fresh(target.f_ret.clone());
                let rebox =
                    CoerceInst::new(*d, Operand::Local(r), target.f_ret.clone(), Repr::Tagged)?;
                // The `Call` writes `r`; the re-box follows it (pushed after).
                replacement.push(MirInst::Call {
                    dst: Some(r),
                    func: target.f_fid,
                    args: call_args,
                });
                replacement.push(MirInst::Coerce(rebox));
                return Some((dead, replacement));
            }
        }
    };
    replacement.push(MirInst::Call {
        dst: call_dst,
        func: target.f_fid,
        args: call_args,
    });
    Some((dead, replacement))
}

/// Coerce a recovered (`Tagged`) positional arg into F's param repr `to`. A `Raw`
/// float/bool/int param takes the CHECKED unbox (`rt_unbox_*`, TypeError-not-SEGV,
/// mirroring the thunk's `bind_arg_checked`); a heap/closure param the unchecked
/// gradual `Tagged -> Heap` retype.
fn make_arg_coerce(dst: LocalId, val: Operand, to: &Repr) -> Option<CoerceInst> {
    match to {
        Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I64) | Repr::Raw(RawKind::I8) => {
            CoerceInst::new_checked(dst, val, Repr::Tagged, to.clone())
        }
        _ => CoerceInst::new(dst, val, Repr::Tagged, to.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::Devirt;
    use crate::testutil::{interned, interned_file};
    use crate::OptimizationPass;
    use pyaot_mir::{
        verify, CoerceInst, Const, ContainerOp, LocalDecl, MirBlock, MirFunction, MirInst,
        MirProgram, MirTerminator, Operand, StrPool,
    };
    use pyaot_types::{generic_sig, HeapShape, Repr};
    use pyaot_utils::{BlockId, FuncId, LocalId};

    fn l(i: u32) -> LocalId {
        LocalId::new(i)
    }
    fn op(i: u32) -> Operand {
        Operand::Local(l(i))
    }

    fn func(
        name: &str,
        params: Vec<Repr>,
        ret: Repr,
        locals: Vec<Repr>,
        blocks: Vec<(Vec<MirInst>, MirTerminator)>,
    ) -> MirFunction {
        MirFunction {
            name: interned(name),
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
            verify(f, &p.funcs).expect("devirt output must verify");
        }
    }

    /// The specialized `F` (fid 2): `f(a, b)` returning its first arg. Two Tagged
    /// params, Tagged return.
    fn specialized_f() -> MirFunction {
        func(
            "f",
            vec![Repr::Tagged, Repr::Tagged],
            Repr::Tagged,
            vec![Repr::Tagged, Repr::Tagged],
            vec![(vec![], MirTerminator::Return(Some(op(0))))],
        )
    }

    /// A simple positional uniform thunk (fid 1) over `F` (fid 2): params
    /// `[env, __args__, __kwargs__]` (all Tagged = `generic_sig` + env), one direct
    /// `Call` to `F` whose first arg is NOT local 0 (so `pass_env == false`).
    fn simple_thunk() -> MirFunction {
        func(
            "f.<uniform>",
            vec![Repr::Tagged, Repr::Tagged, Repr::Tagged],
            Repr::Tagged,
            vec![Repr::Tagged, Repr::Tagged, Repr::Tagged, Repr::Tagged],
            vec![(
                vec![MirInst::Call {
                    dst: Some(l(3)),
                    func: FuncId::new(2),
                    // first arg is local 1 (not the env, local 0) → pass_env false.
                    args: vec![op(1), op(2)],
                }],
                MirTerminator::Return(Some(op(3))),
            )],
        )
    }

    /// `main` (fid 0): builds the `step`-style closure over the thunk and the
    /// real `lower_indirect_call` args-tuple + `CallIndirect` pattern over it.
    /// Mirrors `--emit-mir --opt-level none`: every `TupleSet`'s container arg is a
    /// per-slot bit-identity `Coerce(tup -> Tagged)` (PITFALLS A5), and the final
    /// `args_op` is another `Coerce(tup -> Tagged)`.
    ///
    /// locals: 0 closure, 1 size, 2 tup, 3 cT0, 4 pos0, 5 val0, 6 cT1, 7 pos1,
    /// 8 val1, 9 args_op, 10 kwargs, 11 ci-dst.
    fn main_with_indirect() -> MirFunction {
        let tup_repr = Repr::Heap(HeapShape::TupleVar(Box::new(Repr::Tagged)));
        let to_tagged = |dst: u32, src: u32| {
            MirInst::Coerce(
                CoerceInst::new(
                    l(dst),
                    op(src),
                    Repr::Heap(HeapShape::TupleVar(Box::new(Repr::Tagged))),
                    Repr::Tagged,
                )
                .expect("tuple -> tagged is legal"),
            )
        };
        func(
            "__main__",
            vec![],
            Repr::Tagged,
            vec![
                Repr::Closure(Box::new(generic_sig())), // 0
                Repr::Raw(pyaot_types::RawKind::I64),   // 1 size
                tup_repr,                               // 2 tup
                Repr::Tagged,                           // 3 cT0 (tup->Tagged)
                Repr::Raw(pyaot_types::RawKind::I64),   // 4 pos0
                Repr::Tagged,                           // 5 val0
                Repr::Tagged,                           // 6 cT1 (tup->Tagged)
                Repr::Raw(pyaot_types::RawKind::I64),   // 7 pos1
                Repr::Tagged,                           // 8 val1
                Repr::Tagged,                           // 9 args_op (tup->Tagged)
                Repr::Tagged,                           // 10 kwargs
                Repr::Tagged,                           // 11 ci-dst
            ],
            vec![(
                vec![
                    MirInst::MakeClosure {
                        dst: l(0),
                        func: FuncId::new(1),
                        captures: vec![],
                    },
                    MirInst::Const {
                        dst: l(1),
                        val: Const::Int(2),
                    },
                    MirInst::CallContainer {
                        dst: Some(l(2)),
                        op: ContainerOp::TupleNew,
                        args: vec![op(1)],
                    },
                    // slot 0
                    to_tagged(3, 2),
                    MirInst::Const {
                        dst: l(4),
                        val: Const::Int(0),
                    },
                    MirInst::Const {
                        dst: l(5),
                        val: Const::None,
                    },
                    MirInst::CallContainer {
                        dst: None,
                        op: ContainerOp::TupleSet,
                        args: vec![op(3), op(4), op(5)],
                    },
                    // slot 1
                    to_tagged(6, 2),
                    MirInst::Const {
                        dst: l(7),
                        val: Const::Int(1),
                    },
                    MirInst::Const {
                        dst: l(8),
                        val: Const::None,
                    },
                    MirInst::CallContainer {
                        dst: None,
                        op: ContainerOp::TupleSet,
                        args: vec![op(6), op(7), op(8)],
                    },
                    // args tuple + null kwargs
                    to_tagged(9, 2),
                    MirInst::Const {
                        dst: l(10),
                        val: Const::NullPtr,
                    },
                    MirInst::CallIndirect {
                        dst: Some(l(11)),
                        callee: op(0),
                        args: vec![op(9), op(10)],
                        sig: generic_sig(),
                    },
                ],
                MirTerminator::Return(Some(op(11))),
            )],
        )
    }

    #[test]
    fn devirts_monomorphic_indirect_call() {
        let mut p = program(vec![main_with_indirect(), simple_thunk(), specialized_f()]);
        Devirt.run(&mut p);

        let m = &p.funcs[0];
        let insts = &m.blocks[0].insts;
        // The CallIndirect is gone, replaced by a direct Call to F (fid 2).
        assert!(
            !insts
                .iter()
                .any(|i| matches!(i, MirInst::CallIndirect { .. })),
            "indirect call must be devirtualized"
        );
        let (args, dst) = insts
            .iter()
            .find_map(|i| match i {
                MirInst::Call { func, args, dst } if func.index() == 2 => Some((args, dst)),
                _ => None,
            })
            .expect("a direct Call to F must be emitted");
        // F's params are Tagged, so the recovered vals (locals 5, 8) pass through.
        let arg_locals: Vec<u32> = args
            .iter()
            .map(|Operand::Local(id)| id.index() as u32)
            .collect();
        assert_eq!(arg_locals, vec![5, 8], "args recovered in order");
        assert_eq!(*dst, Some(l(11)), "F.ret == Tagged → reuse the ci dst");
        // The args-tuple builder is gone (no TupleNew / TupleSet left).
        assert!(
            !insts.iter().any(|i| matches!(
                i,
                MirInst::CallContainer {
                    op: ContainerOp::TupleNew | ContainerOp::TupleSet,
                    ..
                }
            )),
            "the dead args-tuple builder must be removed"
        );
        // The closure is still passed (MakeClosure stays).
        assert!(insts
            .iter()
            .any(|i| matches!(i, MirInst::MakeClosure { .. })));
        verify_all(&p);
    }

    #[test]
    fn skips_multi_def_callee() {
        // A second def of the closure local defeats the single-def anchor.
        let mut m = main_with_indirect();
        m.blocks[0].insts.insert(
            1,
            MirInst::MakeClosure {
                dst: l(0),
                func: FuncId::new(1),
                captures: vec![],
            },
        );
        let mut p = program(vec![m, simple_thunk(), specialized_f()]);
        Devirt.run(&mut p);
        assert!(
            p.funcs[0].blocks[0]
                .insts
                .iter()
                .any(|i| matches!(i, MirInst::CallIndirect { .. })),
            "a multi-def closure must leave the indirect call untouched"
        );
        verify_all(&p);
    }

    #[test]
    fn skips_structurally_complex_thunk() {
        // A thunk carrying a `TupleFromIter` (varargs machinery) is not a simple
        // positional binder → the indirect call survives.
        let mut thunk = simple_thunk();
        thunk.locals.push(LocalDecl {
            repr: Repr::Heap(HeapShape::TupleVar(Box::new(Repr::Tagged))),
        });
        thunk.blocks[0].insts.insert(
            0,
            MirInst::CallContainer {
                dst: Some(l(4)),
                op: ContainerOp::TupleFromIter,
                args: vec![op(1)],
            },
        );
        let mut p = program(vec![main_with_indirect(), thunk, specialized_f()]);
        Devirt.run(&mut p);
        assert!(
            p.funcs[0].blocks[0]
                .insts
                .iter()
                .any(|i| matches!(i, MirInst::CallIndirect { .. })),
            "a structurally-complex thunk must leave the indirect call untouched"
        );
        verify_all(&p);
    }
}
