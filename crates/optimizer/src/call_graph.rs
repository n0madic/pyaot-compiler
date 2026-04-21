//! Whole-program call-graph for MIR modules.
//!
//! Implements Phase 1 §1.5 (`ARCHITECTURE_REFACTOR.md`): a direct
//! caller/callee map plus Tarjan-style strongly-connected components for
//! bottom-up interprocedural analyses. Consumers include the whole-program
//! parameter-type inference (S1.11) and field-type inference (S1.12), both
//! of which iterate SCCs to fixed point.
//!
//! ## Call edges collected
//!
//! * **Direct** — `InstructionKind::CallDirect { func, .. }` and internal
//!   `InstructionKind::CallNamed { name, .. }` record precise
//!   `caller → callee` edges with the exact call site.
//! * **Indirect** — `InstructionKind::Call { func: Operand, .. }` through
//!   a function-pointer or closure. The graph traces `FuncAddr` /
//!   `Copy` / `Refine` / `Phi` chains at the site when possible; when a
//!   site can't be resolved precisely it falls back to the conservative
//!   address-taken set.
//! * **Virtual** — `CallVirtual` / `CallVirtualNamed`. Exact when the
//!   receiver already has a concrete class in MIR metadata; otherwise
//!   conservative across vtable entries filtered by slot or `name_hash`.
//! * `RuntimeCall` — runtime-library calls are not part of the user call
//!   graph; they do not flow into WPA decisions.

use std::collections::HashMap;

use indexmap::{IndexMap, IndexSet};
use pyaot_mir::{InstructionKind, Module, Operand};
use pyaot_types::Type;
use pyaot_utils::{BlockId, ClassId, FuncId, LocalId};

/// One specific call edge. `(caller, callee)` pairs are de-duplicated in
/// the graph by `(caller, callee)` key, but `CallSite::block` and
/// `instruction` let consumers locate the physical call for rewriting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSite {
    pub caller: FuncId,
    pub callee: FuncId,
    pub block: BlockId,
    /// Index of the call instruction within `block.instructions`. For
    /// virtual/indirect edges inferred from address-taken sets, no exact
    /// instruction exists — the field is set to `usize::MAX` as a
    /// sentinel.
    pub instruction: usize,
    /// Classification of the edge. Direct edges are exact; indirect and
    /// virtual edges are conservative over-approximations.
    pub kind: CallKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallSiteKey {
    pub caller: FuncId,
    pub block: BlockId,
    pub instruction: usize,
}

/// How precisely the call graph tracks a given edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// `CallDirect { func, .. }` — precisely one target.
    Direct,
    /// `Call { func: Operand::Local(_), .. }` — indirect through a
    /// function-pointer operand. The edge is conservative: it exists from
    /// the caller to every address-taken function.
    Indirect,
    /// `CallVirtual` / `CallVirtualNamed` — dispatched through a vtable.
    /// Conservative: targets methods reachable through module vtables
    /// (slot-matched when available; otherwise every known vtable method).
    Virtual,
}

/// Whole-program call graph.
#[derive(Debug, Clone, Default)]
pub struct CallGraph {
    /// `callers[f]` — every call site where `f` is invoked.
    pub callers: IndexMap<FuncId, Vec<CallSite>>,
    /// `callees[f]` — every call site originating from `f`.
    pub callees: IndexMap<FuncId, Vec<CallSite>>,
    /// Strongly connected components in topological order: the first
    /// SCC has no outgoing edges to later SCCs; each subsequent SCC may
    /// call into earlier ones (classical reverse-topological SCC ordering
    /// suitable for bottom-up analyses). Singletons are included.
    pub sccs: Vec<Vec<FuncId>>,
    /// Functions whose address is taken via `FuncAddr` anywhere in the
    /// module. Indirect calls conservatively target this set.
    pub address_taken: IndexSet<FuncId>,
    /// Exact or conservative targets for each physical call site.
    pub site_targets: HashMap<CallSiteKey, IndexSet<FuncId>>,
    /// Whether the site target set is exact (`true`) or a conservative
    /// over-approximation (`false`).
    pub site_exact: HashMap<CallSiteKey, bool>,
}

