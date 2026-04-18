//! Cytron-Wegman-Zadeck SSA construction for MIR functions.
//!
//! Implements the classical three-phase algorithm ("Efficiently computing
//! static single assignment form and the control dependence graph", ACM
//! TOPLAS 1991):
//!
//! 1. **Collect defs** — for each `LocalId`, record the set of basic blocks
//!    that define it.
//! 2. **Insert φ-nodes** — for every local with ≥ 2 defining blocks, walk
//!    the iterated dominance frontier starting from those blocks and insert
//!    a `Phi { dest: <same LocalId>, sources: [] }` at the head of each
//!    frontier block. Sources are filled in during renaming.
//! 3. **Rename** — dominator-tree pre-order walk. Maintain a per-original-
//!    local stack of current SSA versions. Each def pushes a fresh
//!    `LocalId`; each use pops the current top. On leaving a block, pop the
//!    stack frames pushed for that block. For each successor's φ, append
//!    `(this_block, current_top_of_stack)` to the sources list.
//!
//! Phase 1 §1.3 of `ARCHITECTURE_REFACTOR.md`. Session S1.6a activates the
//! pass only on straight-line functions (no `Branch` terminators); S1.6b
//! extends to branching/looping; S1.6c to generators and closures.

use std::collections::HashMap;

use indexmap::{IndexMap, IndexSet};
use pyaot_utils::{BlockId, LocalId};
use smallvec::SmallVec;

use crate::dom_tree::terminator_successors;
use crate::{
    Function, Instruction, InstructionKind, Local, Operand, RaiseCause, RuntimeFunc, Terminator,
};

/// A RuntimeFunc is "void" if its codegen path never writes the `dest`
/// LocalId — either the runtime function returns nothing, or the call is
/// handled elsewhere (e.g. exception-raising dispatches as a terminator).
/// Callers use this to decide whether an `InstructionKind::RuntimeCall`
/// should be treated as defining its `dest` for SSA renaming purposes.
///
/// Visibility note: `pub(crate)` so `ssa_check` shares the exact same
/// predicate. A mismatch here would cause the checker to flag valid
/// MIR as invalid (e.g. void `rt_*_set` / `rt_string_builder_append`
/// that reuse a LocalId as a side-effectful placeholder).
pub(crate) fn runtime_call_is_void(func: &RuntimeFunc) -> bool {
    match func {
        // Descriptor-based: returns field is authoritative.
        RuntimeFunc::Call(def) => def.returns.is_none(),
        // Legacy variants known to leave `dest` untouched at codegen.
        RuntimeFunc::AssertFail
        | RuntimeFunc::PrintValue(_)
        | RuntimeFunc::ExcRegisterClassName
        | RuntimeFunc::ExcRaise
        | RuntimeFunc::ExcReraise
        | RuntimeFunc::ExcClear => true,
        // These return values or are dispatched via terminators whose own
        // codegen still writes through `dest`, so keep them as SSA defs.
        RuntimeFunc::MakeStr
        | RuntimeFunc::MakeBytes
        | RuntimeFunc::ExcSetjmp
        | RuntimeFunc::ExcGetType
        | RuntimeFunc::ExcHasException
        | RuntimeFunc::ExcGetCurrent
        | RuntimeFunc::ExcIsinstanceClass
        | RuntimeFunc::ExcRaiseCustom
        | RuntimeFunc::ExcInstanceStr => false,
    }
}

/// Transform `func` into SSA form in place using Cytron's algorithm.
///
/// On success, sets `func.is_ssa = true` and invalidates the cached
/// dominator tree (the CFG block graph is unchanged but the renamed
/// LocalIds invalidate any derived caches that reference locals).
///
/// Safe to call repeatedly: a no-op on already-SSA functions (detected via
/// `func.is_ssa`).
pub fn construct_ssa(func: &mut Function) {
    if func.is_ssa {
        return;
    }

    // Phase 0: prune blocks unreachable from the entry. Cytron's algorithm
    // only visits reachable blocks via the dominator tree, so any phi at a
    // reachable merge point would receive no source on an edge from an
    // unreachable predecessor. Remove such blocks so every CFG edge is
    // covered by the rename walk.
    prune_unreachable_blocks(func);

    // Phase 1: collect defining blocks and use-blocks for every local.
    let defs = collect_defs(func);
    let uses = collect_use_blocks(func);

    // Phase 2: compute iterated dominance frontier for each local and
    // insert φ-nodes at merge points. Pruned SSA: multi-def locals
    // always run IDF; single-def locals skip IDF if the def already
    // dominates every use (classical Cytron optimisation).
    insert_phis(func, &defs, &uses);

    // Phase 3: rename — dominator-tree pre-order walk assigning fresh
    // LocalIds to every def and propagating the current SSA version to
    // every use.
    rename(func);

    func.is_ssa = true;
    func.invalidate_dom_tree();
}

// ============================================================================
// Phase 0: dead-block removal
// ============================================================================

fn prune_unreachable_blocks(func: &mut Function) {
    use std::collections::VecDeque;

    let mut reachable: indexmap::IndexSet<BlockId> = indexmap::IndexSet::new();
    let mut queue: VecDeque<BlockId> = VecDeque::new();
    reachable.insert(func.entry_block);
    queue.push_back(func.entry_block);
    while let Some(bid) = queue.pop_front() {
        let Some(block) = func.blocks.get(&bid) else {
            continue;
        };
        for succ in terminator_successors(&block.terminator) {
            if reachable.insert(succ) {
                queue.push_back(succ);
            }
        }
    }
    if reachable.len() == func.blocks.len() {
        return;
    }
    func.blocks.retain(|id, _| reachable.contains(id));
    // Dropped blocks invalidate any cached dom tree.
    func.invalidate_dom_tree();
}

// ============================================================================
// Phase 1: def collection
// ============================================================================

fn collect_defs(func: &Function) -> HashMap<LocalId, IndexSet<BlockId>> {
    let mut defs: HashMap<LocalId, IndexSet<BlockId>> = HashMap::new();

    // Parameters are defined at the entry block.
    for p in &func.params {
        defs.entry(p.id).or_default().insert(func.entry_block);
    }

    for (&bid, block) in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = instruction_def(&inst.kind) {
                defs.entry(d).or_default().insert(bid);
            }
        }
    }
    defs
}

