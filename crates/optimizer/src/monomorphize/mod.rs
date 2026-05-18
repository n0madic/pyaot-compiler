//! Monomorphization pass for generic MIR functions.
//!
//! Replaces every `CallDirect` to a generic template with a call to a
//! per-call-site specialization: a cloned copy of the template with all
//! `Type::Var` leaves substituted by the concrete types observed at that
//! call site (as propagated by WPA).
//!
//! After this pass, the module must contain zero `Type::Var` leaves in any
//! `Local::ty`, `Function::return_type`, or `Function::params[*].ty`.
//!
//! Pass placement: **after** first WPA (so call-arg types are concrete)
//! and **before** `abi_repair` (so the repaired ABI is computed on concrete
//! types).

mod clone;
mod devirt_indirect;

use std::collections::{HashSet, VecDeque};

use pyaot_mir::{Constant, Function, InstructionKind, Module, Operand};
use pyaot_types::{derive_subst, Type};
use pyaot_utils::{FuncId, StringInterner};

use crate::pass::OptimizationPass;

use self::clone::specialize_function;

/// Maximum specialization depth to prevent infinite recursion on recursive generics.
const MAX_SPECIALIZATION_DEPTH: usize = 8;

/// Cache key for a specialisation. Two structurally distinct kinds:
/// - `Generic`: Var-based specialisation (S3.3a/b.1) — keyed on the
///   template FuncId plus the concrete arg-type vector at the call site.
/// - `Wrapper`: structural specialisation of decorator wrappers (S3.3b.2)
///   — keyed on `(wrapper_id, captured_func_id)`. The captured-function
///   identity is the only axis of variation; argument types are derived
///   from the captured function's signature, not the call site.
#[derive(Debug, Clone, PartialEq)]
enum SpecKey {
    Generic(FuncId, Vec<Type>),
    Wrapper(FuncId, FuncId),
}

/// Patch produced by `collect_*_call_patches`. Carries enough information
/// for the worklist driver to clone the template and rewrite the call site.
#[derive(Debug, Clone)]
enum SpecPatch {
    /// Var-substitution specialisation. `arg_types` are the concrete types
    /// observed at the call site for each parameter slot; `derive_subst`
    /// turns them into a Var→Type map.
    Generic {
        block_idx: usize,
        instr_idx: usize,
        template_id: FuncId,
        arg_types: Vec<Type>,
    },
    /// Decorator-wrapper specialisation. `captured_func_id` was recovered
    /// by backward-tracing the fn-ptr argument through `ValueFromInt` to
    /// its `FuncAddr` source.
    Wrapper {
        block_idx: usize,
        instr_idx: usize,
        wrapper_id: FuncId,
        fn_ptr_param_idx: usize,
        captured_func_id: FuncId,
    },
}

pub struct MonomorphizePass;

impl OptimizationPass for MonomorphizePass {
    fn name(&self) -> &str {
        "monomorphize"
    }

    fn run_once(&mut self, module: &mut Module, interner: &mut StringInterner) -> bool {
        run(module, interner)
    }

    fn max_iterations(&self) -> usize {
        1
    }

    fn is_fixpoint(&self) -> bool {
        false
    }
}

/// Entry point for CLI — monomorphize all generic templates in the module.
pub fn run(module: &mut Module, interner: &mut StringInterner) -> bool {
    monomorphize_module(module, interner)
}