impl CallGraph {
    /// Build the call graph for `module` — runs one pass over every
    /// function's MIR body and one Tarjan SCC pass over the resulting
    /// digraph. Runs in `O(V + E)` time.
    pub fn build(module: &Module) -> Self {
        let address_taken = collect_address_taken(module);
        let local_name_targets = build_local_name_targets(module);
        let virtual_targets_by_slot = collect_virtual_targets_by_slot(module);
        let virtual_targets_by_name = collect_virtual_targets_by_name_hash(module);
        let vtable_slot_map = build_vtable_slot_map(module);
        let vtable_name_map = build_vtable_name_map(module);

        let mut callers: IndexMap<FuncId, Vec<CallSite>> = IndexMap::new();
        let mut callees: IndexMap<FuncId, Vec<CallSite>> = IndexMap::new();
        let mut site_targets: HashMap<CallSiteKey, IndexSet<FuncId>> = HashMap::new();
        let mut site_exact: HashMap<CallSiteKey, bool> = HashMap::new();
        // Ensure every function has an entry in both maps, even if it has
        // zero edges — simplifies consumers that iterate `callees.keys()`.
        for &func_id in module.functions.keys() {
            callers.entry(func_id).or_default();
            callees.entry(func_id).or_default();
        }

        for (&caller_id, func) in &module.functions {
            let def_map = build_local_def_map(func);
            for (&bid, block) in &func.blocks {
                for (idx, inst) in block.instructions.iter().enumerate() {
                    match &inst.kind {
                        InstructionKind::CallDirect { func: callee, .. } => {
                            push_edge(
                                &mut callers,
                                &mut callees,
                                &mut site_targets,
                                &mut site_exact,
                                CallSite {
                                    caller: caller_id,
                                    callee: *callee,
                                    block: bid,
                                    instruction: idx,
                                    kind: CallKind::Direct,
                                },
                                true,
                            );
                        }
                        InstructionKind::CallNamed { name, .. } => {
                            let Some(&callee) = local_name_targets.get(name.as_str()) else {
                                continue;
                            };
                            push_edge(
                                &mut callers,
                                &mut callees,
                                &mut site_targets,
                                &mut site_exact,
                                CallSite {
                                    caller: caller_id,
                                    callee,
                                    block: bid,
                                    instruction: idx,
                                    kind: CallKind::Direct,
                                },
                                true,
                            );
                        }
                        InstructionKind::Call { func, .. } => {
                            let exact_targets =
                                resolve_callable_targets(func, &def_map).filter(|t| !t.is_empty());
                            let is_exact = exact_targets.is_some();
                            let targets = exact_targets.unwrap_or_else(|| address_taken.clone());
                            for target in targets {
                                push_edge(
                                    &mut callers,
                                    &mut callees,
                                    &mut site_targets,
                                    &mut site_exact,
                                    CallSite {
                                        caller: caller_id,
                                        callee: target,
                                        block: bid,
                                        instruction: idx,
                                        kind: CallKind::Indirect,
                                    },
                                    is_exact,
                                );
                            }
                        }
                        InstructionKind::CallVirtual { obj, slot, .. } => {
                            if let Some(class_id) = operand_class_id(obj, func) {
                                if let Some(&target) = vtable_slot_map.get(&(class_id, *slot)) {
                                    push_edge(
                                        &mut callers,
                                        &mut callees,
                                        &mut site_targets,
                                        &mut site_exact,
                                        CallSite {
                                            caller: caller_id,
                                            callee: target,
                                            block: bid,
                                            instruction: idx,
                                            kind: CallKind::Virtual,
                                        },
                                        true,
                                    );
                                    continue;
                                }
                            }

                            let Some(targets) = virtual_targets_by_slot.get(slot) else {
                                continue;
                            };
                            for &target in targets {
                                push_edge(
                                    &mut callers,
                                    &mut callees,
                                    &mut site_targets,
                                    &mut site_exact,
                                    CallSite {
                                        caller: caller_id,
                                        callee: target,
                                        block: bid,
                                        instruction: idx,
                                        kind: CallKind::Virtual,
                                    },
                                    false,
                                );
                            }
                        }
                        InstructionKind::CallVirtualNamed { obj, name_hash, .. } => {
                            if let Some(class_id) = operand_class_id(obj, func) {
                                let exact_named_dispatch = module
                                    .class_info
                                    .get(&class_id)
                                    .is_some_and(|meta| !meta.is_protocol);
                                if exact_named_dispatch {
                                    if let Some(&target) =
                                        vtable_name_map.get(&(class_id, *name_hash))
                                    {
                                        push_edge(
                                            &mut callers,
                                            &mut callees,
                                            &mut site_targets,
                                            &mut site_exact,
                                            CallSite {
                                                caller: caller_id,
                                                callee: target,
                                                block: bid,
                                                instruction: idx,
                                                kind: CallKind::Virtual,
                                            },
                                            true,
                                        );
                                        continue;
                                    }
                                }
                            }

                            let Some(targets) = virtual_targets_by_name.get(name_hash) else {
                                continue;
                            };
                            for &target in targets {
                                push_edge(
                                    &mut callers,
                                    &mut callees,
                                    &mut site_targets,
                                    &mut site_exact,
                                    CallSite {
                                        caller: caller_id,
                                        callee: target,
                                        block: bid,
                                        instruction: idx,
                                        kind: CallKind::Virtual,
                                    },
                                    false,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let sccs = tarjan_sccs(&callees, module.functions.keys().copied());

        CallGraph {
            callers,
            callees,
            sccs,
            address_taken,
            site_targets,
            site_exact,
        }
    }

    pub fn targets_at(
        &self,
        caller: FuncId,
        block: BlockId,
        instruction: usize,
    ) -> IndexSet<FuncId> {
        self.site_targets
            .get(&CallSiteKey {
                caller,
                block,
                instruction,
            })
            .cloned()
            .unwrap_or_default()
    }

    pub fn site_targets_are_exact(
        &self,
        caller: FuncId,
        block: BlockId,
        instruction: usize,
    ) -> bool {
        self.site_exact
            .get(&CallSiteKey {
                caller,
                block,
                instruction,
            })
            .copied()
            .unwrap_or(false)
    }

    /// Returns the SCC index containing `func`. `O(N)` linear scan — fine
    /// for a one-shot lookup; callers that need many lookups should build
    /// their own `HashMap<FuncId, usize>` by walking `self.sccs` once.
    pub fn scc_of(&self, func: FuncId) -> Option<usize> {
        self.sccs.iter().position(|s| s.contains(&func))
    }

    /// Whether `func` participates in a cycle of the (direct) call graph
    /// — either via direct self-recursion (`f` calls `f`) or indirect
    /// recursion (`f ∈ SCC of size ≥ 2`). Indirect / virtual edges are
    /// **not** considered: the canonical over-approximation would mark
    /// too many functions as recursive for inliner heuristics.
    ///
    /// Returns `false` for singleton SCCs without self-loops — the common
    /// case for leaf helpers that are prime inlining candidates.
    pub fn is_recursive(&self, func: FuncId) -> bool {
        // Indirect / virtual edges inflate SCCs spuriously; check only the
        // direct subgraph.
        if let Some(callees) = self.callees.get(&func) {
            if callees
                .iter()
                .any(|s| s.callee == func && s.kind == CallKind::Direct)
            {
                return true;
            }
        }
        match self.scc_of(func) {
            Some(idx) => {
                let scc = &self.sccs[idx];
                // Non-trivial SCC if ≥ 2 direct-edge members.
                if scc.len() < 2 {
                    return false;
                }
                // Confirm the SCC edges are direct — Tarjan merged on
                // every edge kind. Check that at least one direct edge
                // connects two members.
                for &a in scc {
                    if let Some(callees) = self.callees.get(&a) {
                        for site in callees {
                            if site.kind == CallKind::Direct
                                && site.callee != a
                                && scc.contains(&site.callee)
                            {
                                return true;
                            }
                        }
                    }
                }
                false
            }
            None => false,
        }
    }
}

fn collect_address_taken(module: &Module) -> IndexSet<FuncId> {
    let mut out = IndexSet::new();
    for func in module.functions.values() {
        for block in func.blocks.values() {
            for inst in &block.instructions {
                if let InstructionKind::FuncAddr { func: target, .. } = &inst.kind {
                    out.insert(*target);
                }
            }
        }
    }
    out
}

fn build_local_name_targets(module: &Module) -> HashMap<&str, FuncId> {
    module
        .functions
        .iter()
        .map(|(&func_id, func)| (func.name.as_str(), func_id))
        .collect()
}

fn build_local_def_map(func: &pyaot_mir::Function) -> HashMap<LocalId, InstructionKind> {
    let mut defs = HashMap::new();
    for block in func.blocks.values() {
        for inst in &block.instructions {
            let dest = match &inst.kind {
                InstructionKind::Const { dest, .. }
                | InstructionKind::BinOp { dest, .. }
                | InstructionKind::UnOp { dest, .. }
                | InstructionKind::Call { dest, .. }
                | InstructionKind::CallDirect { dest, .. }
                | InstructionKind::CallNamed { dest, .. }
                | InstructionKind::CallVirtual { dest, .. }
                | InstructionKind::CallVirtualNamed { dest, .. }
                | InstructionKind::FuncAddr { dest, .. }
                | InstructionKind::BuiltinAddr { dest, .. }
                | InstructionKind::RuntimeCall { dest, .. }
                | InstructionKind::Copy { dest, .. }
                | InstructionKind::GcAlloc { dest, .. }
                | InstructionKind::FloatToInt { dest, .. }
                | InstructionKind::BoolToInt { dest, .. }
                | InstructionKind::IntToFloat { dest, .. }
                | InstructionKind::FloatBits { dest, .. }
                | InstructionKind::IntBitsToFloat { dest, .. }
                | InstructionKind::FloatAbs { dest, .. }
                | InstructionKind::ExcGetType { dest }
                | InstructionKind::ExcHasException { dest }
                | InstructionKind::ExcGetCurrent { dest }
                | InstructionKind::ExcCheckType { dest, .. }
                | InstructionKind::ExcCheckClass { dest, .. }
                | InstructionKind::Phi { dest, .. }
                | InstructionKind::Refine { dest, .. } => Some(*dest),
                _ => None,
            };
            if let Some(dest) = dest {
                defs.insert(dest, inst.kind.clone());
            }
        }
    }
    defs
}

fn resolve_callable_targets(
    operand: &Operand,
    def_map: &HashMap<LocalId, InstructionKind>,
) -> Option<IndexSet<FuncId>> {
    fn resolve_local(
        local: LocalId,
        def_map: &HashMap<LocalId, InstructionKind>,
        visiting: &mut IndexSet<LocalId>,
    ) -> Option<IndexSet<FuncId>> {
        if !visiting.insert(local) {
            return None;
        }
        let resolved = match def_map.get(&local)? {
            InstructionKind::FuncAddr { func, .. } => Some(IndexSet::from_iter([*func])),
            InstructionKind::Copy { src, .. } | InstructionKind::Refine { src, .. } => {
                resolve_operand(src, def_map, visiting)
            }
            InstructionKind::Phi { sources, .. } => {
                let mut out = IndexSet::new();
                for (_, src) in sources {
                    let targets = resolve_operand(src, def_map, visiting)?;
                    out.extend(targets);
                }
                Some(out)
            }
            _ => None,
        };
        visiting.shift_remove(&local);
        resolved
    }

    fn resolve_operand(
        operand: &Operand,
        def_map: &HashMap<LocalId, InstructionKind>,
        visiting: &mut IndexSet<LocalId>,
    ) -> Option<IndexSet<FuncId>> {
        match operand {
            Operand::Local(local) => resolve_local(*local, def_map, visiting),
            Operand::Constant(_) => None,
        }
    }

    resolve_operand(operand, def_map, &mut IndexSet::new())
}

fn collect_virtual_targets_by_slot(module: &Module) -> IndexMap<usize, IndexSet<FuncId>> {
    let mut out: IndexMap<usize, IndexSet<FuncId>> = IndexMap::new();
    for vtable in &module.vtables {
        for entry in &vtable.entries {
            out.entry(entry.slot)
                .or_default()
                .insert(entry.method_func_id);
        }
    }
    out
}

fn collect_virtual_targets_by_name_hash(module: &Module) -> IndexMap<u64, IndexSet<FuncId>> {
    let mut out: IndexMap<u64, IndexSet<FuncId>> = IndexMap::new();
    for vtable in &module.vtables {
        for entry in &vtable.entries {
            out.entry(entry.name_hash)
                .or_default()
                .insert(entry.method_func_id);
        }
    }
    out
}

fn build_vtable_slot_map(module: &Module) -> HashMap<(ClassId, usize), FuncId> {
    let mut map = HashMap::new();
    for vtable in &module.vtables {
        for entry in &vtable.entries {
            map.insert((vtable.class_id, entry.slot), entry.method_func_id);
        }
    }
    map
}

fn build_vtable_name_map(module: &Module) -> HashMap<(ClassId, u64), FuncId> {
    let mut map = HashMap::new();
    for vtable in &module.vtables {
        for entry in &vtable.entries {
            map.insert((vtable.class_id, entry.name_hash), entry.method_func_id);
        }
    }
    map
}

fn operand_class_id(operand: &Operand, func: &pyaot_mir::Function) -> Option<ClassId> {
    let Operand::Local(id) = operand else {
        return None;
    };
    let ty = func.locals.get(id).map(|local| &local.ty).or_else(|| {
        func.params
            .iter()
            .find(|param| param.id == *id)
            .map(|param| &param.ty)
    })?;
    match ty {
        Type::Class { class_id, .. } => Some(*class_id),
        _ => None,
    }
}

fn push_edge(
    callers: &mut IndexMap<FuncId, Vec<CallSite>>,
    callees: &mut IndexMap<FuncId, Vec<CallSite>>,
    site_targets: &mut HashMap<CallSiteKey, IndexSet<FuncId>>,
    site_exact: &mut HashMap<CallSiteKey, bool>,
    site: CallSite,
    exact: bool,
) {
    let key = CallSiteKey {
        caller: site.caller,
        block: site.block,
        instruction: site.instruction,
    };
    callees.entry(site.caller).or_default().push(site);
    callers.entry(site.callee).or_default().push(site);
    site_targets.entry(key).or_default().insert(site.callee);
    site_exact
        .entry(key)
        .and_modify(|current| *current &= exact)
        .or_insert(exact);
}

// ============================================================================
// Tarjan's strongly-connected-components algorithm (iterative)
// ============================================================================

/// Tarjan SCC on the caller→callee digraph. Returns SCCs in **reverse
/// topological order** — any SCC only points to earlier SCCs in the list.
/// Functions that don't appear as a key in `callees` (no outgoing edges)
/// still count as singleton SCCs if they appear in `all_funcs`.
fn tarjan_sccs(
    callees: &IndexMap<FuncId, Vec<CallSite>>,
    all_funcs: impl IntoIterator<Item = FuncId>,
) -> Vec<Vec<FuncId>> {
    // Classical iterative Tarjan to avoid recursion depth issues on
    // deeply-connected modules. Each node has (index, lowlink, on_stack).
    let mut index_counter: u32 = 0;
    let mut stack: Vec<FuncId> = Vec::new();
    let mut on_stack: HashMap<FuncId, bool> = HashMap::new();
    let mut index: HashMap<FuncId, u32> = HashMap::new();
    let mut lowlink: HashMap<FuncId, u32> = HashMap::new();
    let mut sccs: Vec<Vec<FuncId>> = Vec::new();

    // Dedup successor lists on-the-fly so repeated Indirect edges to the
    // same target don't force us to revisit nodes more than once.
    let succs: HashMap<FuncId, Vec<FuncId>> = callees
        .iter()
        .map(|(&k, sites)| {
            let mut v: Vec<FuncId> = sites.iter().map(|s| s.callee).collect();
            v.sort_by_key(|f| f.0);
            v.dedup();
            (k, v)
        })
        .collect();

    // Work-stack entries: (node, iter-index). We don't allocate a
    // separate Iterator — store the current successor index and
    // resume from there on re-entry.
    enum Frame {
        Enter(FuncId),
        Resume { node: FuncId, next: usize },
    }
    let mut work: Vec<Frame> = Vec::new();

    for root in all_funcs {
        if index.contains_key(&root) {
            continue;
        }
        work.push(Frame::Enter(root));
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Enter(v) => {
                    if index.contains_key(&v) {
                        continue;
                    }
                    index.insert(v, index_counter);
                    lowlink.insert(v, index_counter);
                    index_counter += 1;
                    stack.push(v);
                    on_stack.insert(v, true);
                    work.push(Frame::Resume { node: v, next: 0 });
                }
                Frame::Resume { node, next } => {
                    let empty: Vec<FuncId> = Vec::new();
                    let list = succs.get(&node).unwrap_or(&empty);
                    if next < list.len() {
                        let w = list[next];
                        // Save our resume point before descending.
                        work.push(Frame::Resume {
                            node,
                            next: next + 1,
                        });
                        if !index.contains_key(&w) {
                            work.push(Frame::Enter(w));
                        } else if *on_stack.get(&w).unwrap_or(&false) {
                            let v_low = *lowlink.get(&node).unwrap();
                            let w_idx = *index.get(&w).unwrap();
                            lowlink.insert(node, v_low.min(w_idx));
                        }
                    } else {
                        // All successors processed — propagate lowlink from
                        // any child currently on the stack.
                        for &w in list {
                            if *on_stack.get(&w).unwrap_or(&false) {
                                let v_low = *lowlink.get(&node).unwrap();
                                let w_low = *lowlink.get(&w).unwrap();
                                lowlink.insert(node, v_low.min(w_low));
                            }
                        }
                        if lowlink.get(&node) == index.get(&node) {
                            // Root of an SCC — pop until we pop `node`.
                            let mut scc = Vec::new();
                            while let Some(w) = stack.pop() {
                                on_stack.insert(w, false);
                                scc.push(w);
                                if w == node {
                                    break;
                                }
                            }
                            sccs.push(scc);
                        }
                    }
                }
            }
        }
    }

    sccs
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_mir::{Constant, Function, Instruction, Local, Operand, Terminator};
    use pyaot_types::Type;
    use pyaot_utils::LocalId;

    fn mk_func(id: u32) -> Function {
        Function::new(
            FuncId::from(id),
            format!("f{id}"),
            Vec::new(),
            Type::Int,
            None,
        )
    }

    fn add_call_direct(func: &mut Function, callee: FuncId) {
        let dest = LocalId::from(99u32);
        func.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
            },
        );
        let bb = func.entry_block;
        func.block_mut(bb).instructions.push(Instruction {
            kind: InstructionKind::CallDirect {
                dest,
                func: callee,
                args: Vec::new(),
            },
            span: None,
        });
        func.block_mut(bb).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Int(0))));
    }