/// Collect, per local, the set of blocks where it is USED (by any
/// instruction or terminator, including as a Phi source). Used by
/// `insert_phis` for pruned SSA: single-def locals skip φ-insertion
/// only when every use block is dominated by the defining block.
fn collect_use_blocks(func: &Function) -> HashMap<LocalId, IndexSet<BlockId>> {
    let mut uses: HashMap<LocalId, IndexSet<BlockId>> = HashMap::new();
    for (&bid, block) in &func.blocks {
        for inst in &block.instructions {
            let mut kind_uses: IndexSet<LocalId> = IndexSet::new();
            collect_kind_uses(&inst.kind, &mut kind_uses);
            for id in kind_uses {
                uses.entry(id).or_default().insert(bid);
            }
        }
        let mut term_uses: IndexSet<LocalId> = IndexSet::new();
        collect_terminator_uses(&block.terminator, &mut term_uses);
        for id in term_uses {
            uses.entry(id).or_default().insert(bid);
        }
    }
    uses
}

fn instruction_def(kind: &InstructionKind) -> Option<LocalId> {
    use InstructionKind::*;
    match kind {
        // RuntimeCalls whose runtime function returns nothing do NOT produce
        // a new SSA value — the `dest` slot is a placeholder the codegen
        // leaves untouched (see the "Void function" branch in
        // `codegen::runtime_calls::compile_runtime_func_def`). MIR often
        // reuses an existing LocalId as this placeholder (e.g. `TupleSet`
        // stores into the same tuple local it mutates), so renaming that
        // LocalId here would shadow the live value and subsequent uses
        // would pick up an uninitialised Cranelift slot. Treating these as
        // non-defining keeps the tuple/list/dict mutation chain intact.
        RuntimeCall { dest, func, .. } => {
            if runtime_call_is_void(func) {
                None
            } else {
                Some(*dest)
            }
        }
        Const { dest, .. }
        | BinOp { dest, .. }
        | UnOp { dest, .. }
        | Call { dest, .. }
        | CallDirect { dest, .. }
        | CallNamed { dest, .. }
        | CallVirtual { dest, .. }
        | CallVirtualNamed { dest, .. }
        | FuncAddr { dest, .. }
        | BuiltinAddr { dest, .. }
        | Copy { dest, .. }
        | GcAlloc { dest, .. }
        | FloatToInt { dest, .. }
        | BoolToInt { dest, .. }
        | IntToFloat { dest, .. }
        | FloatBits { dest, .. }
        | IntBitsToFloat { dest, .. }
        | FloatAbs { dest, .. }
        | ExcGetType { dest }
        | ExcHasException { dest }
        | ExcGetCurrent { dest }
        | ExcCheckType { dest, .. }
        | ExcCheckClass { dest, .. }
        | Phi { dest, .. }
        | Refine { dest, .. } => Some(*dest),
        // `GcPush.frame` / `ExcPushFrame.frame_local` are Cranelift-level
        // synthesized definitions: the codegen computes the frame address
        // from a stack slot and stores it via `def_var(frame, addr)`.
        // Classically no MIR instruction produces these values, yet they
        // must be treated as defs for SSA: otherwise the checker sees
        // `ExcPushFrame` / `GcPush` both as uses-without-defs (since no
        // def exists) AND as uses that must be dominated by a non-existent
        // def. Classify them as defining their respective frame local.
        GcPush { frame } => Some(*frame),
        ExcPushFrame { frame_local } => Some(*frame_local),
        GcPop | ExcPopFrame | ExcClear | ExcStartHandling | ExcEndHandling => None,
    }
}

// ============================================================================
// Phase 2: φ-insertion via iterated dominance frontier
// ============================================================================

fn insert_phis(
    func: &mut Function,
    defs: &HashMap<LocalId, IndexSet<BlockId>>,
    uses: &HashMap<LocalId, IndexSet<BlockId>>,
) {
    let dom = func.dom_tree().clone();

    let mut phis_by_block: HashMap<BlockId, Vec<LocalId>> = HashMap::new();

    for (local, def_blocks) in defs {
        if def_blocks.is_empty() {
            continue;
        }

        // Pruned SSA: restore the classical single-def φ-skip
        // optimisation, but only when the single def actually
        // dominates every use block. Multi-def locals always run the
        // IDF walk. The dominance check catches our match-statement
        // lowering case (element-extraction in a pattern-check block
        // whose CFG successor merges before the body) — there the
        // single def block does NOT dominate the use block, so we
        // still run IDF and a φ is placed at the merge point.
        //
        // S1.6e originally relaxed this to "run IDF for every local"
        // after discovering the non-dominating case, which grew
        // `construct_ssa`'s cost from O(multi-def-locals) to
        // O(all-locals) and cost 50–85% of end_to_end compile time.
        // Pruned SSA restores the O(multi-def) cost profile while
        // preserving invariance.
        if def_blocks.len() == 1 {
            let def_block = *def_blocks.iter().next().unwrap();
            let all_dominated = uses
                .get(local)
                .is_none_or(|use_blocks| use_blocks.iter().all(|ub| dom.dominates(def_block, *ub)));
            if all_dominated {
                continue;
            }
        }

        // Iterated dominance frontier: start worklist with defining blocks;
        // for each popped block, for every block in its DF, if we haven't
        // already placed a φ for this local there, do so and push onto the
        // worklist.
        let mut has_phi: IndexSet<BlockId> = IndexSet::new();
        let mut worklist: Vec<BlockId> = def_blocks.iter().copied().collect();
        while let Some(b) = worklist.pop() {
            for df_block in dom.dominance_frontier(b) {
                if has_phi.insert(df_block) {
                    phis_by_block.entry(df_block).or_default().push(*local);
                    // Each φ acts as an additional "definition" of the local
                    // in df_block, so we may need to place further φs on its
                    // own dominance frontier.
                    if !def_blocks.contains(&df_block) {
                        worklist.push(df_block);
                    }
                }
            }
        }
    }

    // Materialise the recorded φ-nodes at the head of each block.
    for (block_id, locals) in phis_by_block {
        let block = func
            .blocks
            .get_mut(&block_id)
            .expect("phi target block exists");
        // Insert in reverse so prepends preserve collection order.
        let mut new_phis: Vec<Instruction> = locals
            .into_iter()
            .map(|l| Instruction {
                kind: InstructionKind::Phi {
                    dest: l,
                    sources: Vec::new(),
                },
                span: None,
            })
            .collect();
        new_phis.extend(std::mem::take(&mut block.instructions));
        block.instructions = new_phis;
    }
}