/// Returns `true` if any specializations were created.
fn monomorphize_module(module: &mut Module, interner: &mut StringInterner) -> bool {
    // --- Pre-pass: identify templates and monomorphic callers ---
    // Var-based templates (S3.3a/b.1).
    let generic_templates: HashSet<FuncId> = module
        .functions
        .values()
        .filter(|f| f.is_generic_template)
        .map(|f| f.id)
        .collect();
    // Wrapper templates (S3.3b.2): structural marker on decorator wrappers.
    let wrapper_templates: HashSet<FuncId> = module
        .functions
        .values()
        .filter(|f| f.wrapper_fn_ptr_capture_index.is_some())
        .map(|f| f.id)
        .collect();

    if generic_templates.is_empty() && wrapper_templates.is_empty() {
        return false;
    }

    let monomorphic_callers: Vec<FuncId> = module
        .functions
        .values()
        .filter(|f| !f.is_generic_template)
        .map(|f| f.id)
        .collect();

    // --- Specialization cache: (SpecKey, FuncId) pairs ---
    // Type doesn't implement Hash (f64 variant), so we use a Vec with linear search.
    // N is small (one entry per unique template+arg-type combo), so O(N) is fine.
    let mut spec_cache: Vec<(SpecKey, FuncId)> = Vec::new();
    // Monotone counter for fresh FuncIds (start above max existing id).
    let mut next_id: u32 = module
        .functions
        .keys()
        .map(|id| id.0 + 1)
        .max()
        .unwrap_or(0);

    // Worklist: (func_id, specialization_depth).
    let mut worklist: VecDeque<(FuncId, usize)> =
        monomorphic_callers.into_iter().map(|id| (id, 0)).collect();

    // Accumulated new specializations to add to module after the worklist drains.
    let mut changed = false;

    // Process callers; re-enqueue any caller where we made progress so chained
    // specializations (e.g. `b.rebox().get()` — `rebox` specialization concretizes
    // `b2`'s type, which then unblocks `get`'s specialization) iterate to fixpoint.
    while let Some((caller_id, depth)) = worklist.pop_front() {
        let mut patches = collect_template_call_patches(module, caller_id, &generic_templates);
        patches.extend(collect_wrapper_call_patches(
            module,
            caller_id,
            &wrapper_templates,
        ));
        if patches.is_empty() {
            continue;
        }

        let mut progress = false;
        for patch in patches {
            if depth >= MAX_SPECIALIZATION_DEPTH {
                eprintln!(
                    "monomorphize: depth limit reached on patch {patch:?} — \
                     call site will retain generic body"
                );
                continue;
            }

            let (block_idx, instr_idx, spec_key, spec_id_opt) =
                match resolve_patch(module, &patch, &mut spec_cache, &mut next_id, interner) {
                    Some(v) => v,
                    None => continue,
                };

            // If a fresh specialization was created, add it to the module
            // immediately (NOT to a deferred new_functions Vec) so the
            // worklist's next pop can find this spec via
            // `module.functions.get(&spec_id)` and chain-specialize any
            // template calls in its body. Pushing to a deferred Vec — the
            // prior shape — caused `collect_template_call_patches` to
            // return empty for the popped spec id (caller not found),
            // leaving the inner template calls unspecialized. Phase 3a-4.
            if let Some(spec_func) = spec_id_opt {
                let spec_id = spec_func.id;
                module.add_function(spec_func);
                worklist.push_back((spec_id, depth + 1));
                changed = true;
            }

            // The specialization id (cached or just-created).
            let spec_id = spec_cache
                .iter()
                .find(|(k, _)| k == &spec_key)
                .map(|(_, id)| *id)
                .expect("spec_id was just inserted");

            // The specialization's concrete return type — used both to rewrite
            // the call-site `func` and to retype the caller's `dest` local so
            // chained method calls on that local can be specialized in the
            // same caller pass. Specs are now in `module.functions`
            // immediately (Phase 3a-4 fix); the prior fallback through a
            // deferred `new_functions` Vec is gone.
            let spec_return_type = module
                .functions
                .get(&spec_id)
                .map(|f| f.return_type.clone())
                .unwrap_or(Type::Any);

            // Rewrite the call site in the caller and update the dest local's type.
            let caller = module.functions.get_mut(&caller_id).expect("caller exists");
            let block = caller
                .blocks
                .values_mut()
                .nth(block_idx)
                .expect("block exists");
            let instr = &mut block.instructions[instr_idx];
            if let InstructionKind::CallDirect { dest, func, .. } = &mut instr.kind {
                let dest_local = *dest;
                *func = spec_id;
                if let Some(local) = caller.locals.get_mut(&dest_local) {
                    if local.ty.contains_var() || local.ty != spec_return_type {
                        local.ty = spec_return_type.clone();
                        // Phase 3a-1 (Strong-Typed MIR): keep parallel
                        // mir_ty in sync with the narrowed ty. Without
                        // this, the lowering-emitted mir_ty (originally
                        // a Var translation) persists and downstream
                        // passes / verifier see a stale Var-typed slot
                        // even after specialization.
                        local.mir_ty =
                            Some(pyaot_mir::type_to_mir_type_register(&spec_return_type));
                        progress = true;
                    }
                }
            }
        }

        if progress {
            // Propagate updated dest-local types forward through `Copy { dest,
            // src: Local(s) }` chains so aliased variables see the concrete
            // type before the next iteration's patch collection.
            propagate_copy_types(module, caller_id);
            // Re-enqueue caller: chained specializations may have unblocked
            // patches that were skipped (arg type was Var on the previous pass).
            worklist.push_back((caller_id, depth));
        }
    }

    // --- Post-pass: purge zero-caller templates ---
    // Build caller → callee reference set.
    let mut callee_refs: HashSet<FuncId> = HashSet::new();
    for func in module.functions.values() {
        if func.is_generic_template {
            continue; // templates don't call each other in S3.3a
        }
        for block in func.blocks.values() {
            for instr in &block.instructions {
                if let InstructionKind::CallDirect { func: callee, .. } = &instr.kind {
                    callee_refs.insert(*callee);
                }
            }
        }
    }
    // Vtables reference template method FuncIds directly (pre-mono devirt
    // may have resolved CallVirtual → CallDirect(template), but the vtable
    // entry itself is never rewritten). Keep such templates alive so codegen
    // and devirt post-pass can still see them.
    let vtable_refs: HashSet<FuncId> = module
        .vtables
        .iter()
        .flat_map(|vt| vt.entries.iter().map(|e| e.method_func_id))
        .collect();
    let templates_to_purge: Vec<FuncId> = generic_templates
        .iter()
        .filter(|id| !callee_refs.contains(id) && !vtable_refs.contains(id))
        .copied()
        .collect();
    for id in &templates_to_purge {
        module.functions.shift_remove(id);
    }

    changed
}