    #[test]
    fn empty_module_has_no_sccs() {
        let module = Module::new();
        let cg = CallGraph::build(&module);
        assert!(cg.sccs.is_empty());
        assert!(cg.address_taken.is_empty());
    }

    #[test]
    fn singleton_with_no_calls_is_one_scc() {
        let mut module = Module::new();
        module.add_function(mk_func(0));
        let cg = CallGraph::build(&module);
        assert_eq!(cg.sccs.len(), 1);
        assert_eq!(cg.sccs[0], vec![FuncId::from(0u32)]);
        assert!(cg.callees[&FuncId::from(0u32)].is_empty());
        assert!(cg.callers[&FuncId::from(0u32)].is_empty());
    }

    #[test]
    fn linear_chain_three_singletons_reverse_topo() {
        // f0 → f1 → f2
        let mut module = Module::new();
        let mut f0 = mk_func(0);
        let f1_id = FuncId::from(1u32);
        let f2_id = FuncId::from(2u32);
        add_call_direct(&mut f0, f1_id);
        let mut f1 = mk_func(1);
        add_call_direct(&mut f1, f2_id);
        let f2 = mk_func(2);
        module.add_function(f0);
        module.add_function(f1);
        module.add_function(f2);

        let cg = CallGraph::build(&module);
        assert_eq!(cg.sccs.len(), 3);
        // Reverse-topo: leaves first (f2), then f1, then f0.
        assert_eq!(cg.sccs[0], vec![f2_id]);
        assert_eq!(cg.sccs[1], vec![f1_id]);
        assert_eq!(cg.sccs[2], vec![FuncId::from(0u32)]);

        // Edges bookkeeping.
        assert_eq!(cg.callees[&FuncId::from(0u32)].len(), 1);
        assert_eq!(cg.callers[&f2_id].len(), 1);
    }

