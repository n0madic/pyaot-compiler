//! Box/Unbox Fusion — adjacent-pair cancellation for tagged Value box/unbox.
//!
//! Phase 6 of the Storage-Uniform refactor. Recovers a portion of the
//! performance lost when Phase 2 collapsed all class-field storage to
//! uniformly tagged `Value` slots: lowering now emits `BoxValue` /
//! `UnboxValue` round-trips at storage and ABI boundaries that were
//! previously absorbed by the `RT_INSTANCE_*_F64` fast path. This pass
//! cancels adjacent box/unbox pairs that the lowering pipeline introduces
//! mechanically — peephole already covers the `Int` / `Bool` MIR-form and
//! the pure-runtime-call form for `Float`; this pass closes the remaining
//! Float gaps.
//!
//! Patterns (block-local, adjacent instructions only):
//!
//! - `BoxValue { Float } + UnboxValue { Float }` → `Copy(box.src)`
//! - `UnboxValue { Float } + BoxValue { Float }` → `Copy(unbox.src)`
//! - Mixed-form (MIR ↔ runtime call) for `Float`:
//!   - `BoxValue { Float } + RT_UNBOX_FLOAT(...)` → `Copy(box.src)`
//!   - `RT_BOX_FLOAT(x) + UnboxValue { Float }` → `Copy(x)`
//!   - `UnboxValue { Float } + RT_BOX_FLOAT(unbox.dest)` → `Copy(unbox.src)`
//!   - `RT_UNBOX_FLOAT(x) + BoxValue { Float }` → `Copy(x)`
//!
//! TODO(Phase 6 follow-up): read-arith-write field cycle fusion. The
//! pattern
//!
//! ```text
//! v0 = rt_instance_get_field(obj, off)
//! v1 = UnboxValue(v0, Float)
//! ... pure arith on v1 ...
//! vn = BoxValue(vn-arith, Float)
//! rt_instance_set_field(obj, off, vn)
//! ```
//!
//! cannot be fused without runtime support that Phase 2 deliberately
//! removed (`RT_INSTANCE_GET/SET_FIELD_F64`). Recovery requires either
//! re-introducing raw-field fast paths with a tag-check elision, or a
//! codegen-time specialization that walks the field offset and reads/
//! writes the storage as raw bits — design TBD.

use indexmap::IndexMap;
use pyaot_core_defs::runtime_func_def::{RT_BOX_FLOAT, RT_UNBOX_FLOAT};
use pyaot_mir::{Function, Instruction, InstructionKind, Local, Module, Operand, RuntimeFunc};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId, StringInterner};

use crate::pass::OptimizationPass;

const MAX_ITERATIONS: usize = 10;

/// Run one box-fusion sweep over every function. Returns `true` if any
/// pair was cancelled.
pub(crate) fn box_fusion_once(module: &mut Module) -> bool {
    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();
    let mut changed = false;
    for func_id in func_ids {
        if let Some(func) = module.functions.get_mut(&func_id) {
            changed |= optimize_function(func);
        }
    }
    changed
}

/// Run box-fusion to a fixpoint (bounded by `MAX_ITERATIONS`).
pub fn run_box_fusion(module: &mut Module) {
    for _ in 0..MAX_ITERATIONS {
        if !box_fusion_once(module) {
            break;
        }
    }
}

fn optimize_function(func: &mut Function) -> bool {
    let mut changed = false;
    // Split borrow: `func.blocks` and `func.locals` are independent fields.
    let locals = &mut func.locals;
    for block in func.blocks.values_mut() {
        changed |= simplify_pairs(&mut block.instructions, locals);
    }
    changed
}

/// Walk adjacent instruction pairs in a block; replace the second
/// instruction of a cancellable pair with `Copy`. The first instruction
/// stays in place — DCE will reap it once its result becomes unused.
///
/// When a cancellation fires, propagate the source operand's declared
/// `ty` and `is_gc_root` to the Copy's dest local. The original
/// instruction (e.g. `BoxValue { dest, src_type: Float }`) declared
/// `dest` as the *box product* type (tagged Float). Once replaced by
/// `Copy(src)`, `dest` actually holds the source's bit pattern verbatim
/// — its declared type must follow, otherwise downstream passes
/// (`type_inference`, `abi_repair`) read stale metadata and emit
/// re-boxing (e.g. `rt_box_float`) that bitcasts the bits as f64.
fn simplify_pairs(instructions: &mut [Instruction], locals: &mut IndexMap<LocalId, Local>) -> bool {
    let mut changed = false;
    if instructions.len() < 2 {
        return false;
    }
    for i in 0..instructions.len() - 1 {
        let (first_kind, second_kind) = {
            let (left, right) = instructions.split_at(i + 1);
            (&left[i].kind, &right[0].kind)
        };
        if let Some(replacement) = match_float_pair(first_kind, second_kind) {
            if let InstructionKind::Copy {
                dest,
                src: Operand::Local(src_id),
            } = &replacement
            {
                let dest_id = *dest;
                let src_id = *src_id;
                if let Some(src_local) = locals.get(&src_id) {
                    let new_ty = src_local.ty.clone();
                    let new_is_gc_root = src_local.is_gc_root;
                    // Phase 3f audit: also propagate src's mir_ty when the
                    // fused Copy makes dest a bit-for-bit alias of src. The
                    // verifier's Copy check uses mir_ty (via resolved_mir_type)
                    // so leaving the dest's stale mir_ty in place would silently
                    // surface a type mismatch in later passes.
                    let new_mir_ty = src_local.mir_ty.clone();
                    if let Some(dest_local) = locals.get_mut(&dest_id) {
                        dest_local.ty = new_ty;
                        dest_local.is_gc_root = new_is_gc_root;
                        if new_mir_ty.is_some() {
                            dest_local.mir_ty = new_mir_ty;
                        }
                    }
                }
            }
            instructions[i + 1].kind = replacement;
            changed = true;
        }
    }
    changed
}