// ============================================================================
// Phase 3: rename
// ============================================================================

struct Renamer {
    /// Original → current stack of SSA versions.
    stacks: HashMap<LocalId, Vec<LocalId>>,
    /// Counter for allocating fresh LocalIds.
    next_local: u32,
    /// New locals map being built. Keyed by fresh LocalId.
    new_locals: IndexMap<LocalId, Local>,
    /// Snapshot of the original `Local` metadata. Cloned because `rename_block`
    /// needs &mut access to the function while still looking up types/names
    /// by original LocalId.
    original_locals: IndexMap<LocalId, Local>,
    /// Dominator-tree children (inverse of idom).
    dom_children: HashMap<BlockId, Vec<BlockId>>,
    /// Successors by block id.
    successors: HashMap<BlockId, SmallVec<[BlockId; 2]>>,
    /// For each block, the ordered list of ORIGINAL LocalIds of its
    /// leading φ-nodes. Captured before renaming begins because once a
    /// block is visited its φ `dest` fields get rewritten to fresh ids,
    /// which destroys the information needed to fill back-edge phi
    /// sources for that same block from a later-visited predecessor.
    phi_originals: HashMap<BlockId, Vec<LocalId>>,
}

fn rename(func: &mut Function) {
    // Allocate fresh-LocalId counter starting one past the current max.
    let next = func.locals.keys().map(|id| id.0 + 1).max().unwrap_or(0);

    // Compute dom-tree children from the tree's idom map.
    let dom = func.dom_tree().clone();
    let mut dom_children: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for bid in func.blocks.keys().copied() {
        if let Some(parent) = dom.immediate_dominator(bid) {
            dom_children.entry(parent).or_default().push(bid);
        }
    }

    // Successor table for the φ-source fill-in step.
    let mut successors: HashMap<BlockId, SmallVec<[BlockId; 2]>> = HashMap::new();
    for (&bid, block) in &func.blocks {
        successors.insert(bid, terminator_successors(&block.terminator));
    }

    // Snapshot every block's leading-Phi originals before any renaming runs.
    // Needed for correct back-edge phi-source fill-in: once a block has
    // been visited, its φ dests are rewritten to fresh ids.
    let mut phi_originals: HashMap<BlockId, Vec<LocalId>> = HashMap::new();
    for (&bid, block) in &func.blocks {
        let mut origs = Vec::new();
        for inst in &block.instructions {
            match &inst.kind {
                InstructionKind::Phi { dest, .. } => origs.push(*dest),
                _ => break,
            }
        }
        if !origs.is_empty() {
            phi_originals.insert(bid, origs);
        }
    }

    let mut renamer = Renamer {
        stacks: HashMap::new(),
        next_local: next,
        new_locals: IndexMap::new(),
        original_locals: func.locals.clone(),
        dom_children,
        successors,
        phi_originals,
    };

    // Seed parameter versions at entry: each parameter is already a unique
    // SSA value at function entry, so it keeps its original LocalId and
    // appears on its stack as the "incoming" version.
    for p in &func.params {
        renamer
            .new_locals
            .insert(p.id, renamer.original_locals[&p.id].clone());
        renamer.stacks.entry(p.id).or_default().push(p.id);
    }
    // Track which LocalIds are parameters — we keep their original IDs and
    // skip allocating fresh ones for them at entry.
    let param_ids: IndexSet<LocalId> = func.params.iter().map(|p| p.id).collect();
    renamer.next_local = renamer
        .next_local
        .max(param_ids.iter().map(|id| id.0 + 1).max().unwrap_or(0));

    // Recursive dominator-tree walk. `renamer` is &mut so we need to thread
    // per-block work through without holding long-lived borrows on func.
    rename_block(func, &mut renamer, func.entry_block, &param_ids);

    // Any LocalId referenced post-rename but missing from new_locals is a
    // use-without-dominating-def — in non-SSA MIR these were implicitly
    // zero-initialised by Cranelift's `declare_var`. Preserve the entry so
    // codegen's var_map still contains it; the SSA checker flags it as a
    // `UseWithoutDef` violation only if `is_ssa=true` and the checker is
    // run on the function.
    let referenced = collect_referenced_locals(func);
    for id in referenced {
        if !renamer.new_locals.contains_key(&id) {
            if let Some(local) = renamer.original_locals.get(&id).cloned() {
                renamer.new_locals.insert(id, local);
            }
        }
    }

    func.locals = renamer.new_locals;
}

fn collect_referenced_locals(func: &Function) -> IndexSet<LocalId> {
    let mut out: IndexSet<LocalId> = IndexSet::new();
    for block in func.blocks.values() {
        for inst in &block.instructions {
            if let Some(d) = instruction_def(&inst.kind) {
                out.insert(d);
            }
            collect_kind_uses(&inst.kind, &mut out);
        }
        collect_terminator_uses(&block.terminator, &mut out);
    }
    out
}

fn collect_kind_uses(kind: &InstructionKind, out: &mut IndexSet<LocalId>) {
    use InstructionKind::*;
    let push = |op: &Operand, out: &mut IndexSet<LocalId>| {
        if let Operand::Local(id) = op {
            out.insert(*id);
        }
    };
    match kind {
        Const { .. }
        | FuncAddr { .. }
        | BuiltinAddr { .. }
        | GcAlloc { .. }
        | GcPop
        | ExcPopFrame
        | ExcClear
        | ExcGetType { .. }
        | ExcHasException { .. }
        | ExcGetCurrent { .. }
        | ExcCheckType { .. }
        | ExcCheckClass { .. }
        | ExcStartHandling
        | ExcEndHandling => {}
        BinOp { left, right, .. } => {
            push(left, out);
            push(right, out);
        }
        UnOp { operand, .. } => push(operand, out),
        Copy { src, .. }
        | FloatToInt { src, .. }
        | BoolToInt { src, .. }
        | IntToFloat { src, .. }
        | FloatBits { src, .. }
        | IntBitsToFloat { src, .. }
        | FloatAbs { src, .. } => push(src, out),
        Call { func, args, .. } => {
            push(func, out);
            for a in args {
                push(a, out);
            }
        }
        CallDirect { args, .. } | CallNamed { args, .. } | RuntimeCall { args, .. } => {
            for a in args {
                push(a, out);
            }
        }
        CallVirtual { obj, args, .. } | CallVirtualNamed { obj, args, .. } => {
            push(obj, out);
            for a in args {
                push(a, out);
            }
        }
        // GcPush / ExcPushFrame define their frame_local (Cranelift-
        // synthesized def_var); see classification in `instruction_def`.
        // They must NOT appear in collect_kind_uses or the def would
        // "depend on itself" when computing reachable locals.
        GcPush { .. } | ExcPushFrame { .. } => {}
        Phi { sources, .. } => {
            for (_, op) in sources {
                push(op, out);
            }
        }
        Refine { src, .. } => push(src, out),
    }
}