    #[test]
    fn direct_recursion_is_one_scc() {
        // f0 → f0
        let mut module = Module::new();
        let mut f0 = mk_func(0);
        add_call_direct(&mut f0, FuncId::from(0u32));
        module.add_function(f0);
        let cg = CallGraph::build(&module);
        assert_eq!(cg.sccs.len(), 1);
        assert_eq!(cg.sccs[0], vec![FuncId::from(0u32)]);
    }

    #[test]
    fn mutual_recursion_is_one_scc() {
        // f0 ↔ f1 (both call each other); f2 isolated
        let mut module = Module::new();
        let mut f0 = mk_func(0);
        add_call_direct(&mut f0, FuncId::from(1u32));
        let mut f1 = mk_func(1);
        add_call_direct(&mut f1, FuncId::from(0u32));
        let f2 = mk_func(2);
        module.add_function(f0);
        module.add_function(f1);
        module.add_function(f2);

        let cg = CallGraph::build(&module);
        // Expect two SCCs: {f2} and {f0,f1}. f2 has no calls and f0/f1
        // form a cycle.
        assert_eq!(cg.sccs.len(), 2);
        assert!(cg.sccs.iter().any(|s| s.len() == 2
            && s.contains(&FuncId::from(0u32))
            && s.contains(&FuncId::from(1u32))));
        assert!(cg.sccs.iter().any(|s| s == &vec![FuncId::from(2u32)]));
        assert_eq!(cg.scc_of(FuncId::from(0u32)), cg.scc_of(FuncId::from(1u32)));
    }