/// Try every Float box/unbox cancellation pattern. Returns the
/// replacement for the second instruction, or `None` if no pattern fires.
fn match_float_pair(first: &InstructionKind, second: &InstructionKind) -> Option<InstructionKind> {
    // BoxValue { Float } then UnboxValue { Float }
    if let (
        InstructionKind::BoxValue {
            dest: first_dest,
            src: orig_src,
            src_type: Type::Float,
        },
        InstructionKind::UnboxValue {
            dest: second_dest,
            src: inner_src,
            dest_type: Type::Float,
        },
    ) = (first, second)
    {
        if matches!(inner_src, Operand::Local(id) if *id == *first_dest) {
            return Some(InstructionKind::Copy {
                dest: *second_dest,
                src: orig_src.clone(),
            });
        }
    }

    // UnboxValue { Float } then BoxValue { Float }
    if let (
        InstructionKind::UnboxValue {
            dest: first_dest,
            src: orig_src,
            dest_type: Type::Float,
        },
        InstructionKind::BoxValue {
            dest: second_dest,
            src: inner_src,
            src_type: Type::Float,
        },
    ) = (first, second)
    {
        if matches!(inner_src, Operand::Local(id) if *id == *first_dest) {
            return Some(InstructionKind::Copy {
                dest: *second_dest,
                src: orig_src.clone(),
            });
        }
    }

    // BoxValue { Float } then rt_unbox_float(...)
    if let (
        InstructionKind::BoxValue {
            dest: first_dest,
            src: orig_src,
            src_type: Type::Float,
        },
        InstructionKind::RuntimeCall {
            dest: second_dest,
            func,
            args,
        },
    ) = (first, second)
    {
        if is_rt_unbox_float(func)
            && args.len() == 1
            && matches!(&args[0], Operand::Local(id) if *id == *first_dest)
        {
            return Some(InstructionKind::Copy {
                dest: *second_dest,
                src: orig_src.clone(),
            });
        }
    }

    // rt_box_float(x) then UnboxValue { Float }
    if let (
        InstructionKind::RuntimeCall {
            dest: first_dest,
            func,
            args,
        },
        InstructionKind::UnboxValue {
            dest: second_dest,
            src: inner_src,
            dest_type: Type::Float,
        },
    ) = (first, second)
    {
        if is_rt_box_float(func)
            && args.len() == 1
            && matches!(inner_src, Operand::Local(id) if *id == *first_dest)
        {
            return Some(InstructionKind::Copy {
                dest: *second_dest,
                src: args[0].clone(),
            });
        }
    }

    // UnboxValue { Float } then rt_box_float(...)
    if let (
        InstructionKind::UnboxValue {
            dest: first_dest,
            src: orig_src,
            dest_type: Type::Float,
        },
        InstructionKind::RuntimeCall {
            dest: second_dest,
            func,
            args,
        },
    ) = (first, second)
    {
        if is_rt_box_float(func)
            && args.len() == 1
            && matches!(&args[0], Operand::Local(id) if *id == *first_dest)
        {
            return Some(InstructionKind::Copy {
                dest: *second_dest,
                src: orig_src.clone(),
            });
        }
    }

    // rt_unbox_float(x) then BoxValue { Float }
    if let (
        InstructionKind::RuntimeCall {
            dest: first_dest,
            func,
            args,
        },
        InstructionKind::BoxValue {
            dest: second_dest,
            src: inner_src,
            src_type: Type::Float,
        },
    ) = (first, second)
    {
        if is_rt_unbox_float(func)
            && args.len() == 1
            && matches!(inner_src, Operand::Local(id) if *id == *first_dest)
        {
            return Some(InstructionKind::Copy {
                dest: *second_dest,
                src: args[0].clone(),
            });
        }
    }

    None
}

fn is_rt_box_float(func: &RuntimeFunc) -> bool {
    matches!(func, RuntimeFunc::Call(def) if std::ptr::eq(*def, &RT_BOX_FLOAT))
}

fn is_rt_unbox_float(func: &RuntimeFunc) -> bool {
    matches!(func, RuntimeFunc::Call(def) if std::ptr::eq(*def, &RT_UNBOX_FLOAT))
}

/// Pass wrapper for the optimization pipeline.
pub struct BoxFusionPass;

impl OptimizationPass for BoxFusionPass {
    fn name(&self) -> &str {
        "box-fusion"
    }

    fn run_once(&mut self, module: &mut Module, _interner: &mut StringInterner) -> bool {
        box_fusion_once(module)
    }

    fn max_iterations(&self) -> usize {
        MAX_ITERATIONS
    }
}

#[cfg(test)]
mod tests;
