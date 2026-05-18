//! Raw local demotion — cross-block box/unbox cancellation.
//!
//! Phase 5 of the Storage-Uniform refactor (Variant C, Step 2). Extends
//! the adjacent-pair box/unbox cancellation in `peephole` and
//! `box_fusion` across the SSA def-use graph.
//!
//! ## What it does
//!
//! For every box producer in the function — `BoxValue { src, src_type }`
//! or the legacy runtime-call form `rt_box_float(src)` — record the raw
//! source operand and the boxed primitive type. Then walk every unbox
//! consumer — `UnboxValue { src, dest_type }` or `rt_unbox_float(src)` —
//! and rewrite each one whose source local was produced by a matching
//! box (same primitive type) to a direct `Copy` of the original raw
//! operand. The orphaned `BoxValue` survives the rewrite untouched and
//! is reaped by `dce` immediately after.
//!
//! Box/Unbox round-trips are exact inverses for tagged primitives
//! (`(x << 3) | 1` ⇆ `x >> 3`; `rt_box_float` ⇆ `rt_unbox_float`), so
//! `unbox(box(x)) == x` whenever the types agree. Substituting the
//! original raw operand in place of the unbox is therefore semantically
//! sound.
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
//! - Does not trace through `Copy` / `Refine` / `Phi` chains. `Phi`
//!   merges that join multiple box producers (potentially with different
//!   `src_type`s) require a richer dataflow analysis; deferred.
//! - Does not re-type the box dest local. The local stays in
//!   `func.locals` until the surrounding `BoxValue` is removed by `dce`.
//! - Does not eliminate boxes whose dests still escape (storage write,
//!   ABI call, capture, return). Such boxes are necessary; only the
//!   redundant unbox uses are rewritten.
//!
//! ## Type-soundness
//!
//! Every rewrite is gated by exact `src_type == dest_type` between the
//! recorded box producer and the unbox consumer. `Type::None` is
//! intentionally not a candidate — `UnboxValue { dest_type: None }` is
//! not a valid construction (codegen rejects), so there is nothing to
//! cancel.

use indexmap::IndexMap;
use pyaot_core_defs::runtime_func_def::{RT_BOX_FLOAT, RT_UNBOX_FLOAT};
use pyaot_mir::{Function, InstructionKind, Module, Operand, RuntimeFunc};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId, StringInterner};

use crate::OptimizationPass;

/// Recorded box producer: `(raw operand, primitive type)`.
type BoxProducer = (Operand, Type);

/// Run raw-demotion once over every SSA function in the module.
/// Returns `true` if at least one unbox was rewritten.
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

/// Walk a function once: collect every box producer, then rewrite every
/// type-matching unbox consumer to `Copy` of the original raw operand.
fn demote_function(func: &mut Function) -> bool {
    let producers = collect_box_producers(func);
    if producers.is_empty() {
        return false;
    }
    rewrite_unbox_uses(func, &producers)
}

/// Scan every block once and record every box producer's raw source.
/// SSA discipline guarantees each `LocalId` has at most one definition.
fn collect_box_producers(func: &Function) -> IndexMap<LocalId, BoxProducer> {
    let mut producers = IndexMap::new();
    for block in func.blocks.values() {
        for inst in &block.instructions {
            match &inst.kind {
                InstructionKind::BoxValue {
                    dest,
                    src,
                    src_type,
                } if matches!(src_type, Type::Int | Type::Bool | Type::Float) => {
                    producers.insert(*dest, (src.clone(), src_type.clone()));
                }
                InstructionKind::RuntimeCall {
                    dest,
                    func: rt_func,
                    args,
                } if is_rt_box_float(rt_func) && args.len() == 1 => {
                    producers.insert(*dest, (args[0].clone(), Type::Float));
                }
                _ => {}
            }
        }
    }
    producers
}

/// Walk every block and rewrite `UnboxValue` / `rt_unbox_float`
/// instructions whose source local is a recorded box producer with a
/// matching primitive type. Each rewrite becomes `Copy(raw_src)`.
fn rewrite_unbox_uses(func: &mut Function, producers: &IndexMap<LocalId, BoxProducer>) -> bool {
    let mut changed = false;
    for block in func.blocks.values_mut() {
        for inst in &mut block.instructions {
            let replacement = match &inst.kind {
                InstructionKind::UnboxValue {
                    dest,
                    src: Operand::Local(src_local),
                    dest_type,
                } if matches!(dest_type, Type::Int | Type::Bool | Type::Float) => producers
                    .get(src_local)
                    .filter(|(_, ty)| ty == dest_type)
                    .map(|(raw_src, _)| InstructionKind::Copy {
                        dest: *dest,
                        src: raw_src.clone(),
                    }),
                InstructionKind::RuntimeCall {
                    dest,
                    func: rt_func,
                    args,
                } if is_rt_unbox_float(rt_func) && args.len() == 1 => {
                    if let Operand::Local(src_local) = &args[0] {
                        producers
                            .get(src_local)
                            .filter(|(_, ty)| matches!(ty, Type::Float))
                            .map(|(raw_src, _)| InstructionKind::Copy {
                                dest: *dest,
                                src: raw_src.clone(),
                            })
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(kind) = replacement {
                inst.kind = kind;
                changed = true;
            }
        }
    }
    changed
}

fn is_rt_box_float(func: &RuntimeFunc) -> bool {
    matches!(func, RuntimeFunc::Call(def) if std::ptr::eq(*def, &RT_BOX_FLOAT))
}

fn is_rt_unbox_float(func: &RuntimeFunc) -> bool {
    matches!(func, RuntimeFunc::Call(def) if std::ptr::eq(*def, &RT_UNBOX_FLOAT))
}

/// Pass wrapper for the optimization pipeline.
pub struct RawLocalDemotionPass;

impl OptimizationPass for RawLocalDemotionPass {
    fn name(&self) -> &str {
        "raw-demotion"
    }

    fn run_once(&mut self, module: &mut Module, _interner: &mut StringInterner) -> bool {
        raw_demotion_once(module)
    }

    fn is_fixpoint(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
