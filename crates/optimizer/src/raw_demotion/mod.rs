//! Box/Unbox elision — unified cross-block cancellation pass.
//!
//! Single source of truth for tagged-Value box/unbox round-trip elimination.
//! Subsumes the legacy `peephole` box/unbox patterns (Int / Bool MIR + Float
//! runtime adjacent pairs) and the `box_fusion` Float-only adjacent-pair
//! pass. Both directions and any distance — including across `Copy` chains
//! and basic blocks — fold to a `Copy` of the underlying raw operand.
//!
//! ## Patterns recognised
//!
//! For `T ∈ {Int, Bool, Float}`:
//!
//! - **Forward (B→U)**:
//!   - `UnboxValue { dest, src: box_local, dest_type: T }` where
//!     `box_local` was defined by `BoxValue { src_type: T }` (any block) —
//!     rewritten to `Copy(box.src)`.
//!   - For `T == Float`, `rt_unbox_float` consumers of `rt_box_float` /
//!     `BoxValue { Float }` producers are also rewritten.
//! - **Reverse (U→B)**:
//!   - `BoxValue { dest, src: unbox_local, src_type: T }` where
//!     `unbox_local` was defined by `UnboxValue { dest_type: T }` (any
//!     block) — rewritten to `Copy(unbox.src)`.
//!   - For `T == Float`, mixed runtime / MIR forms are handled symmetrically.
//! - **Through Copy aliases**: a `Copy { dest: y, src: x }` whose `x` is
//!   a box/unbox producer makes `y` an equivalent producer; consumers of
//!   `y` are then rewritten to refer to the original raw operand.
//!
//! ## mir_ty propagation
//!
//! When a rewrite turns `BoxValue / UnboxValue → Copy`, the `dest` local's
//! declared type and `mir_ty` no longer match the new physical shape
//! (the box product type is shed). Without re-typing, downstream
//! `type_inference` / `abi_repair` read stale metadata and re-emit boxing
//! that bitcasts the bits incorrectly (e.g. `rt_box_float` of i64 bits).
//! On every rewrite we mirror the source local's `ty` and `mir_ty` onto
//! `dest` — load-bearing for float autograd (`microgpt.py`) and the
//! `RT_INSTANCE_*_F64` fast-path recovery this pass replaced.
//!
//! ## Type-soundness
//!
//! Every rewrite is gated by exact `src_type == dest_type` between the
//! recorded box/unbox producer and the consumer. Cross-type rewrites
//! (e.g. `box{Int} → rt_unbox_float`) are rejected: `rt_unbox_float`
//! is tag-dispatching and would coerce an Int-tagged input to f64
//! instead of bit-equal forwarding.
//!
//! ## SSA dependency
//!
//! Each `LocalId` must have a single definition for the producer map to
//! be reliable. Functions with `is_ssa == false` are skipped — they are
//! not produced by any path in the current pipeline (SSA construction
//! runs before optimization), but the guard keeps the pass safe under
//! future re-orderings.
//!
//! ## What it does NOT do
//!
//! - Does not trace through `Refine` / `Phi` chains. `Phi` merges that
//!   join multiple producers (potentially with different `src_type`s)
//!   require a richer dataflow analysis; deferred.
//! - Does not eliminate producers whose dests still escape (storage
//!   write, ABI call, capture, return). Such producers stay; only the
//!   redundant consumer instruction is rewritten. The dead producer is
//!   reaped by `dce`.

use indexmap::IndexMap;
use pyaot_core_defs::runtime_func_def::RT_UNBOX_FLOAT;
use pyaot_mir::{Function, InstructionKind, Module, Operand, RuntimeFunc};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId, StringInterner};

use crate::OptimizationPass;

/// Recorded box producer: `(raw operand, primitive type)`.
///
/// The raw operand is the input that was boxed. The primitive type is
/// the `src_type` declared on the originating `BoxValue` (or `Float` for
/// `rt_box_float`).
type BoxProducer = (Operand, Type);

/// Recorded unbox producer: `(boxed source operand, primitive type)`.
///
/// The boxed source operand is the tagged Value that was unboxed. The
/// primitive type is the `dest_type` declared on the originating
/// `UnboxValue` (or `Float` for `rt_unbox_float`).
type UnboxProducer = (Operand, Type);

/// Run box-elision over every SSA function in the module.
/// Returns `true` if at least one rewrite fired.
pub(crate) fn raw_demotion_once(module: &mut Module) -> bool {
    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();
    let mut changed = false;
    for func_id in func_ids {
        if let Some(func) = module.functions.get_mut(&func_id) {
            if !func.is_ssa {
                continue;
            }
            changed |= demote_function(func);
        }
    }
    changed
}