/// Resolve a `SpecPatch` into (block_idx, instr_idx, spec_key, optional new
/// specialised function). Returns `None` if the patch can't be processed.
///
/// The cache is consulted; on a hit, returns the existing spec id. On a miss,
/// builds a fresh specialisation via `specialize_function` (Generic mode) or
/// `specialize_wrapper` (Wrapper mode), inserts it into the cache, and
/// returns the freshly-built `Function`.
fn resolve_patch(
    module: &Module,
    patch: &SpecPatch,
    spec_cache: &mut Vec<(SpecKey, FuncId)>,
    next_id: &mut u32,
    _interner: &mut StringInterner,
) -> Option<(usize, usize, SpecKey, Option<Function>)> {
    match patch {
        SpecPatch::Generic {
            block_idx,
            instr_idx,
            template_id,
            arg_types,
        } => {
            let template = module.functions.get(template_id)?;
            let param_types: Vec<Type> = template.params.iter().map(|p| p.ty.clone()).collect();
            let subst = derive_subst(&param_types, arg_types)?;
            let spec_key = SpecKey::Generic(*template_id, arg_types.clone());

            if spec_cache.iter().any(|(k, _)| k == &spec_key) {
                return Some((*block_idx, *instr_idx, spec_key, None));
            }

            let fresh_id = FuncId::from(*next_id);
            *next_id += 1;
            let base_name = template.name.clone();
            let type_suffix: Vec<String> = arg_types.iter().map(|t| format!("{t:?}")).collect();
            let fresh_name = format!("{}@<{}>", base_name, type_suffix.join(","));
            let specialized = specialize_function(template, &subst, fresh_id, fresh_name);
            spec_cache.push((spec_key.clone(), fresh_id));
            Some((*block_idx, *instr_idx, spec_key, Some(specialized)))
        }
        SpecPatch::Wrapper {
            block_idx,
            instr_idx,
            wrapper_id,
            fn_ptr_param_idx,
            captured_func_id,
        } => {
            let wrapper = module.functions.get(wrapper_id)?;
            let captured = module.functions.get(captured_func_id)?;
            let captured_signature = Type::Function {
                params: captured.params.iter().map(|p| p.ty.clone()).collect(),
                ret: Box::new(captured.return_type.clone()),
            };
            let spec_key = SpecKey::Wrapper(*wrapper_id, *captured_func_id);

            if spec_cache.iter().any(|(k, _)| k == &spec_key) {
                return Some((*block_idx, *instr_idx, spec_key, None));
            }

            let fresh_id = FuncId::from(*next_id);
            *next_id += 1;
            let base_name = wrapper.name.clone();
            let captured_name = captured.name.clone();
            let fresh_name = format!("{base_name}@<{captured_name}>");
            let mut specialized = clone::specialize_wrapper(
                wrapper,
                *fn_ptr_param_idx,
                *captured_func_id,
                captured_signature,
                fresh_id,
                fresh_name,
            );
            // Devirt the runtime trampoline calls inside the body now that
            // the captured target is statically known. This converts
            // `rt_call_with_tuple_args` into `CallDirect{captured_id, …}`
            // and lets WPA pass 2 propagate the precise return type.
            // Devirt is skipped when wrapper arity differs from captured
            // (e.g. `*args` packing) — the runtime trampoline keeps the
            // call correct in that case.
            let captured_param_types: Vec<Type> =
                captured.params.iter().map(|p| p.ty.clone()).collect();
            let did_devirt = devirt_indirect::devirt_wrapper_indirect_calls(
                &mut specialized,
                *fn_ptr_param_idx,
                *captured_func_id,
                &captured_param_types,
            );
            // Tighten return type only when devirt actually wired the call to
            // the captured target — otherwise the indirect trampoline still
            // returns `Any` and downstream typing must reflect that.
            if did_devirt {
                specialized.return_type = captured.return_type.clone();
            }
            spec_cache.push((spec_key.clone(), fresh_id));
            Some((*block_idx, *instr_idx, spec_key, Some(specialized)))
        }
    }
}

