//! Flow-sensitive type inference over SSA MIR — core engine (S1.8a).
//!
//! Phase 1 §1.4 (`ARCHITECTURE_REFACTOR.md`) replaces the three legacy
//! HIR-level inference paths (`compute_expr_type` /
//! `infer_expr_type_inner` / `infer_deep_expr_type`) with a single MIR-
//! level pass that walks the SSA CFG in reverse-post-order and computes
//! a type for every `LocalId`. This module lands the **core engine**:
//! the data structure (`TypeTable`) plus the RPO walk + fixed-point
//! iteration skeleton.
//!
//! ## Semantics
//!
//! * **Seed** — every `LocalId` starts with the type recorded in
//!   `func.locals[id].ty`. SSA construction (§1.3, S1.6) already copies
//!   the original Local's type onto each fresh version, so the seed is
//!   an upper bound (`Any` for unannotated params; the per-version
//!   concrete type otherwise).
//! * **Phi** — the join of the source operand types, using
//!   `Type::unify_field_type` (Phase 3 will replace this with the proper
//!   lattice `join`).
//! * **Refine** — the explicit `ty` field. This lets downstream analyses
//!   specialise code inside an `isinstance`-dominated block without
//!   tracking dominance themselves.
//! * **Other defining instructions** — inherit the seed type. S1.8b/c
//!   will extend this to compute dest types from operand types + op
//!   semantics (e.g. `BinOp::Add` on two `Int`s yields `Int`).
//!
//! The pass iterates to a fixed point bounded by
//! `MAX_ITERATIONS = func.locals.len() * 4` — well above the theoretical
//! bound for a monotone lattice ascent; in practice two iterations
//! suffice for well-formed SSA.
//!
//! ## What this pass does NOT do yet (S1.8b/c follow-up)
//!
//! * Compute per-instruction output types from operand types. The
//!   engine currently leaves every non-Phi non-Refine type at the seed
//!   value. S1.8b extends to cover `Const` / `BinOp` / `Copy` /
//!   `Call{Direct,Named}` etc.
//! * Emit `Refine` instructions at `isinstance`-branch successors.
//!   That's S1.8c.
//! * Delete the legacy `SymbolTable` maps (`refined_var_types`,
//!   `prescan_var_types`, `narrowed_union_vars`). S1.9.
//! * Integrate into the compile pipeline. Until S1.8b produces a
//!   non-trivial table, nothing consumes the output.

use std::collections::HashMap;

use indexmap::IndexMap;
use pyaot_mir::{Function, InstructionKind, Module, Operand, RuntimeFunc};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId};

/// Upper bound on TypeInferencePass iterations per function. A well-
/// formed SSA function reaches a fixed point in 1-2 sweeps; the cap is
/// defensive against pathological inputs (e.g., irreducible CFGs where
/// successor types feed back through phis).
const MAX_ITERATIONS_PER_FUNCTION: usize = 32;

/// Per-function mapping from `LocalId` → inferred `Type`. Every `LocalId`
/// that appears in `func.locals` is present; never `None`.
pub type FunctionTypes = IndexMap<LocalId, Type>;

/// Whole-module type table. Keyed by `FuncId`; per-function map is
/// indexed by `LocalId`.
#[derive(Debug, Clone, Default)]
pub struct TypeTable {
    per_func: HashMap<FuncId, FunctionTypes>,
}

impl TypeTable {
    /// Infer types for every function in `module`. Runs the RPO walk +
    /// fixed-point iteration per function independently; the pass is
    /// intra-procedural at this stage. WPA parameter / field inference
    /// (S1.11 / S1.12) layer on top by cross-linking call sites.
    pub fn infer_module(module: &Module) -> Self {
        let mut per_func: HashMap<FuncId, FunctionTypes> = HashMap::new();
        for (&func_id, func) in &module.functions {
            per_func.insert(func_id, infer_function(func, Some(module)));
        }
        Self { per_func }
    }

    /// Looks up the inferred type for `(func_id, local_id)`. Returns
    /// `None` if the function wasn't in the module at inference time or
    /// the LocalId has no entry (unreachable def or inconsistent input).
    pub fn get(&self, func_id: FuncId, local_id: LocalId) -> Option<&Type> {
        self.per_func.get(&func_id)?.get(&local_id)
    }

    /// Borrow the whole map for a function. Primarily for tests and
    /// MIR-dumping tools.
    pub fn function_types(&self, func_id: FuncId) -> Option<&FunctionTypes> {
        self.per_func.get(&func_id)
    }

    /// Replace a function's per-LocalId type map. Used by interprocedural
    /// passes (WPA param / field inference) that recompute a function's
    /// types after param or field updates flow in from other functions.
    pub fn set_function_types(&mut self, func_id: FuncId, types: FunctionTypes) {
        self.per_func.insert(func_id, types);
    }
}

/// Infer types for a single function. Exposed for targeted tests and
/// interprocedural passes that only care about one function at a time.
///
/// `module` is optional — callers that have the enclosing MIR module
/// available get cross-function lookups (e.g., `CallDirect` resolves to
/// the callee's declared return type). Stand-alone callers pass `None`
/// and accept that direct-call dests fall back to their seed type.
pub fn infer_function(func: &Function, module: Option<&Module>) -> FunctionTypes {
    infer_function_with_seed(func, module, &HashMap::new())
}