/// One sweep over a function: collect producers + Copy aliases, then
/// rewrite every matching consumer.
fn demote_function(func: &mut Function) -> bool {
    let (box_producers, unbox_producers) = collect_producers(func);
    if box_producers.is_empty() && unbox_producers.is_empty() {
        return false;
    }
    rewrite_consumers(func, &box_producers, &unbox_producers)
}

/// Scan every block and record every box/unbox producer's raw source.
/// Also propagate producer identity through `Copy { dest, src }` chains:
/// `dest` becomes an alias of `src`'s producer entry. SSA discipline
/// guarantees each `LocalId` has at most one definition, so a single
/// sweep in instruction order is enough to capture every chain (Copy's
/// `src` must be defined earlier in any SSA function).
fn collect_producers(
    func: &Function,
) -> (
    IndexMap<LocalId, BoxProducer>,
    IndexMap<LocalId, UnboxProducer>,
) {
    let mut box_producers: IndexMap<LocalId, BoxProducer> = IndexMap::new();
    let mut unbox_producers: IndexMap<LocalId, UnboxProducer> = IndexMap::new();

    for block in func.blocks.values() {
        for inst in &block.instructions {
            match &inst.kind {
                InstructionKind::BoxValue {
                    dest,
                    src,
                    src_type,
                } if matches!(src_type, Type::Int | Type::Bool | Type::Float) => {
                    box_producers.insert(*dest, (src.clone(), src_type.clone()));
                }
                InstructionKind::RuntimeCall { dest, args, .. }
                    if inst.kind.boxed_primitive_type() == Some(Type::Float) && args.len() == 1 =>
                {
                    box_producers.insert(*dest, (args[0].clone(), Type::Float));
                }
                InstructionKind::UnboxValue {
                    dest,
                    src,
                    dest_type,
                } if matches!(dest_type, Type::Int | Type::Bool | Type::Float) => {
                    unbox_producers.insert(*dest, (src.clone(), dest_type.clone()));
                }
                InstructionKind::RuntimeCall {
                    dest,
                    func: rt_func,
                    args,
                } if is_rt_unbox_float(rt_func) && args.len() == 1 => {
                    unbox_producers.insert(*dest, (args[0].clone(), Type::Float));
                }
                // Copy aliases: `Copy { dest, src: Local(x) }` makes
                // `dest` an alias of `x`'s producer record (whichever
                // map `x` is in). This is the cross-Copy tracing that
                // lets adjacent-pair tests with an intermediate `Copy`
                // still fuse.
                InstructionKind::Copy {
                    dest,
                    src: Operand::Local(src_local),
                } => {
                    if let Some(producer) = box_producers.get(src_local).cloned() {
                        box_producers.insert(*dest, producer);
                    }
                    if let Some(producer) = unbox_producers.get(src_local).cloned() {
                        unbox_producers.insert(*dest, producer);
                    }
                }
                _ => {}
            }
        }
    }

    (box_producers, unbox_producers)
}

