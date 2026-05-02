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

use std::collections::{HashSet, VecDeque};

use pyaot_mir::{Constant, Function, InstructionKind, Module, Operand};
use pyaot_types::{derive_subst, Type};
use pyaot_utils::{FuncId, StringInterner};

use crate::pass::OptimizationPass;

use self::clone::specialize_function;

/// Maximum specialization depth to prevent infinite recursion on recursive generics.
const MAX_SPECIALIZATION_DEPTH: usize = 8;

/// Cache key: (template_func_id, concrete_arg_types_for_all_params).
type SpecKey = (FuncId, Vec<Type>);

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
    let templates: HashSet<FuncId> = module
        .functions
        .values()
        .filter(|f| f.is_generic_template)
        .map(|f| f.id)
        .collect();

    if templates.is_empty() {
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
    let mut new_functions: Vec<Function> = Vec::new();

    let mut changed = false;

    // Process callers; re-enqueue any caller where we made progress so chained
    // specializations (e.g. `b.rebox().get()` — `rebox` specialization concretizes
    // `b2`'s type, which then unblocks `get`'s specialization) iterate to fixpoint.
    while let Some((caller_id, depth)) = worklist.pop_front() {
        let patches = collect_template_call_patches(module, caller_id, &templates);
        if patches.is_empty() {
            continue;
        }

        let mut progress = false;
        for (block_idx, instr_idx, template_id, arg_types) in patches {
            if depth >= MAX_SPECIALIZATION_DEPTH {
                eprintln!(
                    "monomorphize: depth limit reached specializing {template_id:?} — \
                     call site will retain generic body"
                );
                continue;
            }

            // Build subst from template param types + call-arg types.
            let template = match module.functions.get(&template_id) {
                Some(f) => f,
                None => continue,
            };
            let param_types: Vec<Type> = template.params.iter().map(|p| p.ty.clone()).collect();
            let subst = match derive_subst(&param_types, &arg_types) {
                Some(s) => s,
                None => {
                    eprintln!(
                        "monomorphize: cannot derive subst for {template_id:?} with \
                         args {arg_types:?} — call site will use generic body"
                    );
                    continue;
                }
            };

            // Cache lookup.
            let spec_key: SpecKey = (template_id, arg_types);
            let spec_id = if let Some(&(_, cached_id)) =
                spec_cache.iter().find(|(k, _)| k == &spec_key)
            {
                cached_id
            } else {
                // Create specialization.
                let fresh_id = FuncId::from(next_id);
                next_id += 1;

                let base_name = module.functions[&template_id].name.clone();
                let type_suffix: Vec<String> =
                    spec_key.1.iter().map(|t| format!("{t:?}")).collect();
                let fresh_name = format!("{}@<{}>", base_name, type_suffix.join(","));
                let _ = interner; // available for future use

                let template_fn = &module.functions[&template_id];
                let specialized = specialize_function(template_fn, &subst, fresh_id, fresh_name);

                spec_cache.push((spec_key, fresh_id));
                new_functions.push(specialized);

                // Put the specialization on the worklist so its own template calls get resolved.
                worklist.push_back((fresh_id, depth + 1));

                changed = true;
                fresh_id
            };

            // The specialization's concrete return type — used both to rewrite
            // the call-site `func` and to retype the caller's `dest` local so
            // chained method calls on that local can be specialized in the
            // same caller pass.
            let spec_return_type = new_functions
                .iter()
                .find(|f| f.id == spec_id)
                .map(|f| f.return_type.clone())
                .or_else(|| {
                    module
                        .functions
                        .get(&spec_id)
                        .map(|f| f.return_type.clone())
                })
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
                        local.ty = spec_return_type;
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

    // Insert all new specializations.
    for func in new_functions {
        // Check any specialization we're about to add — it might itself be
        // a specialization that produced more specializations; add them too
        // by ensuring their blocks now reference fresh spec IDs.
        module.add_function(func);
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
    let templates_to_purge: Vec<FuncId> = templates
        .iter()
        .filter(|id| !callee_refs.contains(id) && !vtable_refs.contains(id))
        .copied()
        .collect();
    for id in &templates_to_purge {
        module.functions.shift_remove(id);
    }

    changed
}

/// Collect (block_index, instr_index, template_id, arg_types) for all
/// `CallDirect` instructions in `caller_id` that target a template.
///
/// Argument types are resolved from the caller's local type map.
fn collect_template_call_patches(
    module: &Module,
    caller_id: FuncId,
    templates: &HashSet<FuncId>,
) -> Vec<(usize, usize, FuncId, Vec<Type>)> {
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
                patches.push((block_idx, instr_idx, *func, arg_types));
            }
        }
    }
    patches
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