/// Collect Var-template patches for all `CallDirect` instructions in
/// `caller_id` that target a Var-based generic template. Argument types
/// are resolved from the caller's local type map.
fn collect_template_call_patches(
    module: &Module,
    caller_id: FuncId,
    templates: &HashSet<FuncId>,
) -> Vec<SpecPatch> {
    let caller = match module.functions.get(&caller_id) {
        Some(f) => f,
        None => return Vec::new(),
    };

    let mut patches = Vec::new();
    for (block_idx, block) in caller.blocks.values().enumerate() {
        for (instr_idx, instr) in block.instructions.iter().enumerate() {
            if let InstructionKind::CallDirect { func, args, .. } = &instr.kind {
                if !templates.contains(func) {
                    continue;
                }
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|op| match op {
                        Operand::Local(id) => caller
                            .locals
                            .get(id)
                            .map(|l| l.ty.clone())
                            .unwrap_or(Type::Any),
                        Operand::Constant(c) => const_type(c),
                    })
                    .collect();
                // Skip if any arg type is still a Var (caller is unresolved).
                // The fixpoint loop in `monomorphize_module` retries this
                // caller after chained specializations concretize its locals.
                if arg_types.iter().any(|t| t.contains_var()) {
                    continue;
                }
                patches.push(SpecPatch::Generic {
                    block_idx,
                    instr_idx,
                    template_id: *func,
                    arg_types,
                });
            }
        }
    }
    patches
}

/// Collect wrapper-template patches for all `CallDirect` instructions in
/// `caller_id` that target a decorator-wrapper function whose fn-pointer
/// argument can be statically traced to a `FuncAddr` source.
///
/// Pattern recognised (mirror of `lower_wrapper_call:118-127`):
/// ```text
/// %raw   = FuncAddr { func: original_id }
/// %fnptr = ValueFromInt { src: %raw }
/// %dest  = CallDirect { func: wrapper_id, args: [%fnptr, ...] }
/// ```
/// All three instructions are required to live in the **same block** as the
/// `CallDirect` for the trace to succeed. Calls whose fn-pointer originates
/// elsewhere (parameter, dynamic dispatch, cross-block phi) are skipped —
/// the wrapper retains its generic body and runtime trampoline.
fn collect_wrapper_call_patches(
    module: &Module,
    caller_id: FuncId,
    wrapper_templates: &HashSet<FuncId>,
) -> Vec<SpecPatch> {
    let caller = match module.functions.get(&caller_id) {
        Some(f) => f,
        None => return Vec::new(),
    };

    let mut patches = Vec::new();
    for (block_idx, block) in caller.blocks.values().enumerate() {
        for (instr_idx, instr) in block.instructions.iter().enumerate() {
            if let InstructionKind::CallDirect { func, args, .. } = &instr.kind {
                if !wrapper_templates.contains(func) {
                    continue;
                }
                let wrapper = match module.functions.get(func) {
                    Some(f) => f,
                    None => continue,
                };
                let Some(fn_ptr_idx) = wrapper.wrapper_fn_ptr_capture_index else {
                    continue;
                };
                let fnptr_arg = match args.get(fn_ptr_idx) {
                    Some(Operand::Local(id)) => *id,
                    _ => continue,
                };
                let Some(captured_id) =
                    find_funcaddr_source(block.instructions.as_slice(), instr_idx, fnptr_arg)
                else {
                    continue;
                };
                patches.push(SpecPatch::Wrapper {
                    block_idx,
                    instr_idx,
                    wrapper_id: *func,
                    fn_ptr_param_idx: fn_ptr_idx,
                    captured_func_id: captured_id,
                });
            }
        }
    }
    patches
}