/// Same as [`infer_function`] but seeds selected `LocalId`s from
/// `seed_overrides` instead of `func.locals[id].ty`. The WPA parameter
/// inference pass (§1.6, S1.11) uses this to inject call-site-derived
/// types for function params, overriding the original HIR-lowering
/// coarse seed.
pub fn infer_function_with_seed(
    func: &Function,
    module: Option<&Module>,
    seed_overrides: &HashMap<LocalId, Type>,
) -> FunctionTypes {
    // Seed: every LocalId carries the type recorded on its Local entry —
    // unless overridden by `seed_overrides` (only relevant for function
    // params, whose type is externally provided).
    let mut types: FunctionTypes = IndexMap::new();
    for (&id, local) in &func.locals {
        let ty = seed_overrides
            .get(&id)
            .cloned()
            .unwrap_or_else(|| local.ty.clone());
        types.insert(id, ty);
    }

    // No SSA = no Phi = no fixed-point shuffling. Single seed pass is
    // sufficient for legacy non-SSA MIR; Refine still takes effect.
    if !func.is_ssa {
        apply_single_pass(func, &mut types, module);
        return types;
    }

    // RPO over the CFG — Phi sources must have been visited before the
    // Phi's block for meaningful join.
    let rpo: Vec<_> = func.dom_tree().reverse_post_order().to_vec();

    for _ in 0..MAX_ITERATIONS_PER_FUNCTION {
        let mut changed = false;
        for bid in &rpo {
            let block = match func.blocks.get(bid) {
                Some(b) => b,
                None => continue,
            };
            for inst in &block.instructions {
                if apply_instruction(&inst.kind, &mut types, module) {
                    changed = true;
                }
                // `Call` needs `&Function` context to trace its operand
                // back to a `FuncAddr` def; dispatched outside the
                // kind-only `apply_instruction` path.
                if matches!(&inst.kind, InstructionKind::Call { .. })
                    && infer_call_return_via_func_addr(&inst.kind, func, module, &mut types)
                {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    types
}

/// Apply the type-inference rule for a single instruction, returning
/// `true` if the table changed. Handles Phi, Refine, and the per-kind
/// result-type rules added in S1.8b (Const, Copy, CallDirect, GcAlloc).
/// Instructions without a rule leave the seed type intact — they are
/// neither widened nor narrowed by this pass.
fn apply_instruction(
    kind: &InstructionKind,
    types: &mut FunctionTypes,
    module: Option<&Module>,
) -> bool {
    match kind {
        InstructionKind::Phi { dest, sources } => {
            if let Some(new_ty) = join_operand_types(sources.iter().map(|(_, op)| op), types) {
                return update_type(types, *dest, new_ty);
            }
            false
        }
        InstructionKind::Refine { dest, ty, .. } => update_type(types, *dest, ty.clone()),

        // S1.8b rules. Each is narrow enough that over-writing the seed
        // is monotone (any refined operand types from earlier Refine /
        // Phi nodes only narrow, never widen, the result).
        InstructionKind::Const { dest, value } => update_type(types, *dest, constant_type(value)),
        InstructionKind::Copy { dest, src } => {
            let ty = operand_type(src, types);
            update_type(types, *dest, ty)
        }
        InstructionKind::CallDirect { dest, func, .. } => {
            match module.and_then(|m| m.functions.get(func)) {
                Some(callee) => update_type(types, *dest, callee.return_type.clone()),
                None => false,
            }
        }
        InstructionKind::GcAlloc { dest, ty, .. } => update_type(types, *dest, ty.clone()),

        // S1.8c rules.
        InstructionKind::BinOp {
            dest,
            op,
            left,
            right,
        } => {
            let left_ty = operand_type(left, types);
            let right_ty = operand_type(right, types);
            let result_ty = binop_result_type(*op, &left_ty, &right_ty);
            update_type(types, *dest, result_ty)
        }
        InstructionKind::UnOp { dest, op, operand } => {
            let operand_ty = operand_type(operand, types);
            let result_ty = unop_result_type(*op, &operand_ty);
            update_type(types, *dest, result_ty)
        }
        InstructionKind::RuntimeCall { dest, func, .. } => match runtime_call_return_type(func) {
            Some(ty) => update_type(types, *dest, ty),
            None => false,
        },

        // Remaining kinds (CallNamed, CallVirtual*, exc / boxing /
        // conversion helpers) stay at the seed type for now.
        // `InstructionKind::Call` goes through a dedicated path in
        // `run_pass` that has the `func: &Function` context needed to
        // resolve a function-pointer operand back to its defining
        // `FuncAddr`; see `infer_call_return_via_func_addr` below.
        InstructionKind::Call { .. } => false,

        _ => false,
    }
}

/// Resolve an indirect `Call`'s dest type by tracing the function-pointer
/// operand back to its defining `FuncAddr` within the same function. A
/// pure syntactic lookup — no data-flow analysis. Works in the common
/// closure-lowering pattern:
///
/// ```text
/// t = FuncAddr(some_func);
/// result = Call(t, [args]);
/// ```
///
/// which SSA construction leaves intact because `t` has a single defining
/// instruction. If `t` is defined by anything other than `FuncAddr`
/// (e.g. a `Phi` over multiple candidates, or a `Copy` from a parameter),
/// returns `None` and the dest stays at seed.
fn infer_call_return_via_func_addr(
    inst: &InstructionKind,
    func: &Function,
    module: Option<&Module>,
    types: &mut FunctionTypes,
) -> bool {
    let (dest, callee_operand) = match inst {
        InstructionKind::Call { dest, func, .. } => (*dest, func),
        _ => return false,
    };
    let Operand::Local(target_local) = callee_operand else {
        return false;
    };
    let Some(module) = module else {
        return false;
    };

    // Scan all blocks for the single defining FuncAddr of `target_local`.
    // SSA guarantees at most one def, so the first hit is authoritative.
    let mut callee_id = None;
    'outer: for block in func.blocks.values() {
        for i in &block.instructions {
            if let InstructionKind::FuncAddr {
                dest: addr_dest,
                func: target,
            } = &i.kind
            {
                if addr_dest == target_local {
                    callee_id = Some(*target);
                    break 'outer;
                }
            }
        }
    }
    let Some(callee_id) = callee_id else {
        return false;
    };
    let Some(callee) = module.functions.get(&callee_id) else {
        return false;
    };
    update_type(types, dest, callee.return_type.clone())
}

/// Result type of `left op right`. Comparison / logical-and/or / boolean
/// operators return `Bool`. Arithmetic promotes through the numeric tower
/// via `Type::unify_numeric`. Bitwise operators preserve the operand type
/// (both sides must be integer-compatible; enforced at lowering).
///
/// Pre-Phase-3 lattice limitations: when either operand is `Any` the
/// result falls back to `Any`. `Type::Never` (WPA bottom seed) absorbs
/// to the other side.
fn binop_result_type(op: pyaot_mir::BinOp, left: &Type, right: &Type) -> Type {
    use pyaot_mir::BinOp;
    match op {
        // Comparisons always yield Bool.
        BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtE | BinOp::Gt | BinOp::GtE => Type::Bool,

        // Short-circuit logical ops. Python's `and`/`or` return one of
        // the operands, not a Bool — the conservative upper bound is the
        // union of operand types. `Type::unify_field_type` gives that
        // via `normalize_union`.
        BinOp::And | BinOp::Or => merge_operand_types(left, right),

        // Division in Python always produces Float (`/` is true division;
        // `//` is FloorDiv below). MIR preserves that distinction.
        BinOp::Div => Type::Float,

        // Floor division, modulus, and arithmetic: classical numeric
        // tower via `Type::unify_numeric`. Bool + Int = Int; Int + Float
        // = Float; etc.
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::FloorDiv | BinOp::Mod | BinOp::Pow => {
            merge_operand_types(left, right)
        }

        // Bitwise operators: in Python these are defined on Int (and
        // Bool as 0/1). Result matches the wider operand type — promote
        // via the numeric tower for consistency.
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::LShift | BinOp::RShift => {
            merge_operand_types(left, right)
        }
    }
}

/// Result type of `op operand`. `Neg` on Int → Int, on Float → Float
/// (preserves numeric type); `Not` always → Bool; `Invert` (`~`) preserves
/// the integer operand type.
fn unop_result_type(op: pyaot_mir::UnOp, operand: &Type) -> Type {
    use pyaot_mir::UnOp;
    match op {
        UnOp::Not => Type::Bool,
        UnOp::Neg | UnOp::Invert => operand.clone(),
    }
}

/// Return type of an `InstructionKind::RuntimeCall`, for the subset of
/// `RuntimeFunc` variants whose Python-level return type is unambiguous.
/// Returns `None` for the descriptor-based `RuntimeFunc::Call(def)` —
/// its Cranelift `returns` field doesn't distinguish raw `Int` from a
/// heap pointer at I64, so we leave those at the seed. Other variants
/// that return nothing (`AssertFail`, `PrintValue`, etc.) are handled
/// at SSA-construction time via `runtime_call_is_void` — they don't
/// produce defs so this helper isn't reached for them.
fn runtime_call_return_type(func: &RuntimeFunc) -> Option<Type> {
    match func {
        // Heap-allocated strings / bytes — always return a pointer to
        // the respective object type.
        RuntimeFunc::MakeStr => Some(Type::Str),
        RuntimeFunc::MakeBytes => Some(Type::Bytes),

        // Exception-handling runtime. These have fixed Cranelift
        // return types that map cleanly to Python types.
        RuntimeFunc::ExcSetjmp => Some(Type::Int), // i32 result code
        RuntimeFunc::ExcGetType => Some(Type::Int), // i32 type tag
        RuntimeFunc::ExcHasException => Some(Type::Bool),
        RuntimeFunc::ExcGetCurrent => Some(Type::HeapAny), // ptr to exception obj
        RuntimeFunc::ExcIsinstanceClass => Some(Type::Bool),
        RuntimeFunc::ExcInstanceStr => Some(Type::Str),

        // Descriptor-based calls span the entire stdlib; their return
        // type is the RuntimeFuncDef's `returns` field, but that
        // Cranelift-level type (I64, etc.) is ambiguous between Int
        // and heap pointer. Leave at seed — WPA and/or consumer-side
        // context disambiguate.
        RuntimeFunc::Call(_) => None,

        // Void / noreturn runtime calls never reach here (their dest
        // isn't treated as a def by SSA construction). Covered for
        // exhaustiveness only.
        RuntimeFunc::AssertFail
        | RuntimeFunc::PrintValue(_)
        | RuntimeFunc::ExcRegisterClassName
        | RuntimeFunc::ExcRaise
        | RuntimeFunc::ExcReraise
        | RuntimeFunc::ExcClear
        | RuntimeFunc::ExcRaiseCustom => None,
    }
}

/// Join two operand types monotonically, handling the `Any` / `Never`
/// bounds `Type::unify_field_type` doesn't simplify on its own.
fn merge_operand_types(a: &Type, b: &Type) -> Type {
    if matches!(a, Type::Never) {
        return b.clone();
    }
    if matches!(b, Type::Never) {
        return a.clone();
    }
    if matches!(a, Type::Any) || matches!(b, Type::Any) {
        return Type::Any;
    }
    Type::unify_field_type(a, b)
}

/// Single-pass type refinement for legacy (non-SSA) MIR: Refine still
/// applies, and so does the Phi join when Phi instructions happen to be
/// present, but no fixed-point iteration is performed. Every non-SSA
/// function in the workspace must still produce a consistent
/// `FunctionTypes` so consumers can query uniformly.
fn apply_single_pass(func: &Function, types: &mut FunctionTypes, module: Option<&Module>) {
    for block in func.blocks.values() {
        for inst in &block.instructions {
            apply_instruction(&inst.kind, types, module);
            if matches!(&inst.kind, InstructionKind::Call { .. }) {
                infer_call_return_via_func_addr(&inst.kind, func, module, types);
            }
        }
    }
}

// ============================================================================
// Whole-program parameter type inference (§1.6, S1.11a core)
// ============================================================================

/// Upper bound on WPA fixed-point sweeps over each SCC. WPA converges in
/// `O(num_union_widenings)` per SCC — joining types up the lattice is
/// monotone, so after each function's params have been widened to their
/// max, further iterations are no-ops. 16 is comfortably above the
/// worst-case bound for Python's small type lattice.
const MAX_WPA_SCC_ITERATIONS: usize = 16;

/// Refine every function's parameter types from the join of arg types at
/// all **direct** call sites, then re-run intra-procedural inference on
/// each refined function. Writes the updated `FunctionTypes` back into
/// `table` in-place.
///
/// Processes `CallGraph::sccs` in **reverse order** so that each SCC's
/// callers are fully inferred before we infer the callee's params: the
/// spec's `sccs` vector is reverse-topological (leaves first, roots
/// last), so iterating `.rev()` starts with the roots / entry points
/// and works downward through the call tree. Within each SCC, iterate
/// to fixed point to handle mutual recursion.
///
/// Indirect and virtual calls are currently skipped (no reliable way to
/// tie a specific arg to a specific callee without devirtualisation).
/// Devirtualisation (S1.15) will convert virtual calls to direct calls
/// where the receiver's concrete class is known, at which point WPA
/// picks them up automatically.
///
/// When a function has no direct callers, its params are left at the
/// seed type — typically `Any` for unannotated params, or the declared
/// annotation otherwise. Entry points like `__pyaot_module_init__` fall
/// into this bucket.
pub fn wpa_param_inference(
    module: &Module,
    call_graph: &crate::call_graph::CallGraph,
    table: &mut TypeTable,
) {
    use crate::call_graph::CallKind;

    // Reset params of every directly-called function to `Type::Never`
    // (lattice bottom) before the fixed-point iteration. Dataflow joins
    // then widen monotonically from bottom — without this seed-clearing
    // step a recursive self-call picks up the pre-WPA seed (typically
    // `Any`) on its first pass and contaminates the join forever, since
    // `Type::unify_field_type` doesn't simplify `Union([Any, Int])` to
    // just `Any`. Functions with **no** direct callers (entry points,
    // externally-invoked) keep their original seed, so their test-only
    // behaviour is unchanged.
    for (&func_id, func) in &module.functions {
        let has_direct_caller = call_graph
            .callers
            .get(&func_id)
            .map(|sites| sites.iter().any(|s| s.kind == CallKind::Direct))
            .unwrap_or(false);
        if !has_direct_caller {
            continue;
        }
        if let Some(existing) = table.function_types(func_id).cloned() {
            let mut cleared = existing;
            for p in &func.params {
                cleared.insert(p.id, Type::Never);
            }
            table.set_function_types(func_id, cleared);
        }
    }

    // Reverse-topological iteration: roots (callers) → leaves (callees).
    // We need caller arg types to refine callee params, so callers must
    // be stable before we touch their callees.
    let scc_order: Vec<Vec<FuncId>> = call_graph.sccs.iter().rev().cloned().collect();

    for scc in &scc_order {
        for _iter in 0..MAX_WPA_SCC_ITERATIONS {
            let mut any_changed = false;
            for &func_id in scc {
                if refine_function_params(module, call_graph, table, func_id) {
                    any_changed = true;
                }
            }
            if !any_changed {
                break;
            }
        }
    }
}

/// Run [`wpa_param_inference`] to a whole-program fixed point.
///
/// A single `wpa_param_inference` call iterates each SCC internally to
/// fixed point, but once an SCC is closed the algorithm moves on and
/// never re-visits it. When a later SCC changes its callers' return
/// types — which may flow back up into an earlier SCC's call-site arg
/// types — that earlier SCC's param types could be further refined. This
/// wrapper loops `wpa_param_inference` until a full module pass makes no
/// changes, guaranteeing every function's params have seen every
/// applicable refinement.
///
/// In practice two passes suffice: one to compute return types across
/// the SCC graph, a second to propagate those return types back into
/// callers' arg-type observations. The upper bound
/// `MAX_FULL_PROGRAM_ITERATIONS = 8` is defensive against pathological
/// cases (mutual-recursion edge cases where monotone ascent needs more
/// widening rounds); it is never expected to be hit for well-formed
/// Python programs.
pub fn wpa_param_inference_to_fixed_point(
    module: &Module,
    call_graph: &crate::call_graph::CallGraph,
    table: &mut TypeTable,
) {
    for _ in 0..MAX_FULL_PROGRAM_ITERATIONS {
        // Snapshot the per-function types before the pass so we can
        // detect whether anything globally changed. `TypeTable` is
        // `Clone`; each inner `IndexMap` is relatively small in
        // practice (a few hundred entries per function at most) so the
        // snapshot is cheap.
        let before = table.per_func.clone();
        wpa_param_inference(module, call_graph, table);
        if before == table.per_func {
            break;
        }
    }
}

const MAX_FULL_PROGRAM_ITERATIONS: usize = 8;

/// Monotone join for WPA's fixed-point ascent. Treats `Never` as bottom
/// (absorbs into the other operand) and `Any` as top (absorbs the other
/// operand); otherwise delegates to `Type::unify_field_type` for the
/// numeric-tower + tuple-shape logic. The two special cases are needed
/// because `Type::unify_field_type` / `normalize_union` don't themselves
/// simplify `Any ⊔ Int` to `Any`, leaving the ascent stuck on
/// `Union([Any, Int])` for recursive SCCs.
fn wpa_join_types(a: &Type, b: &Type) -> Type {
    if matches!(a, Type::Never) {
        return b.clone();
    }
    if matches!(b, Type::Never) {
        return a.clone();
    }
    if matches!(a, Type::Any) || matches!(b, Type::Any) {
        return Type::Any;
    }
    Type::unify_field_type(a, b)
}

/// One pass over a single function: collect arg types across every
/// direct call site, join them per parameter position, seed the function
/// with the joined types, and re-run intra-procedural inference.
/// Returns `true` if the resulting `FunctionTypes` differ from what was
/// previously stored in `table` for this function.
fn refine_function_params(
    module: &Module,
    call_graph: &crate::call_graph::CallGraph,
    table: &mut TypeTable,
    func_id: FuncId,
) -> bool {
    use crate::call_graph::CallKind;

    let func = match module.functions.get(&func_id) {
        Some(f) => f,
        None => return false,
    };
    let n_params = func.params.len();
    if n_params == 0 {
        return false;
    }

    // Join arg types from every direct call site.
    let mut joined: Vec<Option<Type>> = vec![None; n_params];
    let callers = match call_graph.callers.get(&func_id) {
        Some(c) => c,
        None => return false,
    };
    for site in callers {
        if site.kind != CallKind::Direct {
            // Skip Indirect / Virtual — we can't reliably match their arg
            // lists to a specific callee without devirtualisation.
            continue;
        }
        let caller = match module.functions.get(&site.caller) {
            Some(c) => c,
            None => continue,
        };
        let block = match caller.blocks.get(&site.block) {
            Some(b) => b,
            None => continue,
        };
        let inst = match block.instructions.get(site.instruction) {
            Some(i) => i,
            None => continue,
        };
        let args = match &inst.kind {
            InstructionKind::CallDirect { args, .. } => args,
            _ => continue,
        };
        let caller_types = table.function_types(site.caller);
        for (i, arg) in args.iter().enumerate().take(n_params) {
            let arg_ty = match caller_types {
                Some(t) => operand_type(arg, t),
                None => Type::Any,
            };
            joined[i] = Some(match &joined[i] {
                None => arg_ty,
                Some(prev) => wpa_join_types(prev, &arg_ty),
            });
        }
    }

    // Build seed overrides for every param we derived a type for.
    let mut overrides: HashMap<LocalId, Type> = HashMap::new();
    for (i, param) in func.params.iter().enumerate() {
        if let Some(ty) = &joined[i] {
            overrides.insert(param.id, ty.clone());
        }
    }
    if overrides.is_empty() {
        return false;
    }

    // Re-run intra-procedural inference with the new param seeds.
    let new_types = infer_function_with_seed(func, Some(module), &overrides);

    // Only mark the table dirty if the re-inferred map actually differs.
    let differs = table
        .function_types(func_id)
        .is_none_or(|prev| prev != &new_types);
    if differs {
        table.set_function_types(func_id, new_types);
    }
    differs
}

fn update_type(types: &mut FunctionTypes, id: LocalId, new_ty: Type) -> bool {
    match types.get(&id) {
        Some(existing) if *existing == new_ty => false,
        _ => {
            types.insert(id, new_ty);
            true
        }
    }
}

/// Fold operand types together via `Type::unify_field_type`. Returns
/// `None` if the operand list is empty (a malformed phi). Constants
/// contribute their literal type (`Int`/`Float`/`Bool`/`Str`/`Bytes`/
/// `None`), locals contribute their current entry in `types`.
fn join_operand_types<'a, I>(ops: I, types: &FunctionTypes) -> Option<Type>
where
    I: IntoIterator<Item = &'a Operand>,
{
    let mut acc: Option<Type> = None;
    for op in ops {
        let ty = operand_type(op, types);
        acc = Some(match acc {
            None => ty,
            Some(prev) => Type::unify_field_type(&prev, &ty),
        });
    }
    acc
}

fn operand_type(op: &Operand, types: &FunctionTypes) -> Type {
    match op {
        Operand::Local(id) => types.get(id).cloned().unwrap_or(Type::Any),
        Operand::Constant(c) => constant_type(c),
    }
}

fn constant_type(c: &pyaot_mir::Constant) -> Type {
    use pyaot_mir::Constant;
    match c {
        Constant::Int(_) => Type::Int,
        Constant::Float(_) => Type::Float,
        Constant::Bool(_) => Type::Bool,
        Constant::Str(_) => Type::Str,
        Constant::Bytes(_) => Type::Bytes,
        Constant::None => Type::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_mir::{
        BasicBlock, Constant, Function, Instruction, InstructionKind, Local, Operand, Terminator,
    };
    use pyaot_utils::{BlockId, FuncId, LocalId};

    fn mk_local(id: u32, ty: Type) -> Local {
        Local {
            id: LocalId::from(id),
            name: None,
            ty,
            is_gc_root: false,
        }
    }

    fn empty_func(return_ty: Type) -> Function {
        Function::new(
            FuncId::from(0u32),
            "test".to_string(),
            Vec::new(),
            return_ty,
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
    fn seed_from_locals_carries_into_table() {
        let mut func = empty_func(Type::Int);
        let l = LocalId::from(1u32);
        func.locals.insert(l, mk_local(1, Type::Int));
        let bb0 = BlockId::from(0u32);
        func.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(l)));

        let types = infer_function(&func, None);
        assert_eq!(types.get(&l), Some(&Type::Int));
    }

    #[test]
    fn phi_joins_identical_types_to_same_type() {
        // bb0: const=true → branch bb1/bb2; bb1/bb2 define same Int;
        // bb3: phi(Int, Int) → return
        let mut func = empty_func(Type::Int);
        let c = LocalId::from(0u32);
        let a = LocalId::from(1u32);
        let b = LocalId::from(2u32);
        let m = LocalId::from(3u32);
        func.locals.insert(c, mk_local(0, Type::Bool));
        func.locals.insert(a, mk_local(1, Type::Int));
        func.locals.insert(b, mk_local(2, Type::Int));
        func.locals.insert(m, mk_local(3, Type::Int));

        let bb0 = BlockId::from(0u32);
        let bb1 = add_block(&mut func, 1, Terminator::Unreachable);
        let bb2 = add_block(&mut func, 2, Terminator::Unreachable);
        let bb3 = add_block(&mut func, 3, Terminator::Unreachable);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: c,
                value: Constant::Bool(true),
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
                dest: a,
                value: Constant::Int(1),
            },
            span: None,
        });
        func.block_mut(bb1).terminator = Terminator::Goto(bb3);
        func.block_mut(bb2).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: b,
                value: Constant::Int(2),
            },
            span: None,
        });
        func.block_mut(bb2).terminator = Terminator::Goto(bb3);
        func.block_mut(bb3).instructions.push(Instruction {
            kind: InstructionKind::Phi {
                dest: m,
                sources: vec![(bb1, Operand::Local(a)), (bb2, Operand::Local(b))],
            },
            span: None,
        });
        func.block_mut(bb3).terminator = Terminator::Return(Some(Operand::Local(m)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&m), Some(&Type::Int));
    }

    #[test]
    fn phi_joins_int_and_float_to_float_via_numeric_tower() {
        // Cross-check that the engine uses `Type::unify_field_type` —
        // joining Int and Float should promote to Float (numeric tower).
        let mut func = empty_func(Type::Float);
        let c = LocalId::from(0u32);
        let a = LocalId::from(1u32);
        let b = LocalId::from(2u32);
        let m = LocalId::from(3u32);
        func.locals.insert(c, mk_local(0, Type::Bool));
        func.locals.insert(a, mk_local(1, Type::Int));
        func.locals.insert(b, mk_local(2, Type::Float));
        func.locals.insert(m, mk_local(3, Type::Int));

        let bb0 = BlockId::from(0u32);
        let bb1 = add_block(&mut func, 1, Terminator::Unreachable);
        let bb2 = add_block(&mut func, 2, Terminator::Unreachable);
        let bb3 = add_block(&mut func, 3, Terminator::Unreachable);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: c,
                value: Constant::Bool(true),
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Branch {
            cond: Operand::Local(c),
            then_block: bb1,
            else_block: bb2,
        };
        func.block_mut(bb1).terminator = Terminator::Goto(bb3);
        func.block_mut(bb2).terminator = Terminator::Goto(bb3);
        func.block_mut(bb3).instructions.push(Instruction {
            kind: InstructionKind::Phi {
                dest: m,
                sources: vec![(bb1, Operand::Local(a)), (bb2, Operand::Local(b))],
            },
            span: None,
        });
        func.block_mut(bb3).terminator = Terminator::Return(Some(Operand::Local(m)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        // Seed for m is Int, but the Phi's join of (Int, Float) promotes
        // via the numeric tower to Float.
        assert_eq!(types.get(&m), Some(&Type::Float));
    }

    #[test]
    fn refine_narrows_dest_to_explicit_type() {
        let mut func = empty_func(Type::Int);
        let l = LocalId::from(1u32);
        let r = LocalId::from(2u32);
        func.locals.insert(l, mk_local(1, Type::Any));
        func.locals.insert(r, mk_local(2, Type::Any));
        let bb0 = BlockId::from(0u32);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l,
                value: Constant::Int(7),
            },
            span: None,
        });
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Refine {
                dest: r,
                src: Operand::Local(l),
                ty: Type::Int,
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(r)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Int));
    }

    #[test]
    fn constant_operand_contributes_its_literal_type() {
        // Phi where one source is an Int constant directly (no Local in
        // between) — verifies operand_type's constant branch.
        let mut func = empty_func(Type::Int);
        let c = LocalId::from(0u32);
        let a = LocalId::from(1u32);
        let m = LocalId::from(2u32);
        func.locals.insert(c, mk_local(0, Type::Bool));
        func.locals.insert(a, mk_local(1, Type::Int));
        func.locals.insert(m, mk_local(2, Type::Any));

        let bb0 = BlockId::from(0u32);
        let bb1 = add_block(&mut func, 1, Terminator::Unreachable);
        let bb2 = add_block(&mut func, 2, Terminator::Unreachable);
        let bb3 = add_block(&mut func, 3, Terminator::Unreachable);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: c,
                value: Constant::Bool(true),
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
                dest: a,
                value: Constant::Int(1),
            },
            span: None,
        });
        func.block_mut(bb1).terminator = Terminator::Goto(bb3);
        func.block_mut(bb2).terminator = Terminator::Goto(bb3);
        func.block_mut(bb3).instructions.push(Instruction {
            kind: InstructionKind::Phi {
                dest: m,
                sources: vec![
                    (bb1, Operand::Local(a)),
                    (bb2, Operand::Constant(Constant::Int(42))),
                ],
            },
            span: None,
        });
        func.block_mut(bb3).terminator = Terminator::Return(Some(Operand::Local(m)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&m), Some(&Type::Int));
    }

    #[test]
    fn module_level_infer_covers_every_function() {
        let mut module = Module::new();
        let mut f0 = empty_func(Type::Int);
        f0.locals
            .insert(LocalId::from(0u32), mk_local(0, Type::Int));
        f0.block_mut(f0.entry_block).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Int(0))));
        let mut f1 = Function::new(
            FuncId::from(1u32),
            "f1".to_string(),
            Vec::new(),
            Type::Float,
            None,
        );
        f1.locals
            .insert(LocalId::from(0u32), mk_local(0, Type::Float));
        f1.block_mut(f1.entry_block).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Float(0.0))));
        module.add_function(f0);
        module.add_function(f1);

        let table = TypeTable::infer_module(&module);
        assert_eq!(
            table.get(FuncId::from(0u32), LocalId::from(0u32)),
            Some(&Type::Int)
        );
        assert_eq!(
            table.get(FuncId::from(1u32), LocalId::from(0u32)),
            Some(&Type::Float)
        );
    }

    /// An earlier-in-RPO Phi that feeds a later Phi requires at least
    /// one iteration before the second Phi sees its source's joined
    /// type. Fixed-point ensures both settle.
    #[test]
    fn fixed_point_handles_chained_phis() {
        // bb0 → bb1 (phi m1) → bb2 (phi m2 sources = m1 and const)
        // Use Float+Bool so join widens to Float.
        let mut func = empty_func(Type::Float);
        let c = LocalId::from(0u32);
        let a = LocalId::from(1u32);
        let b = LocalId::from(2u32);
        let m1 = LocalId::from(3u32);
        let m2 = LocalId::from(4u32);
        func.locals.insert(c, mk_local(0, Type::Bool));
        func.locals.insert(a, mk_local(1, Type::Bool));
        func.locals.insert(b, mk_local(2, Type::Float));
        func.locals.insert(m1, mk_local(3, Type::Bool));
        func.locals.insert(m2, mk_local(4, Type::Bool));

        let bb0 = BlockId::from(0u32);
        let bb1 = add_block(&mut func, 1, Terminator::Unreachable);
        let bb2 = add_block(&mut func, 2, Terminator::Unreachable);
        let bb3 = add_block(&mut func, 3, Terminator::Unreachable);

        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: c,
                value: Constant::Bool(true),
            },
            span: None,
        });
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: a,
                value: Constant::Bool(false),
            },
            span: None,
        });
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: b,
                value: Constant::Float(1.5),
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Branch {
            cond: Operand::Local(c),
            then_block: bb1,
            else_block: bb2,
        };
        // bb1: Phi m1 from (bb0, a=Bool) alone — stays Bool.
        func.block_mut(bb1).instructions.push(Instruction {
            kind: InstructionKind::Phi {
                dest: m1,
                sources: vec![(bb0, Operand::Local(a))],
            },
            span: None,
        });
        func.block_mut(bb1).terminator = Terminator::Goto(bb3);
        // bb2 takes the else arm, goes to bb3 directly.
        func.block_mut(bb2).terminator = Terminator::Goto(bb3);
        // bb3: Phi m2 = (bb1 → m1 which is Bool, bb2 → b which is Float).
        // Join should be Float via numeric tower.
        func.block_mut(bb3).instructions.push(Instruction {
            kind: InstructionKind::Phi {
                dest: m2,
                sources: vec![(bb1, Operand::Local(m1)), (bb2, Operand::Local(b))],
            },
            span: None,
        });
        func.block_mut(bb3).terminator = Terminator::Return(Some(Operand::Local(m2)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&m1), Some(&Type::Bool));
        assert_eq!(types.get(&m2), Some(&Type::Float));
    }

    /// S1.8b: `Const` rule. Dest type becomes the literal's type regardless
    /// of the seed. Seeding `Any` and writing a Bool literal must refine
    /// to `Bool`.
    #[test]
    fn const_rule_narrows_dest_to_literal_type() {
        let mut func = empty_func(Type::Bool);
        let l = LocalId::from(1u32);
        // Seed is Any — coarser than the literal.
        func.locals.insert(l, mk_local(1, Type::Any));
        let bb0 = BlockId::from(0u32);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: l,
                value: Constant::Bool(true),
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(l)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&l), Some(&Type::Bool));
    }

    /// S1.8b: `Copy` rule. Dest inherits `src`'s current inferred type —
    /// critical for propagating Refine narrowings through copy chains.
    #[test]
    fn copy_rule_propagates_refined_src_type() {
        let mut func = empty_func(Type::Int);
        let src = LocalId::from(1u32);
        let refined = LocalId::from(2u32);
        let dst = LocalId::from(3u32);
        // Seeds all Any — demonstrates that Refine + Copy together refine
        // dst's type beyond the seed.
        func.locals.insert(src, mk_local(1, Type::Any));
        func.locals.insert(refined, mk_local(2, Type::Any));
        func.locals.insert(dst, mk_local(3, Type::Any));

        let bb0 = BlockId::from(0u32);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Const {
                dest: src,
                value: Constant::Int(7),
            },
            span: None,
        });
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Refine {
                dest: refined,
                src: Operand::Local(src),
                ty: Type::Int,
            },
            span: None,
        });
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Copy {
                dest: dst,
                src: Operand::Local(refined),
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(dst)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&dst), Some(&Type::Int));
    }

    /// S1.8b: `CallDirect` resolves the callee's declared return type from
    /// the enclosing module. Without a module, the rule is a no-op and
    /// the seed survives.
    #[test]
    fn call_direct_rule_picks_up_callee_return_type() {
        // Callee: f1() -> Float.
        let callee_id = FuncId::from(1u32);
        let mut callee = Function::new(callee_id, "f1".to_string(), Vec::new(), Type::Float, None);
        callee.block_mut(callee.entry_block).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Float(1.5))));

        // Caller: f0() calls f1 into a local with seed Any.
        let caller_id = FuncId::from(0u32);
        let mut caller = Function::new(caller_id, "f0".to_string(), Vec::new(), Type::Float, None);
        let dest = LocalId::from(1u32);
        caller.locals.insert(dest, mk_local(1, Type::Any));
        let bb0 = caller.entry_block;
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::CallDirect {
                dest,
                func: callee_id,
                args: Vec::new(),
            },
            span: None,
        });
        caller.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(dest)));
        caller.is_ssa = true;

        let mut module = Module::new();
        module.add_function(callee);
        module.add_function(caller);

        let types = infer_function(&module.functions[&caller_id], Some(&module));
        assert_eq!(types.get(&dest), Some(&Type::Float));

        // Without module, the rule is a no-op; seed (Any) survives.
        let types_no_mod = infer_function(&module.functions[&caller_id], None);
        assert_eq!(types_no_mod.get(&dest), Some(&Type::Any));
    }

    /// S1.8b: `GcAlloc` rule. Dest type comes from the instruction's
    /// explicit `ty` field — narrowing a loose seed.
    #[test]
    fn gc_alloc_rule_uses_explicit_ty_field() {
        let mut func = empty_func(Type::Int);
        let dest = LocalId::from(1u32);
        func.locals.insert(dest, mk_local(1, Type::Any));
        let bb0 = BlockId::from(0u32);
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::GcAlloc {
                dest,
                ty: Type::List(Box::new(Type::Int)),
                size: 16,
            },
            span: None,
        });
        func.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(dest)));

        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&dest), Some(&Type::List(Box::new(Type::Int))));
    }

    // ========================================================================
    // S1.11a: WPA parameter inference tests
    // ========================================================================

    /// Make a callee `f` with one unannotated (Any) parameter at LocalId 0.
    /// The body just returns the parameter so its return type surfaces via
    /// later `CallDirect → return_type` rule if needed.
    fn callee_one_any_param(id: u32, name: &str) -> Function {
        let param = Local {
            id: LocalId::from(0u32),
            name: None,
            ty: Type::Any,
            is_gc_root: false,
        };
        let mut f = Function::new(
            FuncId::from(id),
            name.to_string(),
            vec![param.clone()],
            Type::Any,
            None,
        );
        f.locals.insert(param.id, param.clone());
        f.block_mut(f.entry_block).terminator = Terminator::Return(Some(Operand::Local(param.id)));
        f.is_ssa = true;
        f
    }

    /// WPA narrows `f`'s unannotated param to the type observed at the
    /// single direct call site.
    #[test]
    fn wpa_narrows_single_call_site_arg() {
        let callee_id = FuncId::from(1u32);
        let caller_id = FuncId::from(0u32);

        let callee = callee_one_any_param(1, "callee");
        // Caller: `f(42)` — literal Int at the call site.
        let mut caller =
            Function::new(caller_id, "caller".to_string(), Vec::new(), Type::Int, None);
        let ret = LocalId::from(0u32);
        caller.locals.insert(ret, mk_local(0, Type::Int));
        let bb0 = caller.entry_block;
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::CallDirect {
                dest: ret,
                func: callee_id,
                args: vec![Operand::Constant(Constant::Int(42))],
            },
            span: None,
        });
        caller.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(ret)));
        caller.is_ssa = true;

        let mut module = Module::new();
        module.add_function(callee);
        module.add_function(caller);

        let cg = crate::call_graph::CallGraph::build(&module);
        let mut table = TypeTable::infer_module(&module);

        // Before WPA: callee's param stays at the Any seed.
        assert_eq!(table.get(callee_id, LocalId::from(0u32)), Some(&Type::Any));

        wpa_param_inference(&module, &cg, &mut table);

        // After WPA: param narrows to Int.
        assert_eq!(table.get(callee_id, LocalId::from(0u32)), Some(&Type::Int));
    }

    /// Multiple call sites with differing arg types → joined (Union via
    /// numeric tower or normalize_union) param type.
    #[test]
    fn wpa_joins_multiple_call_sites() {
        let callee_id = FuncId::from(2u32);
        let caller1_id = FuncId::from(0u32);
        let caller2_id = FuncId::from(1u32);

        let callee = callee_one_any_param(2, "callee");

        fn make_caller(id: FuncId, name: &str, arg: Constant, callee_id: FuncId) -> Function {
            let mut c = Function::new(id, name.to_string(), Vec::new(), Type::Int, None);
            let ret = LocalId::from(0u32);
            c.locals.insert(
                ret,
                Local {
                    id: ret,
                    name: None,
                    ty: Type::Int,
                    is_gc_root: false,
                },
            );
            let bb0 = c.entry_block;
            c.block_mut(bb0).instructions.push(Instruction {
                kind: InstructionKind::CallDirect {
                    dest: ret,
                    func: callee_id,
                    args: vec![Operand::Constant(arg)],
                },
                span: None,
            });
            c.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(ret)));
            c.is_ssa = true;
            c
        }

        let caller1 = make_caller(caller1_id, "c1", Constant::Int(1), callee_id);
        let caller2 = make_caller(caller2_id, "c2", Constant::Float(2.5), callee_id);

        let mut module = Module::new();
        module.add_function(callee);
        module.add_function(caller1);
        module.add_function(caller2);

        let cg = crate::call_graph::CallGraph::build(&module);
        let mut table = TypeTable::infer_module(&module);
        wpa_param_inference(&module, &cg, &mut table);

        // Int ⊔ Float via the numeric tower promotes to Float.
        assert_eq!(
            table.get(callee_id, LocalId::from(0u32)),
            Some(&Type::Float)
        );
    }

    /// Function with no direct callers keeps its seed (typical for entry
    /// points like `__pyaot_module_init__`).
    #[test]
    fn wpa_leaves_uncalled_function_at_seed() {
        let callee_id = FuncId::from(0u32);
        let callee = callee_one_any_param(0, "entry");

        let mut module = Module::new();
        module.add_function(callee);

        let cg = crate::call_graph::CallGraph::build(&module);
        let mut table = TypeTable::infer_module(&module);
        wpa_param_inference(&module, &cg, &mut table);

        assert_eq!(table.get(callee_id, LocalId::from(0u32)), Some(&Type::Any));
    }

    /// Recursive function in a self-loop SCC: param type is determined
    /// by the **external** caller, not the recursive self-call. Fixed
    /// point stabilises at the join of external sites.
    #[test]
    fn wpa_handles_recursive_scc() {
        // f(x) — self-recurses. Caller: main() → f(1).
        let f_id = FuncId::from(1u32);
        let main_id = FuncId::from(0u32);

        let param = Local {
            id: LocalId::from(0u32),
            name: None,
            ty: Type::Any,
            is_gc_root: false,
        };
        let self_ret = LocalId::from(1u32);
        let mut f = Function::new(f_id, "f".to_string(), vec![param.clone()], Type::Int, None);
        f.locals.insert(param.id, param.clone());
        f.locals.insert(self_ret, mk_local(1, Type::Int));
        let bb0 = f.entry_block;
        // Recursive self-call: `f(x)`.
        f.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::CallDirect {
                dest: self_ret,
                func: f_id,
                args: vec![Operand::Local(param.id)],
            },
            span: None,
        });
        f.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(self_ret)));
        f.is_ssa = true;

        // main() calls f(1) from outside the recursive SCC.
        let mut main_f = Function::new(main_id, "main".to_string(), Vec::new(), Type::Int, None);
        let main_ret = LocalId::from(0u32);
        main_f.locals.insert(main_ret, mk_local(0, Type::Int));
        let mbb = main_f.entry_block;
        main_f.block_mut(mbb).instructions.push(Instruction {
            kind: InstructionKind::CallDirect {
                dest: main_ret,
                func: f_id,
                args: vec![Operand::Constant(Constant::Int(1))],
            },
            span: None,
        });
        main_f.block_mut(mbb).terminator = Terminator::Return(Some(Operand::Local(main_ret)));
        main_f.is_ssa = true;

        let mut module = Module::new();
        module.add_function(f);
        module.add_function(main_f);

        let cg = crate::call_graph::CallGraph::build(&module);
        let mut table = TypeTable::infer_module(&module);
        wpa_param_inference(&module, &cg, &mut table);

        // External call passes Int; recursive call passes `x` which
        // WPA must resolve to Int (the join converges to Int).
        assert_eq!(table.get(f_id, LocalId::from(0u32)), Some(&Type::Int));
    }

    // ========================================================================
    // S1.8c: BinOp / UnOp rules
    // ========================================================================

    fn push_binop(
        func: &mut Function,
        dest: LocalId,
        op: pyaot_mir::BinOp,
        left: Operand,
        right: Operand,
    ) {
        let bb0 = func.entry_block;
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::BinOp {
                dest,
                op,
                left,
                right,
            },
            span: None,
        });
    }

    #[test]
    fn binop_add_two_ints_is_int() {
        let mut func = empty_func(Type::Int);
        let a = LocalId::from(0u32);
        let b = LocalId::from(1u32);
        let r = LocalId::from(2u32);
        func.locals.insert(a, mk_local(0, Type::Int));
        func.locals.insert(b, mk_local(1, Type::Int));
        func.locals.insert(r, mk_local(2, Type::Any));
        push_binop(
            &mut func,
            r,
            pyaot_mir::BinOp::Add,
            Operand::Local(a),
            Operand::Local(b),
        );
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Int));
    }

    #[test]
    fn binop_add_int_and_float_is_float() {
        let mut func = empty_func(Type::Float);
        let a = LocalId::from(0u32);
        let b = LocalId::from(1u32);
        let r = LocalId::from(2u32);
        func.locals.insert(a, mk_local(0, Type::Int));
        func.locals.insert(b, mk_local(1, Type::Float));
        func.locals.insert(r, mk_local(2, Type::Any));
        push_binop(
            &mut func,
            r,
            pyaot_mir::BinOp::Mul,
            Operand::Local(a),
            Operand::Local(b),
        );
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Float));
    }

    #[test]
    fn binop_division_always_produces_float() {
        let mut func = empty_func(Type::Float);
        let a = LocalId::from(0u32);
        let b = LocalId::from(1u32);
        let r = LocalId::from(2u32);
        func.locals.insert(a, mk_local(0, Type::Int));
        func.locals.insert(b, mk_local(1, Type::Int));
        func.locals.insert(r, mk_local(2, Type::Any));
        push_binop(
            &mut func,
            r,
            pyaot_mir::BinOp::Div,
            Operand::Local(a),
            Operand::Local(b),
        );
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Float));
    }

    #[test]
    fn binop_comparison_is_bool() {
        let mut func = empty_func(Type::Bool);
        let a = LocalId::from(0u32);
        let b = LocalId::from(1u32);
        let r = LocalId::from(2u32);
        func.locals.insert(a, mk_local(0, Type::Int));
        func.locals.insert(b, mk_local(1, Type::Int));
        func.locals.insert(r, mk_local(2, Type::Any));
        push_binop(
            &mut func,
            r,
            pyaot_mir::BinOp::Lt,
            Operand::Local(a),
            Operand::Local(b),
        );
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Bool));
    }

    #[test]
    fn unop_neg_preserves_numeric_type() {
        let mut func = empty_func(Type::Float);
        let a = LocalId::from(0u32);
        let r = LocalId::from(1u32);
        func.locals.insert(a, mk_local(0, Type::Float));
        func.locals.insert(r, mk_local(1, Type::Any));
        func.block_mut(func.entry_block)
            .instructions
            .push(Instruction {
                kind: InstructionKind::UnOp {
                    dest: r,
                    op: pyaot_mir::UnOp::Neg,
                    operand: Operand::Local(a),
                },
                span: None,
            });
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Float));
    }

    #[test]
    fn unop_not_produces_bool() {
        let mut func = empty_func(Type::Bool);
        let a = LocalId::from(0u32);
        let r = LocalId::from(1u32);
        func.locals.insert(a, mk_local(0, Type::Int));
        func.locals.insert(r, mk_local(1, Type::Any));
        func.block_mut(func.entry_block)
            .instructions
            .push(Instruction {
                kind: InstructionKind::UnOp {
                    dest: r,
                    op: pyaot_mir::UnOp::Not,
                    operand: Operand::Local(a),
                },
                span: None,
            });
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Bool));
    }

    // ========================================================================
    // RuntimeCall return-type rules
    // ========================================================================

    fn push_runtime_call(func: &mut Function, dest: LocalId, rt: pyaot_mir::RuntimeFunc) {
        let bb0 = func.entry_block;
        func.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::RuntimeCall {
                dest,
                func: rt,
                args: Vec::new(),
            },
            span: None,
        });
    }

    #[test]
    fn runtime_call_make_str_is_str() {
        let mut func = empty_func(Type::Str);
        let r = LocalId::from(0u32);
        func.locals.insert(r, mk_local(0, Type::Any));
        push_runtime_call(&mut func, r, pyaot_mir::RuntimeFunc::MakeStr);
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Str));
    }

    #[test]
    fn runtime_call_exc_has_exception_is_bool() {
        let mut func = empty_func(Type::Bool);
        let r = LocalId::from(0u32);
        func.locals.insert(r, mk_local(0, Type::Any));
        push_runtime_call(&mut func, r, pyaot_mir::RuntimeFunc::ExcHasException);
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Bool));
    }

    #[test]
    fn runtime_call_exc_isinstance_class_is_bool() {
        let mut func = empty_func(Type::Bool);
        let r = LocalId::from(0u32);
        func.locals.insert(r, mk_local(0, Type::Any));
        push_runtime_call(&mut func, r, pyaot_mir::RuntimeFunc::ExcIsinstanceClass);
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::Bool));
    }

    #[test]
    fn runtime_call_exc_get_current_is_heap_any() {
        let mut func = empty_func(Type::HeapAny);
        let r = LocalId::from(0u32);
        func.locals.insert(r, mk_local(0, Type::Any));
        push_runtime_call(&mut func, r, pyaot_mir::RuntimeFunc::ExcGetCurrent);
        func.block_mut(func.entry_block).terminator = Terminator::Return(Some(Operand::Local(r)));
        func.is_ssa = true;
        let types = infer_function(&func, None);
        assert_eq!(types.get(&r), Some(&Type::HeapAny));
    }

    /// Indirect `Call` where the function-pointer operand is defined by a
    /// `FuncAddr` in the same function — the trace-through rule resolves
    /// the callee and picks up its return type.
    #[test]
    fn call_indirect_via_func_addr_resolves_callee_return() {
        // Callee: f1() -> Str.
        let callee_id = FuncId::from(1u32);
        let mut callee = Function::new(callee_id, "f1".to_string(), Vec::new(), Type::Str, None);
        callee.block_mut(callee.entry_block).terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Int(0))));

        // Caller: addr = FuncAddr(f1); result = Call(addr, []).
        let caller_id = FuncId::from(0u32);
        let mut caller = Function::new(caller_id, "f0".to_string(), Vec::new(), Type::Str, None);
        let addr = LocalId::from(0u32);
        let result = LocalId::from(1u32);
        caller.locals.insert(addr, mk_local(0, Type::Any));
        caller.locals.insert(result, mk_local(1, Type::Any));
        let bb0 = caller.entry_block;
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::FuncAddr {
                dest: addr,
                func: callee_id,
            },
            span: None,
        });
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Call {
                dest: result,
                func: Operand::Local(addr),
                args: Vec::new(),
            },
            span: None,
        });
        caller.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(result)));
        caller.is_ssa = true;

        let mut module = Module::new();
        module.add_function(callee);
        module.add_function(caller);

        let types = infer_function(&module.functions[&caller_id], Some(&module));
        assert_eq!(types.get(&result), Some(&Type::Str));
    }

    /// The full-program fixed-point wrapper re-visits earlier SCCs when
    /// later SCC updates change their arg-type observations. Scenario:
    /// main() → mid(x) → leaf(y). All three are singleton SCCs in
    /// reverse-topo order [leaf, mid, main]. One pass of
    /// `wpa_param_inference`:
    ///   - main: no callers, keeps `Any` seed (no params anyway).
    ///   - mid: called by main with Int → `x: Int`. Re-infer mid — its
    ///     body calls leaf(x), so leaf's call-site arg is now Int.
    ///     But mid is processed AFTER leaf in reverse-topo, so leaf
    ///     was already visited with stale info.
    ///   - leaf: called by mid with the old `Any`-typed x → inferred
    ///     as `Any`.
    /// The whole-program fixed point fires a second outer iteration,
    /// re-visiting leaf with the now-refined mid.x → Int.
    #[test]
    fn wpa_full_program_fixed_point_refines_across_chain() {
        let main_id = FuncId::from(0u32);
        let mid_id = FuncId::from(1u32);
        let leaf_id = FuncId::from(2u32);

        // leaf(y) — returns y.
        let leaf_param = Local {
            id: LocalId::from(0u32),
            name: None,
            ty: Type::Any,
            is_gc_root: false,
        };
        let mut leaf = Function::new(
            leaf_id,
            "leaf".to_string(),
            vec![leaf_param.clone()],
            Type::Any,
            None,
        );
        leaf.locals.insert(leaf_param.id, leaf_param);
        leaf.block_mut(leaf.entry_block).terminator =
            Terminator::Return(Some(Operand::Local(LocalId::from(0u32))));
        leaf.is_ssa = true;

        // mid(x) — calls leaf(x), returns.
        let mid_param = Local {
            id: LocalId::from(0u32),
            name: None,
            ty: Type::Any,
            is_gc_root: false,
        };
        let mid_ret = LocalId::from(1u32);
        let mut mid = Function::new(
            mid_id,
            "mid".to_string(),
            vec![mid_param.clone()],
            Type::Any,
            None,
        );
        mid.locals.insert(mid_param.id, mid_param);
        mid.locals.insert(mid_ret, mk_local(1, Type::Any));
        mid.block_mut(mid.entry_block)
            .instructions
            .push(Instruction {
                kind: InstructionKind::CallDirect {
                    dest: mid_ret,
                    func: leaf_id,
                    args: vec![Operand::Local(LocalId::from(0u32))],
                },
                span: None,
            });
        mid.block_mut(mid.entry_block).terminator =
            Terminator::Return(Some(Operand::Local(mid_ret)));
        mid.is_ssa = true;

        // main() — calls mid(42).
        let main_ret = LocalId::from(0u32);
        let mut main_f = Function::new(main_id, "main".to_string(), Vec::new(), Type::Any, None);
        main_f.locals.insert(main_ret, mk_local(0, Type::Any));
        main_f
            .block_mut(main_f.entry_block)
            .instructions
            .push(Instruction {
                kind: InstructionKind::CallDirect {
                    dest: main_ret,
                    func: mid_id,
                    args: vec![Operand::Constant(Constant::Int(42))],
                },
                span: None,
            });
        main_f.block_mut(main_f.entry_block).terminator =
            Terminator::Return(Some(Operand::Local(main_ret)));
        main_f.is_ssa = true;

        let mut module = Module::new();
        module.add_function(leaf);
        module.add_function(mid);
        module.add_function(main_f);

        let cg = crate::call_graph::CallGraph::build(&module);
        let mut table = TypeTable::infer_module(&module);
        wpa_param_inference_to_fixed_point(&module, &cg, &mut table);

        // Both mid.x and leaf.y should converge to Int after the outer
        // fixed-point loop propagates main's Int arg through mid to leaf.
        assert_eq!(table.get(mid_id, LocalId::from(0u32)), Some(&Type::Int));
        assert_eq!(table.get(leaf_id, LocalId::from(0u32)), Some(&Type::Int));
    }

    /// Without a `Module` available, the rule is a no-op — the seed
    /// survives even when the `FuncAddr` def is present in the function.
    #[test]
    fn call_indirect_without_module_keeps_seed() {
        let caller_id = FuncId::from(0u32);
        let mut caller = Function::new(caller_id, "f0".to_string(), Vec::new(), Type::Any, None);
        let addr = LocalId::from(0u32);
        let result = LocalId::from(1u32);
        caller.locals.insert(addr, mk_local(0, Type::Any));
        caller.locals.insert(result, mk_local(1, Type::Any));
        let bb0 = caller.entry_block;
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::FuncAddr {
                dest: addr,
                func: FuncId::from(42u32),
            },
            span: None,
        });
        caller.block_mut(bb0).instructions.push(Instruction {
            kind: InstructionKind::Call {
                dest: result,
                func: Operand::Local(addr),
                args: Vec::new(),
            },
            span: None,
        });
        caller.block_mut(bb0).terminator = Terminator::Return(Some(Operand::Local(result)));
        caller.is_ssa = true;

        let types = infer_function(&caller, None);
        assert_eq!(types.get(&result), Some(&Type::Any));
    }
}