/// Walk every block and rewrite consumer instructions whose source local
/// is a recorded producer with a matching primitive type. Each rewrite
/// becomes `Copy(raw_src)`; the dest local's `ty` / `mir_ty` is updated
/// to match the new physical shape.
fn rewrite_consumers(
    func: &mut Function,
    box_producers: &IndexMap<LocalId, BoxProducer>,
    unbox_producers: &IndexMap<LocalId, UnboxProducer>,
) -> bool {
    let mut changed = false;
    // Pending dest-local retypings — applied after the instruction walk
    // to keep the borrow on `func.locals` separate from the walk on
    // `func.blocks`.
    let mut retypings: Vec<(LocalId, Option<Type>, Option<pyaot_mir::MirType>)> = Vec::new();

    for block in func.blocks.values_mut() {
        for inst in &mut block.instructions {
            let replacement = match &inst.kind {
                // Forward: UnboxValue(box_local, T) → Copy(box.src)
                InstructionKind::UnboxValue {
                    dest,
                    src: Operand::Local(src_local),
                    dest_type,
                } if matches!(dest_type, Type::Int | Type::Bool | Type::Float) => box_producers
                    .get(src_local)
                    .filter(|(_, ty)| ty == dest_type)
                    .map(|(raw_src, _)| {
                        (
                            InstructionKind::Copy {
                                dest: *dest,
                                src: raw_src.clone(),
                            },
                            *dest,
                            raw_src.clone(),
                        )
                    }),
                // Forward: rt_unbox_float(box_local) → Copy(box.src)
                InstructionKind::RuntimeCall {
                    dest,
                    func: rt_func,
                    args,
                } if is_rt_unbox_float(rt_func) && args.len() == 1 => {
                    if let Operand::Local(src_local) = &args[0] {
                        box_producers
                            .get(src_local)
                            .filter(|(_, ty)| matches!(ty, Type::Float))
                            .map(|(raw_src, _)| {
                                (
                                    InstructionKind::Copy {
                                        dest: *dest,
                                        src: raw_src.clone(),
                                    },
                                    *dest,
                                    raw_src.clone(),
                                )
                            })
                    } else {
                        None
                    }
                }
                // Reverse: BoxValue(unbox_local, T) → Copy(unbox.src)
                InstructionKind::BoxValue {
                    dest,
                    src: Operand::Local(src_local),
                    src_type,
                } if matches!(src_type, Type::Int | Type::Bool | Type::Float) => unbox_producers
                    .get(src_local)
                    .filter(|(_, ty)| ty == src_type)
                    .map(|(raw_src, _)| {
                        (
                            InstructionKind::Copy {
                                dest: *dest,
                                src: raw_src.clone(),
                            },
                            *dest,
                            raw_src.clone(),
                        )
                    }),
                // Reverse: rt_box_float(unbox_local) → Copy(unbox.src)
                InstructionKind::RuntimeCall { dest, args, .. }
                    if inst.kind.boxed_primitive_type() == Some(Type::Float) && args.len() == 1 =>
                {
                    if let Operand::Local(src_local) = &args[0] {
                        unbox_producers
                            .get(src_local)
                            .filter(|(_, ty)| matches!(ty, Type::Float))
                            .map(|(raw_src, _)| {
                                (
                                    InstructionKind::Copy {
                                        dest: *dest,
                                        src: raw_src.clone(),
                                    },
                                    *dest,
                                    raw_src.clone(),
                                )
                            })
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some((kind, dest, raw_src)) = replacement {
                inst.kind = kind;
                changed = true;
                // Queue the dest retyping. Look up the new source type
                // from `func.locals` later — we can't borrow it here
                // while iterating `func.blocks`.
                if let Operand::Local(src_id) = raw_src {
                    retypings.push((dest, Some(Type::Never), None));
                    let _ = src_id;
                    // Real type lookup happens below; we just queue the dest.
                }
            }
        }
    }

    if !changed {
        return false;
    }

    // Phase 2: apply dest retypings using the now-rewritten Copy's src.
    // Walk the (already mutated) instructions again to recover each Copy's
    // src local and copy its declared `ty` / `mir_ty` to the dest. This is
    // the load-bearing `mir_ty` propagation called out at the top of the
    // module — without it, downstream passes treat the dest as the box
    // product type (tagged) and re-introduce boxing.
    propagate_copy_types(func, &retypings);

    changed
}

/// For every `dest` queued by `rewrite_consumers`, find the Copy that
/// now defines it, look up the source local's `ty` / `mir_ty`, and
/// propagate them onto `dest`. Skips cases where the Copy's src is a
/// `Constant` (no Local metadata to copy).
fn propagate_copy_types(
    func: &mut Function,
    retypings: &[(LocalId, Option<Type>, Option<pyaot_mir::MirType>)],
) {
    if retypings.is_empty() {
        return;
    }

    // Build a `dest → src_local` map from the rewritten Copy instructions.
    let mut copy_src: IndexMap<LocalId, LocalId> = IndexMap::new();
    for block in func.blocks.values() {
        for inst in &block.instructions {
            if let InstructionKind::Copy {
                dest,
                src: Operand::Local(src_local),
            } = &inst.kind
            {
                copy_src.insert(*dest, *src_local);
            }
        }
    }

    // Look up each source local's type metadata.
    let mut updates: Vec<(LocalId, Type, Option<pyaot_mir::MirType>)> = Vec::new();
    for (dest, _, _) in retypings {
        let Some(src_local_id) = copy_src.get(dest).copied() else {
            continue;
        };
        let Some(src_local) = func.locals.get(&src_local_id) else {
            continue;
        };
        updates.push((*dest, src_local.ty.clone(), src_local.mir_ty.clone()));
    }

    // Apply the updates.
    for (dest, new_ty, new_mir_ty) in updates {
        if let Some(dest_local) = func.locals.get_mut(&dest) {
            dest_local.ty = new_ty;
            if new_mir_ty.is_some() {
                dest_local.mir_ty = new_mir_ty;
            }
        }
    }
}

fn is_rt_unbox_float(func: &RuntimeFunc) -> bool {
    matches!(func, RuntimeFunc::Call(def) if std::ptr::eq(*def, &RT_UNBOX_FLOAT))
}

/// Pass wrapper for the optimization pipeline. Runs to fixpoint because
/// each rewrite may expose new opportunities through the Copy chains it
/// itself created.
pub struct RawLocalDemotionPass;

impl OptimizationPass for RawLocalDemotionPass {
    fn name(&self) -> &str {
        "raw-demotion"
    }

    fn run_once(&mut self, module: &mut Module, _interner: &mut StringInterner) -> bool {
        raw_demotion_once(module)
    }
}

#[cfg(test)]
mod tests;