/// Backward-trace from `target_local` (used as an argument at
/// `instructions[call_idx]`) up the same block looking for a
/// `FuncAddr → ValueFromInt → ...` chain. Returns the captured FuncId on
/// success, `None` if the producer is not a static `FuncAddr`.
fn find_funcaddr_source(
    instructions: &[pyaot_mir::Instruction],
    call_idx: usize,
    target_local: pyaot_utils::LocalId,
) -> Option<FuncId> {
    // Walk backward from just before `call_idx`. Track which local we're
    // looking for; when we see a `ValueFromInt { dest, src: Local(L) }` that
    // writes to it, switch to looking for `L`'s producer; when we see a
    // `FuncAddr { dest, func }` that writes to it, return `func`.
    let mut needle = target_local;
    for i in (0..call_idx).rev() {
        let inst = &instructions[i];
        match &inst.kind {
            InstructionKind::BoxValue {
                dest,
                src: Operand::Local(s),
                ..
            } if *dest == needle => {
                needle = *s;
            }
            InstructionKind::FuncAddr { dest, func } if *dest == needle => {
                return Some(*func);
            }
            // Any other write to `needle` defeats the trace (the producer
            // is not the simple FuncAddr→ValueFromInt pattern).
            _ => {
                if writes_local(&inst.kind, needle) {
                    return None;
                }
            }
        }
    }
    None
}

/// Returns true if `kind` writes to the local `target` (its `dest` matches).
/// Used by `find_funcaddr_source` to abort the trace when an unrecognised
/// producer hits the needle local. Listed with the actual MIR variants as of
/// `crates/mir/src/instructions.rs`; variants without a `dest: LocalId`
/// (control flow, GC frame management) are unreachable here.
fn writes_local(kind: &InstructionKind, target: pyaot_utils::LocalId) -> bool {
    use InstructionKind as K;
    let dest = match kind {
        K::Const { dest, .. }
        | K::BinOp { dest, .. }
        | K::UnOp { dest, .. }
        | K::Call { dest, .. }
        | K::CallDirect { dest, .. }
        | K::CallNamed { dest, .. }
        | K::CallVirtual { dest, .. }
        | K::CallVirtualNamed { dest, .. }
        | K::FuncAddr { dest, .. }
        | K::BuiltinAddr { dest, .. }
        | K::RuntimeCall { dest, .. }
        | K::Copy { dest, .. }
        | K::GcAlloc { dest, .. }
        | K::FloatToInt { dest, .. }
        | K::BoolToInt { dest, .. }
        | K::IntToFloat { dest, .. }
        | K::FloatBits { dest, .. }
        | K::IntBitsToFloat { dest, .. }
        | K::BoxValue { dest, .. }
        | K::UnboxValue { dest, .. }
        | K::FloatAbs { dest, .. }
        | K::ExcGetType { dest, .. }
        | K::ExcHasException { dest, .. }
        | K::ExcGetCurrent { dest, .. }
        | K::ExcCheckType { dest, .. }
        | K::ExcCheckClass { dest, .. }
        | K::Refine { dest, .. }
        | K::Phi { dest, .. } => *dest,
        K::GcPush { .. }
        | K::GcPop
        | K::ExcPushFrame { .. }
        | K::ExcPopFrame
        | K::ExcClear
        | K::ExcStartHandling
        | K::ExcEndHandling => return false,
    };
    dest == target
}