fn collect_terminator_uses(term: &Terminator, out: &mut IndexSet<LocalId>) {
    let push = |op: &Operand, out: &mut IndexSet<LocalId>| {
        if let Operand::Local(id) = op {
            out.insert(*id);
        }
    };
    match term {
        Terminator::Return(Some(op)) => push(op, out),
        Terminator::Return(None)
        | Terminator::Goto(_)
        | Terminator::Unreachable
        | Terminator::Reraise => {}
        Terminator::Branch { cond, .. } => push(cond, out),
        Terminator::TrySetjmp { frame_local, .. } => {
            out.insert(*frame_local);
        }
        Terminator::Raise { message, cause, .. } => {
            if let Some(op) = message {
                push(op, out);
            }
            if let Some(RaiseCause {
                message: Some(op), ..
            }) = cause
            {
                push(op, out);
            }
        }
        Terminator::RaiseCustom {
            message, instance, ..
        } => {
            if let Some(op) = message {
                push(op, out);
            }
            if let Some(op) = instance {
                push(op, out);
            }
        }
        Terminator::RaiseInstance { instance } => push(instance, out),
    }
}

fn rename_block(
    func: &mut Function,
    ctx: &mut Renamer,
    bid: BlockId,
    param_ids: &IndexSet<LocalId>,
) {
    // Track which original locals we pushed onto stacks in this block so
    // we can pop them when leaving.
    let mut pushed: Vec<LocalId> = Vec::new();

    {
        let block = func
            .blocks
            .get_mut(&bid)
            .expect("block exists in rename walk");

        // φ-nodes come first: each defines a new SSA version of its original
        // local. Sources remain empty here; they are filled by the caller
        // predecessor below.
        for inst in &mut block.instructions {
            if let InstructionKind::Phi { dest, .. } = &mut inst.kind {
                let original = *dest;
                let fresh = alloc_fresh(ctx, original, param_ids, bid);
                *dest = fresh;
                ctx.stacks.entry(original).or_default().push(fresh);
                pushed.push(original);
            } else {
                break;
            }
        }

        // Remaining instructions: rename uses first (current top of stack),
        // then def (push a new fresh version).
        for inst in &mut block.instructions {
            if matches!(inst.kind, InstructionKind::Phi { .. }) {
                continue;
            }
            rename_uses(&mut inst.kind, &ctx.stacks);
            if let Some(original) = instruction_def(&inst.kind) {
                let fresh = alloc_fresh(ctx, original, param_ids, bid);
                rewrite_def(&mut inst.kind, fresh);
                ctx.stacks.entry(original).or_default().push(fresh);
                pushed.push(original);
            }
        }

        rename_terminator_uses(&mut block.terminator, &ctx.stacks);
    }

    // Fill in φ-sources for each successor. The successor may or may not
    // have been visited already (back-edges visit predecessors after the
    // successor/header in dom-tree pre-order). Use the pre-captured
    // `phi_originals` side-table to recover each φ's original LocalId
    // regardless of whether `phi.dest` has been rewritten yet.
    let succs = ctx.successors.get(&bid).cloned().unwrap_or_default();
    for succ in succs {
        let orig_list = match ctx.phi_originals.get(&succ) {
            Some(list) => list.clone(),
            None => continue,
        };
        let succ_block = func.blocks.get_mut(&succ).expect("successor block exists");
        for (idx, original) in orig_list.into_iter().enumerate() {
            let InstructionKind::Phi { sources, .. } = &mut succ_block.instructions[idx].kind
            else {
                debug_assert!(false, "phi_originals mapped a non-Phi slot");
                continue;
            };
            // φ-source for the edge `bid → succ`: use the current top of
            // the rename stack for `original`. If the stack is empty,
            // the variable has no defining version reaching this edge —
            // typical for variables defined inside a nested loop whose
            // outer-header Phi receives "undefined" on the entry edge.
            //
            // Pre-SSA Cranelift `declare_var` zero-initialized every
            // local on entry, so the pre-SSA semantics read zero. Emit
            // a typed zero-default constant here to preserve those
            // semantics AND keep the SSA invariant: the prior fallback
            // (reuse `original` as the source) produced a self-
            // referential `phi(phi.dest, ...)` that breaks the dominance
            // rule and was flagged by `ssa_check` as `UseNotDominated`.
            let cur_operand = match ctx.stacks.get(&original).and_then(|s| s.last()).copied() {
                Some(cur) => Operand::Local(cur),
                None => default_undef_operand(ctx, original),
            };
            sources.push((bid, cur_operand));
        }
    }

    // Recurse into dom-tree children.
    let children = ctx.dom_children.get(&bid).cloned().unwrap_or_default();
    for child in children {
        rename_block(func, ctx, child, param_ids);
    }

    // Pop stacks on leaving.
    for original in pushed.into_iter().rev() {
        if let Some(stack) = ctx.stacks.get_mut(&original) {
            stack.pop();
        }
    }
    let _ = func; // silence unused-warning in debug builds
}

/// Default "undefined" operand to use as a φ-source when no definition
/// of `original` reaches the predecessor edge. Matches pre-SSA Cranelift
/// `declare_var` zero-initialization: numeric locals get `0`, booleans
/// get `false`, `None` gets None, heap locals get a null pointer (which
/// Cranelift encodes as `Int(0)` at the Value level). The concrete value
/// is never semantically consumed on a well-formed execution path — the
/// φ dest is dead on this edge — but it must be well-typed for
/// Cranelift's block-param verifier.
fn default_undef_operand(ctx: &Renamer, original: LocalId) -> Operand {
    use crate::Constant;
    use pyaot_types::Type;
    let ty = ctx
        .original_locals
        .get(&original)
        .map(|l| &l.ty)
        .unwrap_or(&Type::Int);
    let c = match ty {
        Type::Float => Constant::Float(0.0),
        Type::Bool => Constant::Bool(false),
        Type::None => Constant::None,
        // Int and every heap/pointer type: Cranelift represents both
        // as i64 at the ABI level, and zero is a valid null pointer.
        _ => Constant::Int(0),
    };
    Operand::Constant(c)
}