    #[test]
    fn func_addr_makes_target_address_taken_and_indirect_call_edges_fan_out() {
        // f0 takes f2's address and makes an indirect call → conservative
        // edge f0→f2 (the only address-taken target). f1 is not
        // address-taken so no edge from f0 to f1.
        let mut module = Module::new();
        let mut f0 = mk_func(0);
        let dest_addr = LocalId::from(0u32);
        let dest_call = LocalId::from(1u32);
        f0.locals.insert(
            dest_addr,
            Local {
                id: dest_addr,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
            },
        );
        f0.locals.insert(
            dest_call,
            Local {
                id: dest_call,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
            },
        );
        let bb0 = f0.entry_block;
        f0.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::FuncAddr {
                dest: dest_addr,
                func: FuncId::from(2u32),
            },
            span: None,
        });
        f0.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Call {
                dest: dest_call,
                func: Operand::Local(dest_addr),
                args: Vec::new(),
            },
            span: None,
        });
        f0.block_mut(bb0).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Int(0))));

        let f1 = mk_func(1);
        let f2 = mk_func(2);
        module.add_function(f0);
        module.add_function(f1);
        module.add_function(f2);

        let cg = CallGraph::build(&module);
        assert!(cg.address_taken.contains(&FuncId::from(2u32)));
        assert!(!cg.address_taken.contains(&FuncId::from(1u32)));

        let f0_callees = &cg.callees[&FuncId::from(0u32)];
        let direct_targets: Vec<FuncId> = f0_callees.iter().map(|s| s.callee).collect();
        assert!(direct_targets.contains(&FuncId::from(2u32)));
        assert!(!direct_targets.contains(&FuncId::from(1u32)));

        // The indirect site is recorded with CallKind::Indirect.
        assert!(f0_callees.iter().any(|s| s.kind == CallKind::Indirect));
    }

    #[test]
    fn virtual_edges_target_slot_matched_vtable_methods() {
        let mut module = Module::new();
        let mut caller = mk_func(0);
        let obj = LocalId::from(0u32);
        let dest = LocalId::from(1u32);
        caller.locals.insert(
            obj,
            Local {
                id: obj,
                name: None,
                ty: Type::Any,
                is_gc_root: true,
            },
        );
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Any,
                is_gc_root: false,
            },
        );
        let bb0 = caller.entry_block;
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::CallVirtual {
                dest,
                obj: Operand::Local(obj),
                slot: 0,
                args: vec![Operand::Constant(Constant::Int(7))],
            },
            span: None,
        });
        caller.block_mut(bb0).terminator = Terminator::Return(None);

        let method = Function::new(
            FuncId::from(1u32),
            "C$m".to_string(),
            Vec::new(),
            Type::None,
            None,
        );

        module.add_function(caller);
        module.add_function(method);
        module.vtables.push(pyaot_mir::VtableInfo {
            class_id: pyaot_utils::ClassId::from(0u32),
            entries: vec![pyaot_mir::VtableEntry {
                slot: 0,
                name_hash: 0,
                method_func_id: FuncId::from(1u32),
            }],
        });

        let cg = CallGraph::build(&module);
        assert!(!cg.address_taken.contains(&FuncId::from(1u32)));
        assert!(cg.callees[&FuncId::from(0u32)]
            .iter()
            .any(|site| site.kind == CallKind::Virtual && site.callee == FuncId::from(1u32)));
    }

    #[test]
    fn internal_named_calls_resolve_to_exact_targets() {
        let mut module = Module::new();
        let mut caller = mk_func(0);
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
            },
        );
        let bb0 = caller.entry_block;
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::CallNamed {
                dest,
                name: "__module_other_callee".to_string(),
                args: vec![Operand::Constant(Constant::Int(7))],
            },
            span: None,
        });
        caller.block_mut(bb0).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Int(0))));

        let callee = Function::new(
            FuncId::from(1u32),
            "__module_other_callee".to_string(),
            Vec::new(),
            Type::None,
            None,
        );

        module.add_function(caller);
        module.add_function(callee);

        let cg = CallGraph::build(&module);
        let site_targets = cg.targets_at(FuncId::from(0u32), bb0, 0);
        let mut expected = IndexSet::new();
        expected.insert(FuncId::from(1u32));
        assert_eq!(site_targets, expected);
        assert!(cg.callees[&FuncId::from(0u32)]
            .iter()
            .any(|site| site.kind == CallKind::Direct && site.callee == FuncId::from(1u32)));
    }

    #[test]
    fn indirect_sites_follow_phi_of_funcaddr_targets() {
        let mut module = Module::new();
        let mut caller = mk_func(0);
        let addr_a = LocalId::from(0u32);
        let addr_b = LocalId::from(1u32);
        let phi = LocalId::from(2u32);
        let dest = LocalId::from(3u32);
        for local in [addr_a, addr_b, phi, dest] {
            caller.locals.insert(
                local,
                Local {
                    id: local,
                    name: None,
                    ty: Type::Int,
                    is_gc_root: false,
                },
            );
        }
        let bb0 = caller.entry_block;
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::FuncAddr {
                dest: addr_a,
                func: FuncId::from(1u32),
            },
            span: None,
        });
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::FuncAddr {
                dest: addr_b,
                func: FuncId::from(2u32),
            },
            span: None,
        });
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Phi {
                dest: phi,
                sources: vec![(bb0, Operand::Local(addr_a)), (bb0, Operand::Local(addr_b))],
            },
            span: None,
        });
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Call {
                dest,
                func: Operand::Local(phi),
                args: Vec::new(),
            },
            span: None,
        });
        caller.block_mut(bb0).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Int(0))));

        module.add_function(caller);
        module.add_function(mk_func(1));
        module.add_function(mk_func(2));
        module.add_function(mk_func(3));

        let cg = CallGraph::build(&module);
        let site_targets = cg.targets_at(FuncId::from(0u32), bb0, 3);
        let mut expected = IndexSet::new();
        expected.insert(FuncId::from(1u32));
        expected.insert(FuncId::from(2u32));
        assert_eq!(site_targets, expected);
        assert!(!site_targets.contains(&FuncId::from(3u32)));
    }
}