/// Walk a single function in instruction order and propagate the source
/// local's type through every `Copy { dest, src: Local(s) }`. Iterates until
/// fixpoint within the function so multi-step aliasing chains converge.
///
/// Used by `monomorphize_module` after a specialization rewrites a CallDirect
/// dest local: subsequent `b2 = b.rebox()` style aliases must see the
/// concrete `Generic { args: [Int] }` type so chained method calls
/// (`b2.get()`) collect their arg types correctly on the next pass.
fn propagate_copy_types(module: &mut Module, caller_id: FuncId) {
    let Some(func) = module.functions.get_mut(&caller_id) else {
        return;
    };
    loop {
        let mut changed = false;
        let copies: Vec<(pyaot_utils::LocalId, pyaot_utils::LocalId)> = func
            .blocks
            .values()
            .flat_map(|b| {
                b.instructions.iter().filter_map(|inst| {
                    if let InstructionKind::Copy {
                        dest,
                        src: Operand::Local(s),
                    } = &inst.kind
                    {
                        Some((*dest, *s))
                    } else {
                        None
                    }
                })
            })
            .collect();
        for (dest, src) in copies {
            let src_ty = func.locals.get(&src).map(|l| l.ty.clone());
            if let (Some(src_ty), Some(dest_local)) = (src_ty, func.locals.get_mut(&dest)) {
                if dest_local.ty != src_ty && (dest_local.ty.contains_var() || src_ty != Type::Any)
                {
                    dest_local.ty = src_ty;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn const_type(c: &Constant) -> Type {
    match c {
        Constant::Int(_) => Type::Int,
        Constant::Float(_) => Type::Float,
        Constant::Bool(_) => Type::Bool,
        Constant::Str(_) => Type::Str,
        Constant::Bytes(_) => Type::Bytes,
        Constant::None => Type::None,
    }
}

/// Panic if any non-template function in the module still contains `Type::Var`.
/// Call this after the second WPA pass, not immediately after monomorphize::run().
#[cfg(debug_assertions)]
pub fn assert_no_var_remaining(module: &Module) {
    for func in module.functions.values() {
        if func.is_generic_template {
            // Templates that still have callers (dynamic dispatch) are kept;
            // their bodies may still contain Var — that is allowed here.
            continue;
        }
        for param in &func.params {
            assert!(
                !param.ty.contains_var(),
                "monomorphize invariant: Var in param type of {} ({:?}): {:?}",
                func.name,
                func.id,
                param.ty
            );
        }
        assert!(
            !func.return_type.contains_var(),
            "monomorphize invariant: Var in return type of {} ({:?}): {:?}",
            func.name,
            func.id,
            func.return_type
        );
        for local in func.locals.values() {
            assert!(
                !local.ty.contains_var(),
                "monomorphize invariant: Var in local {:?} of {} ({:?}): {:?}",
                local.id,
                func.name,
                func.id,
                local.ty
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_core_defs::runtime_func_def::RT_CALL_WITH_TUPLE_ARGS;
    use pyaot_mir::{
        BasicBlock, Function as MirFunction, Instruction as MirInstruction, Local, RuntimeFunc,
        Terminator,
    };
    use pyaot_utils::{BlockId, LocalId};

    fn make_block_with_funcaddr_chain() -> Vec<MirInstruction> {
        // Mirrors lower_wrapper_call:118-127 layout:
        //   raw    = FuncAddr { func: 7 }
        //   tagged = BoxValue { src: raw, src_type: Int }
        //   _      = CallDirect { args: [tagged, ...] }
        vec![
            MirInstruction {
                kind: InstructionKind::FuncAddr {
                    dest: LocalId::from(10u32),
                    func: FuncId::from(7u32),
                },
                span: None,
            },
            MirInstruction {
                kind: InstructionKind::BoxValue {
                    dest: LocalId::from(11u32),
                    src: Operand::Local(LocalId::from(10u32)),
                    src_type: Type::Int,
                },
                span: None,
            },
            MirInstruction {
                kind: InstructionKind::CallDirect {
                    dest: LocalId::from(12u32),
                    func: FuncId::from(99u32),
                    args: vec![Operand::Local(LocalId::from(11u32))],
                },
                span: None,
            },
        ]
    }

    #[test]
    fn backward_trace_finds_funcaddr_through_box_value() {
        let instrs = make_block_with_funcaddr_chain();
        let captured = find_funcaddr_source(&instrs, 2, LocalId::from(11u32));
        assert_eq!(captured, Some(FuncId::from(7u32)));
    }

    #[test]
    fn backward_trace_returns_none_for_dynamic_fnptr() {
        // The fn-ptr was loaded via a Copy from an unrelated local.
        let instrs = vec![
            MirInstruction {
                kind: InstructionKind::Copy {
                    dest: LocalId::from(11u32),
                    src: Operand::Local(LocalId::from(99u32)),
                },
                span: None,
            },
            MirInstruction {
                kind: InstructionKind::CallDirect {
                    dest: LocalId::from(12u32),
                    func: FuncId::from(99u32),
                    args: vec![Operand::Local(LocalId::from(11u32))],
                },
                span: None,
            },
        ];
        let captured = find_funcaddr_source(&instrs, 1, LocalId::from(11u32));
        assert_eq!(captured, None);
    }

    #[test]
    fn collect_wrapper_patches_skips_non_wrapper_targets() {
        // A caller that issues a CallDirect to a non-wrapper function
        // produces no wrapper patches.
        let mut module = Module::new();
        // Caller function with one CallDirect to a func not in
        // wrapper_templates.
        let mut caller = MirFunction::new(
            FuncId::from(0u32),
            "caller".to_string(),
            Vec::new(),
            Type::None,
            None,
        );
        let block_id = caller.entry_block;
        caller.blocks.insert(
            block_id,
            BasicBlock {
                id: block_id,
                instructions: vec![MirInstruction {
                    kind: InstructionKind::CallDirect {
                        dest: LocalId::from(0u32),
                        func: FuncId::from(42u32),
                        args: vec![],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        module.add_function(caller);
        let wrapper_templates = HashSet::new();
        let patches = collect_wrapper_call_patches(&module, FuncId::from(0u32), &wrapper_templates);
        assert!(patches.is_empty());
    }

    #[test]
    fn spec_key_distinguishes_generic_from_wrapper() {
        let g = SpecKey::Generic(FuncId::from(1u32), vec![Type::Int]);
        let w = SpecKey::Wrapper(FuncId::from(1u32), FuncId::from(1u32));
        assert_ne!(g, w);
    }

    #[test]
    fn collect_wrapper_patches_finds_funcaddr_chain() {
        // Build a module with one caller and one wrapper-template func.
        let mut module = Module::new();

        let wrapper_id = FuncId::from(7u32);
        let mut wrapper = MirFunction::new(
            wrapper_id,
            "wrapper".to_string(),
            vec![Local {
                id: LocalId::from(0u32),
                name: None,
                ty: Type::Any,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            }],
            Type::Any,
            None,
        );
        wrapper.wrapper_fn_ptr_capture_index = Some(0);
        // Body unused by the test — but wrapper needs at least entry block,
        // already provided by Function::new.
        module.add_function(wrapper);

        // Caller: FuncAddr → ValueFromInt → CallDirect(wrapper, [tagged]).
        let caller_id = FuncId::from(0u32);
        let mut caller = MirFunction::new(
            caller_id,
            "caller".to_string(),
            Vec::new(),
            Type::None,
            None,
        );
        let block_id = caller.entry_block;
        caller.blocks.insert(
            block_id,
            BasicBlock {
                id: block_id,
                instructions: vec![
                    MirInstruction {
                        kind: InstructionKind::FuncAddr {
                            dest: LocalId::from(1u32),
                            func: FuncId::from(42u32),
                        },
                        span: None,
                    },
                    MirInstruction {
                        kind: InstructionKind::BoxValue {
                            dest: LocalId::from(2u32),
                            src: Operand::Local(LocalId::from(1u32)),
                            src_type: Type::Int,
                        },
                        span: None,
                    },
                    MirInstruction {
                        kind: InstructionKind::CallDirect {
                            dest: LocalId::from(3u32),
                            func: wrapper_id,
                            args: vec![Operand::Local(LocalId::from(2u32))],
                        },
                        span: None,
                    },
                ],
                terminator: Terminator::Return(None),
            },
        );
        // Caller needs locals registered for find_funcaddr_source to walk;
        // the trace itself is local-id-driven, so absence is fine.
        let _ = block_id;
        module.add_function(caller);

        let mut wrapper_templates = HashSet::new();
        wrapper_templates.insert(wrapper_id);
        let patches = collect_wrapper_call_patches(&module, caller_id, &wrapper_templates);
        assert_eq!(patches.len(), 1);
        match &patches[0] {
            SpecPatch::Wrapper {
                wrapper_id: wid,
                fn_ptr_param_idx,
                captured_func_id,
                ..
            } => {
                assert_eq!(*wid, wrapper_id);
                assert_eq!(*fn_ptr_param_idx, 0);
                assert_eq!(*captured_func_id, FuncId::from(42u32));
            }
            other => panic!("expected Wrapper patch, got {other:?}"),
        }
    }

    /// Compile-time check: ensure RT_CALL_WITH_TUPLE_ARGS is the symbol
    /// devirt looks for. A future renaming would silently bypass devirt
    /// without this guard.
    #[test]
    fn rt_call_with_tuple_args_symbol_is_stable() {
        assert_eq!(RT_CALL_WITH_TUPLE_ARGS.symbol, "rt_call_with_tuple_args");
        // Just to silence unused-import warnings on these helper types.
        let _ = RuntimeFunc::Call(&RT_CALL_WITH_TUPLE_ARGS);
        let _: BlockId = BlockId::from(0u32);
    }

    /// Phase 3a-4 regression: when a freshly-created specialization's
    /// body contains a `CallDirect` to another generic template, the
    /// monomorphizer must chain-specialize that inner call too. The
    /// bug was that fresh specs were accumulated in a deferred Vec and
    /// only inserted into `module.functions` AFTER the worklist drained;
    /// `collect_template_call_patches(module, fresh_spec_id, ...)`
    /// returned empty because the caller wasn't found in module.
    ///
    /// Repro pattern:
    ///   template_outer(v: T) -> T:
    ///       return template_inner(v)
    ///   template_inner(v: T) -> T:
    ///       return v
    ///   caller():
    ///       x: int = template_outer(42)  // triggers chain spec
    ///
    /// After monomorphize_module both `template_outer@<Int>` AND
    /// `template_inner@<Int>` must exist and the inner call inside the
    /// outer spec must point at `template_inner@<Int>` (not the
    /// template).
    #[test]
    fn chain_spec_through_nested_template_call() {
        let mut module = Module::new();
        let mut interner = pyaot_utils::StringInterner::new();
        let var_t = interner.intern("T");
        let var_ty = Type::Var(var_t);

        // template_inner(v: T) -> T { return v }
        let inner_id = FuncId::from(10u32);
        let mut inner = MirFunction::new(
            inner_id,
            "template_inner".to_string(),
            vec![Local {
                id: LocalId::from(0u32),
                name: None,
                ty: var_ty.clone(),
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            }],
            var_ty.clone(),
            None,
        );
        inner.is_generic_template = true;
        inner.typevar_params = vec![var_t];
        let entry = inner.entry_block;
        inner.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Local(LocalId::from(0u32)))),
            },
        );
        module.add_function(inner);

        // template_outer(v: T) -> T { return template_inner(v) }
        let outer_id = FuncId::from(11u32);
        let mut outer = MirFunction::new(
            outer_id,
            "template_outer".to_string(),
            vec![Local {
                id: LocalId::from(0u32),
                name: None,
                ty: var_ty.clone(),
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            }],
            var_ty.clone(),
            None,
        );
        outer.is_generic_template = true;
        outer.typevar_params = vec![var_t];
        let inner_dest = LocalId::from(1u32);
        outer.locals.insert(
            inner_dest,
            Local {
                id: inner_dest,
                name: None,
                ty: var_ty.clone(),
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = outer.entry_block;
        outer.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![MirInstruction {
                    kind: InstructionKind::CallDirect {
                        dest: inner_dest,
                        func: inner_id,
                        args: vec![Operand::Local(LocalId::from(0u32))],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(Some(Operand::Local(inner_dest))),
            },
        );
        module.add_function(outer);

        // caller(): x = template_outer(42)
        let caller_id = FuncId::from(12u32);
        let mut caller = MirFunction::new(
            caller_id,
            "caller".to_string(),
            Vec::new(),
            Type::None,
            None,
        );
        let dest = LocalId::from(0u32);
        caller.locals.insert(
            dest,
            Local {
                id: dest,
                name: None,
                ty: Type::Int,
                is_gc_root: false,
                abi_immutable: false,
                mir_ty: None,
            },
        );
        let entry = caller.entry_block;
        caller.blocks.insert(
            entry,
            BasicBlock {
                id: entry,
                instructions: vec![MirInstruction {
                    kind: InstructionKind::CallDirect {
                        dest,
                        func: outer_id,
                        args: vec![Operand::Constant(Constant::Int(42))],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        module.add_function(caller);

        monomorphize_module(&mut module, &mut interner);

        // After mono, both templates must have an Int specialization, AND
        // the inner call inside outer@<Int> must point at inner@<Int> (a
        // FuncId distinct from `inner_id` — the original template).
        let outer_spec = module
            .functions
            .values()
            .find(|f| f.name.starts_with("template_outer@<"))
            .expect("template_outer was not specialised");
        let inner_call_target = outer_spec
            .blocks
            .values()
            .flat_map(|b| b.instructions.iter())
            .find_map(|i| match &i.kind {
                InstructionKind::CallDirect { func, .. } => Some(*func),
                _ => None,
            })
            .expect("outer spec missing inner CallDirect");
        assert_ne!(
            inner_call_target, inner_id,
            "outer@<Int> still calls the template, chain-spec did not fire"
        );
        let inner_spec_present = module
            .functions
            .values()
            .any(|f| f.name.starts_with("template_inner@<"));
        assert!(
            inner_spec_present,
            "template_inner was not chain-specialised"
        );
    }
}