fn alloc_fresh(
    ctx: &mut Renamer,
    original: LocalId,
    param_ids: &IndexSet<LocalId>,
    _block: BlockId,
) -> LocalId {
    // First def of a non-parameter: reuse the original LocalId so that
    // single-def locals don't change names. Parameter LocalIds are already
    // seeded in `new_locals` at entry.
    if !param_ids.contains(&original)
        && !ctx.new_locals.contains_key(&original)
        && ctx.stacks.get(&original).is_none_or(|s| s.is_empty())
    {
        ctx.new_locals
            .insert(original, ctx.original_locals[&original].clone());
        return original;
    }
    // Otherwise, allocate a fresh LocalId.
    let id = LocalId::from(ctx.next_local);
    ctx.next_local += 1;
    let mut new_local = ctx.original_locals[&original].clone();
    new_local.id = id;
    ctx.new_locals.insert(id, new_local);
    id
}

// ============================================================================
// Use/def rewriting helpers
// ============================================================================

fn rename_uses(kind: &mut InstructionKind, stacks: &HashMap<LocalId, Vec<LocalId>>) {
    use InstructionKind::*;
    match kind {
        Const { .. }
        | FuncAddr { .. }
        | BuiltinAddr { .. }
        | GcAlloc { .. }
        | GcPop
        | ExcPopFrame
        | ExcClear
        | ExcGetType { .. }
        | ExcHasException { .. }
        | ExcGetCurrent { .. }
        | ExcCheckType { .. }
        | ExcCheckClass { .. }
        | ExcStartHandling
        | ExcEndHandling => {}
        BinOp { left, right, .. } => {
            subst_operand(left, stacks);
            subst_operand(right, stacks);
        }
        UnOp { operand, .. } => subst_operand(operand, stacks),
        Copy { src, .. }
        | FloatToInt { src, .. }
        | BoolToInt { src, .. }
        | IntToFloat { src, .. }
        | FloatBits { src, .. }
        | IntBitsToFloat { src, .. }
        | FloatAbs { src, .. } => subst_operand(src, stacks),
        Call { func, args, .. } => {
            subst_operand(func, stacks);
            for a in args {
                subst_operand(a, stacks);
            }
        }
        CallDirect { args, .. } | CallNamed { args, .. } | RuntimeCall { args, .. } => {
            for a in args {
                subst_operand(a, stacks);
            }
        }
        CallVirtual { obj, args, .. } | CallVirtualNamed { obj, args, .. } => {
            subst_operand(obj, stacks);
            for a in args {
                subst_operand(a, stacks);
            }
        }
        // Classified as defs, not uses — no rename needed here.
        GcPush { .. } | ExcPushFrame { .. } => {}
        Phi { .. } => {
            // φ uses are filled in by the predecessor's `rename_block` via
            // `fill_phi_sources`, not here.
        }
        Refine { src, .. } => subst_operand(src, stacks),
    }
}

fn rewrite_def(kind: &mut InstructionKind, fresh: LocalId) {
    use InstructionKind::*;
    match kind {
        Const { dest, .. }
        | BinOp { dest, .. }
        | UnOp { dest, .. }
        | Call { dest, .. }
        | CallDirect { dest, .. }
        | CallNamed { dest, .. }
        | CallVirtual { dest, .. }
        | CallVirtualNamed { dest, .. }
        | FuncAddr { dest, .. }
        | BuiltinAddr { dest, .. }
        | RuntimeCall { dest, .. }
        | Copy { dest, .. }
        | GcAlloc { dest, .. }
        | FloatToInt { dest, .. }
        | BoolToInt { dest, .. }
        | IntToFloat { dest, .. }
        | FloatBits { dest, .. }
        | IntBitsToFloat { dest, .. }
        | FloatAbs { dest, .. }
        | ExcGetType { dest }
        | ExcHasException { dest }
        | ExcGetCurrent { dest }
        | ExcCheckType { dest, .. }
        | ExcCheckClass { dest, .. }
        | Phi { dest, .. }
        | Refine { dest, .. } => {
            *dest = fresh;
        }
        GcPush { frame } => {
            *frame = fresh;
        }
        ExcPushFrame { frame_local } => {
            *frame_local = fresh;
        }
        GcPop | ExcPopFrame | ExcClear | ExcStartHandling | ExcEndHandling => {
            debug_assert!(false, "rewrite_def called on a defless instruction");
        }
    }
}

fn rename_terminator_uses(term: &mut Terminator, stacks: &HashMap<LocalId, Vec<LocalId>>) {
    match term {
        Terminator::Return(Some(op)) => subst_operand(op, stacks),
        Terminator::Return(None)
        | Terminator::Goto(_)
        | Terminator::Unreachable
        | Terminator::Reraise => {}
        Terminator::Branch { cond, .. } => subst_operand(cond, stacks),
        Terminator::TrySetjmp { frame_local, .. } => subst_local(frame_local, stacks),
        Terminator::Raise { message, cause, .. } => {
            if let Some(op) = message {
                subst_operand(op, stacks);
            }
            if let Some(RaiseCause {
                message: Some(op), ..
            }) = cause
            {
                subst_operand(op, stacks);
            }
        }
        Terminator::RaiseCustom {
            message, instance, ..
        } => {
            if let Some(op) = message {
                subst_operand(op, stacks);
            }
            if let Some(op) = instance {
                subst_operand(op, stacks);
            }
        }
        Terminator::RaiseInstance { instance } => subst_operand(instance, stacks),
    }
}

fn subst_operand(op: &mut Operand, stacks: &HashMap<LocalId, Vec<LocalId>>) {
    if let Operand::Local(id) = op {
        subst_local(id, stacks);
    }
}

fn subst_local(id: &mut LocalId, stacks: &HashMap<LocalId, Vec<LocalId>>) {
    if let Some(current) = stacks.get(id).and_then(|s| s.last()).copied() {
        *id = current;
    }
    // If no stack entry exists the local is undefined at this program point.
    // Leave the id unchanged; the SSA checker will surface this as a
    // UseWithoutDef violation if the caller activates it.
}

// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::BasicBlock;
    use pyaot_types::Type;
    use pyaot_utils::FuncId;

    fn mk_local(id: u32, ty: Type) -> Local {
        Local {
            id: LocalId::from(id),
            name: None,
            ty,
            is_gc_root: false,
        }
    }

    fn empty_func() -> Function {
        Function::new(
            FuncId::from(0u32),
            "test".to_string(),
            Vec::new(),
            Type::Int,
            None,
        )
    }

    fn add_block(func: &mut Function, id: u32, t: Terminator) -> BlockId {
        let bid = BlockId::from(id);
        func.blocks.insert(
            bid,
            BasicBlock {
                id: bid,
                instructions: Vec::new(),
                terminator: t,
            },
        );
        bid
    }

    #[test]
    fn straight_line_single_block_versions_single_def() {
        // bb0: l1 = 1; return l1
        let mut func = empty_func();
        let l1 = LocalId::from(1u32);
        func.locals.insert(l1, mk_local(1, Type::Int));
        let bb0 = BlockId::from(0u32);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l1,
                value: crate::Constant::Int(5),
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(l1)));

        construct_ssa(&mut func);

        assert!(func.is_ssa);
        // Single def → LocalId unchanged.
        assert_eq!(func.locals.len(), 1);
        assert!(func.locals.contains_key(&l1));
        // Return still uses l1.
        match &func.block_mut(bb0).terminator {
            Terminator::Return(Some(Operand::Local(id))) => assert_eq!(*id, l1),
            _ => panic!("unexpected terminator"),
        }
    }

    #[test]
    fn straight_line_multi_def_gets_fresh_ids() {
        // bb0: l1 = 1; l1 = 2; return l1
        let mut func = empty_func();
        let l1 = LocalId::from(1u32);
        func.locals.insert(l1, mk_local(1, Type::Int));
        let bb0 = BlockId::from(0u32);
        let block = func.block_mut(bb0);
        block.instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l1,
                value: crate::Constant::Int(1),
            },
            span: None,
        });
        block.instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l1,
                value: crate::Constant::Int(2),
            },
            span: None,
        });
        block.terminator = Terminator::Return(Some(Operand::Local(l1)));

        construct_ssa(&mut func);
        assert!(func.is_ssa);

        // The two defs should have distinct LocalIds after renaming.
        let block = &func.blocks[&bb0];
        let d1 = instruction_def(&block.instructions[0].kind).unwrap();
        let d2 = instruction_def(&block.instructions[1].kind).unwrap();
        assert_ne!(d1, d2);
        // Return should read the second def.
        match &block.terminator {
            Terminator::Return(Some(Operand::Local(id))) => assert_eq!(*id, d2),
            _ => panic!("unexpected terminator"),
        }
        // Checker must pass.
        assert!(crate::ssa_check::check(&func).is_ok());
    }

    #[test]
    fn diamond_merge_gets_phi() {
        // bb0: branch -> bb1 or bb2
        // bb1: l = 10; goto bb3
        // bb2: l = 20; goto bb3
        // bb3: return l
        let mut func = empty_func();
        let l = LocalId::from(1u32);
        let c = LocalId::from(2u32);
        func.locals.insert(l, mk_local(1, Type::Int));
        func.locals.insert(c, mk_local(2, Type::Bool));

        let bb0 = BlockId::from(0u32);
        let bb1 = add_block(&mut func, 1, Terminator::Unreachable);
        let bb2 = add_block(&mut func, 2, Terminator::Unreachable);
        let bb3 = add_block(&mut func, 3, Terminator::Unreachable);

        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: c,
                value: crate::Constant::Bool(true),
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Branch {
            cond: Operand::Local(c),
            then_block: bb1,
            else_block: bb2,
        };

        func.block_mut(bb1).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l,
                value: crate::Constant::Int(10),
            },
            span: None,
        });
        func.block_mut(bb1).terminator = Terminator::Goto(bb3);

        func.block_mut(bb2).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l,
                value: crate::Constant::Int(20),
            },
            span: None,
        });
        func.block_mut(bb2).terminator = Terminator::Goto(bb3);

        func.block_mut(bb3).terminator = Terminator::Return(Some(Operand::Local(l)));

        construct_ssa(&mut func);
        assert!(func.is_ssa);

        // bb3 must start with a Phi for l, with two sources.
        let bb3_block = &func.blocks[&bb3];
        let first = &bb3_block.instructions[0];
        match &first.kind {
            InstructionKind::Phi { sources, .. } => {
                assert_eq!(sources.len(), 2);
            }
            other => panic!("expected Phi, got {:?}", other),
        }
        assert!(crate::ssa_check::check(&func).is_ok());
    }

    /// While-loop back-edge: phi at the header must receive a source from
    /// both the entry (pre-loop) edge and the body (back-edge). This tests
    /// the classic SSA gotcha where the header's phi has already been
    /// renamed when the body processes its Goto successor.
    ///
    ///   bb0: l = 0;  goto bb1
    ///   bb1: phi(l); branch bb2 bb3
    ///   bb2: l = l + 1; goto bb1   (back-edge)
    ///   bb3: return l
    #[test]
    fn while_loop_phi_gets_both_entry_and_back_edge_sources() {
        let mut func = empty_func();
        let l = LocalId::from(1u32);
        let c = LocalId::from(2u32);
        let tmp = LocalId::from(3u32);
        func.locals.insert(l, mk_local(1, Type::Int));
        func.locals.insert(c, mk_local(2, Type::Bool));
        func.locals.insert(tmp, mk_local(3, Type::Int));

        let bb0 = BlockId::from(0u32);
        let bb1 = add_block(&mut func, 1, Terminator::Unreachable);
        let bb2 = add_block(&mut func, 2, Terminator::Unreachable);
        let bb3 = add_block(&mut func, 3, Terminator::Unreachable);

        // bb0: l = 0
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l,
                value: crate::Constant::Int(0),
            },
            span: None,
        });
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: c,
                value: crate::Constant::Bool(true),
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Goto(bb1);

        // bb1: branch on c → bb2 / bb3
        func.block_mut(bb1).terminator = Terminator::Branch {
            cond: Operand::Local(c),
            then_block: bb2,
            else_block: bb3,
        };

        // bb2: l = l + 1; goto bb1 (back-edge)
        func.block_mut(bb2).instructions.push(Instruction {
            kind: InstructionKind::BinOp {
                dest: tmp,
                op: crate::BinOp::Add,
                left: Operand::Local(l),
                right: Operand::Constant(crate::Constant::Int(1)),
            },
            span: None,
        });
        func.block_mut(bb2).instructions.push(Instruction {
            kind: InstructionKind::Copy {
                dest: l,
                src: Operand::Local(tmp),
            },
            span: None,
        });
        func.block_mut(bb2).terminator = Terminator::Goto(bb1);

        // bb3: return l
        func.block_mut(bb3).terminator = Terminator::Return(Some(Operand::Local(l)));

        construct_ssa(&mut func);
        assert!(func.is_ssa);

        // bb1 should start with a Phi that has exactly two sources —
        // one from bb0 (entry edge) and one from bb2 (back-edge). Only
        // `l` is multi-def (bb0 init + bb2 update), so after pruned-
        // SSA only `l` gets a Phi here; `c` and `tmp` are single-def
        // with dominated uses and skip IDF. Find the Phi whose
        // back-edge source flows from bb2's Copy chain.
        let bb1_block = &func.blocks[&bb1];
        let bb2_block = &func.blocks[&bb2];
        let copy_dest = bb2_block
            .instructions
            .iter()
            .find_map(|i| match &i.kind {
                InstructionKind::Copy { dest, .. } => Some(*dest),
                _ => None,
            })
            .expect("copy in bb2");

        // Find the Phi that receives `copy_dest` as its back-edge
        // source — that is the Phi for `l`.
        let l_phi = bb1_block
            .instructions
            .iter()
            .find_map(|i| match &i.kind {
                InstructionKind::Phi { sources, dest } => {
                    let has_copy_dest_src = sources.iter().any(|(pred, op)| {
                        *pred == bb2 && matches!(op, Operand::Local(id) if *id == copy_dest)
                    });
                    if has_copy_dest_src {
                        Some((sources.clone(), *dest))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .expect("phi for `l` must receive bb2's copy dest as back-edge source");

        let (sources, dest) = l_phi;
        assert_eq!(sources.len(), 2, "phi should have 2 sources");
        let pred_blocks: Vec<BlockId> = sources.iter().map(|(b, _)| *b).collect();
        assert!(pred_blocks.contains(&bb0));
        assert!(pred_blocks.contains(&bb2));
        let back_edge_src = sources
            .iter()
            .find(|(b, _)| *b == bb2)
            .map(|(_, op)| op.clone())
            .unwrap();
        assert_eq!(back_edge_src, Operand::Local(copy_dest));
        assert_ne!(
            back_edge_src,
            Operand::Local(dest),
            "back-edge phi source must not be the phi's own dest (self-loop bug)"
        );
        assert!(
            crate::ssa_check::check(&func).is_ok(),
            "SSA checker rejected the loop: {:?}",
            crate::ssa_check::check(&func)
        );
    }

    /// `Refine` is a def + use: renaming must give it a fresh dest, and
    /// subsequent uses of the original must pick up the refined version.
    /// Models the post-isinstance narrowing shape that S1.8 will emit.
    ///
    ///   bb0: l = 1; refine(l as Int); return l
    #[test]
    fn refine_participates_in_ssa_renaming() {
        let mut func = empty_func();
        let l = LocalId::from(1u32);
        func.locals.insert(l, mk_local(1, Type::Int));
        let bb0 = BlockId::from(0u32);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l,
                value: crate::Constant::Int(42),
            },
            span: None,
        });
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Refine {
                dest: l,
                src: Operand::Local(l),
                ty: Type::Int,
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(l)));

        construct_ssa(&mut func);
        assert!(func.is_ssa);

        // Const l → l0, Refine l → l1 (fresh), return uses l1.
        let block = &func.blocks[&bb0];
        let const_dest = match &block.instructions[0].kind {
            InstructionKind::Const { dest, .. } => *dest,
            other => panic!("expected Const, got {:?}", other),
        };
        let (refine_dest, refine_src) = match &block.instructions[1].kind {
            InstructionKind::Refine { dest, src, .. } => (*dest, src.clone()),
            other => panic!("expected Refine, got {:?}", other),
        };
        assert_ne!(
            const_dest, refine_dest,
            "Refine must define a fresh LocalId, not reuse the Const's dest"
        );
        assert_eq!(
            refine_src,
            Operand::Local(const_dest),
            "Refine src must be the previous def of l"
        );
        match &block.terminator {
            Terminator::Return(Some(Operand::Local(id))) => assert_eq!(*id, refine_dest),
            _ => panic!("unexpected terminator"),
        }
        assert!(crate::ssa_check::check(&func).is_ok());
    }

    /// S1.6c regression: a void `RuntimeCall` (runtime function with
    /// `returns: None`, e.g. `rt_string_builder_append`) must NOT be
    /// treated as a def. If it were, multiple side-effectful void
    /// calls that reuse the same LocalId as a placeholder (common in
    /// loop bodies) would look like SSA multi-defs and the checker
    /// would flag them. Both `ssa_construct::instruction_def` and
    /// `ssa_check::instruction_def` must agree via
    /// `runtime_call_is_void`.
    #[test]
    fn void_runtime_call_is_not_treated_as_def() {
        let mut func = empty_func();
        let placeholder = LocalId::from(1u32);
        let builder_local = LocalId::from(2u32);
        func.locals.insert(placeholder, mk_local(1, Type::Int));
        func.locals.insert(builder_local, mk_local(2, Type::Int));

        let bb0 = BlockId::from(0u32);
        // Define `builder_local` so it's a valid operand.
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: builder_local,
                value: crate::Constant::Int(0),
            },
            span: None,
        });
        // Two void RuntimeCalls in a row both using `placeholder` as
        // dest — the pre-S1.6c checker would flag this as multi-def.
        let append_def = &pyaot_core_defs::runtime_func_def::RT_STRING_BUILDER_APPEND;
        for _ in 0..2 {
            func.block_mut(bb0).instructions.push(Instruction {
                kind: InstructionKind::RuntimeCall {
                    dest: placeholder,
                    func: crate::RuntimeFunc::Call(append_def),
                    args: vec![Operand::Local(builder_local), Operand::Local(builder_local)],
                },
                span: None,
            });
        }
        func.block_mut(bb0).terminator = Terminator::Return(None);

        construct_ssa(&mut func);
        assert!(func.is_ssa);
        assert!(
            crate::ssa_check::check(&func).is_ok(),
            "void RuntimeCall with shared dest must not produce multi-def violations"
        );
    }

    /// Pruned-SSA regression: a single-def local whose def block does
    /// NOT dominate all use blocks must still get a φ at the iterated
    /// dominance frontier. Models the match-statement lowering pattern
    /// that motivated S1.6e — elements_bb (def) → skip_bb (merge) →
    /// body (use), where the path `entry → skip_bb → body` bypasses
    /// elements_bb.
    #[test]
    fn pruned_ssa_places_phi_when_single_def_does_not_dominate_use() {
        let mut func = empty_func();
        let cond = LocalId::from(1u32);
        let binding = LocalId::from(2u32);
        let unused = LocalId::from(3u32);
        func.locals.insert(cond, mk_local(1, Type::Bool));
        func.locals.insert(binding, mk_local(2, Type::Int));
        func.locals.insert(unused, mk_local(3, Type::Int));

        let bb_entry = BlockId::from(0u32);
        let bb_def = add_block(&mut func, 1, Terminator::Unreachable);
        let bb_merge = add_block(&mut func, 2, Terminator::Unreachable);
        let bb_use = add_block(&mut func, 3, Terminator::Unreachable);
        let bb_exit = add_block(&mut func, 4, Terminator::Unreachable);

        // entry: cond = true; branch bb_def / bb_merge
        func.block_mut(bb_entry).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: cond,
                value: crate::Constant::Bool(true),
            },
            span: None,
        });
        func.block_mut(bb_entry).terminator = Terminator::Branch {
            cond: Operand::Local(cond),
            then_block: bb_def,
            else_block: bb_merge,
        };

        // bb_def: binding = 42; goto bb_merge
        func.block_mut(bb_def).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: binding,
                value: crate::Constant::Int(42),
            },
            span: None,
        });
        func.block_mut(bb_def).terminator = Terminator::Goto(bb_merge);

        // bb_merge: branch bb_use / bb_exit (merge point; no defs here)
        func.block_mut(bb_merge).terminator = Terminator::Branch {
            cond: Operand::Local(cond),
            then_block: bb_use,
            else_block: bb_exit,
        };

        // bb_use: unused = binding + 0; return unused  (USE of binding)
        func.block_mut(bb_use).instructions.push(Instruction {
            kind: InstructionKind::BinOp {
                dest: unused,
                op: crate::BinOp::Add,
                left: Operand::Local(binding),
                right: Operand::Constant(crate::Constant::Int(0)),
            },
            span: None,
        });
        func.block_mut(bb_use).terminator = Terminator::Return(Some(Operand::Local(unused)));

        // bb_exit: return 0
        func.block_mut(bb_exit).terminator =
            Terminator::Return(Some(Operand::Constant(crate::Constant::Int(0))));

        construct_ssa(&mut func);
        assert!(func.is_ssa);

        // Pruned SSA must place a φ for `binding` at bb_merge (def in
        // bb_def doesn't dominate bb_use because entry → bb_merge
        // bypasses bb_def). A pre-pruned-SSA (S1.6e) "always place Phi"
        // rule would pass this test too; a pre-S1.6e "skip all
        // single-def" rule would fail it.
        let bb_merge_block = &func.blocks[&bb_merge];
        let has_phi_for_binding = bb_merge_block
            .instructions
            .iter()
            .any(|i| matches!(&i.kind, InstructionKind::Phi { .. }));
        assert!(
            has_phi_for_binding,
            "bb_merge must have a φ because bb_def doesn't dominate bb_use"
        );
        assert!(
            crate::ssa_check::check(&func).is_ok(),
            "pruned SSA must produce a checker-clean function"
        );
    }

    /// Pruned-SSA invariant: a single-def local whose def block DOES
    /// dominate every use block must NOT get a φ. Confirms the
    /// classical Cytron shortcut is still active for the common case.
    #[test]
    fn pruned_ssa_skips_phi_when_single_def_dominates_all_uses() {
        let mut func = empty_func();
        let x = LocalId::from(1u32);
        let y = LocalId::from(2u32);
        func.locals.insert(x, mk_local(1, Type::Int));
        func.locals.insert(y, mk_local(2, Type::Int));

        let bb_entry = BlockId::from(0u32);
        let bb_a = add_block(&mut func, 1, Terminator::Unreachable);
        let bb_b = add_block(&mut func, 2, Terminator::Unreachable);
        let bb_join = add_block(&mut func, 3, Terminator::Unreachable);

        // entry: x = 5; branch bb_a / bb_b. `x` is defined in entry,
        // which dominates every other block.
        func.block_mut(bb_entry).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: x,
                value: crate::Constant::Int(5),
            },
            span: None,
        });
        func.block_mut(bb_entry).terminator = Terminator::Branch {
            cond: Operand::Constant(crate::Constant::Bool(true)),
            then_block: bb_a,
            else_block: bb_b,
        };

        // bb_a, bb_b: both use x, neither defines x.
        for &bb in &[bb_a, bb_b] {
            func.block_mut(bb).instructions.push(Instruction {
                kind: InstructionKind::BinOp {
                    dest: y,
                    op: crate::BinOp::Add,
                    left: Operand::Local(x),
                    right: Operand::Constant(crate::Constant::Int(0)),
                },
                span: None,
            });
            func.block_mut(bb).terminator = Terminator::Goto(bb_join);
        }

        // bb_join also uses x.
        func.block_mut(bb_join).terminator = Terminator::Return(Some(Operand::Local(x)));

        construct_ssa(&mut func);
        assert!(func.is_ssa);

        // `x` has a single def in entry which dominates bb_a, bb_b,
        // bb_join. Pruned SSA must skip φ insertion for `x`. Since
        // `y` is multi-def (bb_a + bb_b) it still gets a φ at bb_join
        // — its φ presence is fine; we're just confirming that `x`
        // doesn't ALSO get one.
        let bb_join_block = &func.blocks[&bb_join];
        let x_phi_count = bb_join_block
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    &i.kind,
                    InstructionKind::Phi { sources, .. }
                        if sources.iter().any(|(_, op)| matches!(op, Operand::Local(id) if *id == x))
                )
            })
            .count();
        assert_eq!(
            x_phi_count, 0,
            "x is single-def with dominating def — pruned SSA must skip φ insertion"
        );
        assert!(crate::ssa_check::check(&func).is_ok());
    }
}
