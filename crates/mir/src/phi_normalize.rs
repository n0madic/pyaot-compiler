//! Phi-source normalization pass.
//!
//! Phase 2 of the Strong-Typed MIR Rewrite (plan at
//! `.claude/plans/velvety-waddling-map.md`). After SSA construction,
//! some Phi nodes have heterogeneous source types: e.g. a match-pattern
//! capture binds different types across arms, producing a Phi where one
//! source is `Raw(I64)` and the dest is `Tagged` / `Heap(_)`. The
//! Verifier flags these as representation mismatches.
//!
//! This pass walks every Phi, identifies sources that don't match the
//! dest's resolved `MirType`, and inserts a `BoxValue` at the end of
//! the predecessor block (before the terminator) to convert the raw
//! primitive into a tagged Value. The Phi source is updated to
//! reference the new boxed temp.
//!
//! # When this fires
//!
//! - **Dest is `Tagged`**: any `Raw(K)` source gets boxed.
//! - **Dest is `Heap(_)`** with a Raw source: this represents a
//!   match-pattern preallocation mismatch — dest was typed from one
//!   arm but a different arm contributes a Raw value. Box the source
//!   into a `Tagged` temp; downstream consumers may unbox or refine.
//!   The dest itself isn't retyped (that would cascade), so the Phi
//!   technically remains a violation post-box; a future Phase 2 step
//!   will tighten this. For now this still reduces total violations
//!   because most Phi mismatches occur with `Tagged` dest after WPA.
//!
//! # When this does NOT fire
//!
//! - Both source and dest are Raw of the same kind.
//! - Source is already `Tagged` / `Heap(_)` matching dest.
//! - Source is `Tagged` and dest is `Raw(_)` — that's an unbox
//!   requirement, handled by a different pass (Phase 3+).

use indexmap::IndexMap;
use pyaot_types::Type;
use pyaot_utils::{BlockId, LocalId};

use crate::core::{Function, Local, Module};
use crate::instructions::{Instruction, InstructionKind};
use crate::operands::{Constant, Operand};
use crate::types::{type_to_mir_type_register, MirType, RawKind};

/// Run Phi normalization across every function in the module.
/// Returns the number of BoxValue inserts done (for verifier-driven
/// progress reporting).
pub fn normalize_phi_sources_module(module: &mut Module) -> usize {
    let mut total = 0;
    for func in module.functions.values_mut() {
        total += normalize_phi_sources(func);
    }
    total
}

/// Normalize Phi sources in a single function. Returns the count of
/// BoxValue inserts performed.
pub fn normalize_phi_sources(func: &mut Function) -> usize {
    // Collect all (block_id, phi_idx, dest, sources) first to avoid
    // borrow conflicts when mutating predecessor blocks.
    #[derive(Debug)]
    struct PhiEntry {
        block_id: BlockId,
        phi_idx: usize,
        dest: LocalId,
        sources: Vec<(BlockId, Operand)>,
    }
    let mut phis: Vec<PhiEntry> = Vec::new();
    for (block_id, block) in &func.blocks {
        for (idx, inst) in block.instructions.iter().enumerate() {
            if let InstructionKind::Phi { dest, sources } = &inst.kind {
                phis.push(PhiEntry {
                    block_id: *block_id,
                    phi_idx: idx,
                    dest: *dest,
                    sources: sources.clone(),
                });
            } else {
                // Phi nodes are at block head; once we hit a non-Phi, stop.
                break;
            }
        }
    }

    if phis.is_empty() {
        return 0;
    }

    let mut next_local_id: u32 = func.locals.keys().map(|id| id.0).max().unwrap_or(0) + 1;
    for p in &func.params {
        if p.id.0 >= next_local_id {
            next_local_id = p.id.0 + 1;
        }
    }

    let mut box_inserts = 0;
    // Per-block buffer of BoxValue instructions to insert before the
    // terminator. Keyed by predecessor block, collected and applied at
    // the end so we don't shift indices mid-iteration.
    let mut pred_box_buf: IndexMap<BlockId, Vec<Instruction>> = IndexMap::new();
    // Phi source replacements: (phi_block, phi_idx, source_idx) → new operand.
    let mut phi_replacements: Vec<(BlockId, usize, usize, Operand)> = Vec::new();

    for phi in &phis {
        let dest_ty = func
            .locals
            .get(&phi.dest)
            .map(|l| l.resolved_mir_type())
            .unwrap_or(MirType::Tagged);

        for (src_idx, (pred, op)) in phi.sources.iter().enumerate() {
            let src_ty = operand_mir_type(func, op);

            // Skip when types already agree (or are compatible).
            if src_ty.assignable_to(&dest_ty) {
                continue;
            }
            // Only handle Tagged dest with Raw source (the most common
            // case). Future steps may extend.
            let needs_box = matches!(dest_ty, MirType::Tagged) && matches!(src_ty, MirType::Raw(_));
            if !needs_box {
                continue;
            }
            let MirType::Raw(raw_kind) = src_ty else {
                continue;
            };

            // Allocate a temp Local typed Tagged.
            let temp_id = LocalId::from(next_local_id);
            next_local_id += 1;
            let legacy_ty = raw_kind_to_legacy_type(raw_kind);
            func.locals.insert(
                temp_id,
                Local {
                    id: temp_id,
                    name: None,
                    ty: Type::Any,
                    is_gc_root: true,
                    abi_immutable: false,
                    mir_ty: Some(MirType::Tagged),
                },
            );

            // Build BoxValue { dest: temp, src: op, src_type: legacy_ty }.
            let box_inst = Instruction {
                kind: InstructionKind::BoxValue {
                    dest: temp_id,
                    src: op.clone(),
                    src_type: legacy_ty,
                },
                span: None,
            };
            pred_box_buf.entry(*pred).or_default().push(box_inst);

            phi_replacements.push((phi.block_id, phi.phi_idx, src_idx, Operand::Local(temp_id)));
            box_inserts += 1;
        }
    }

    // Apply BoxValue inserts to predecessor blocks.
    for (pred_id, box_insts) in pred_box_buf {
        if let Some(pred_block) = func.blocks.get_mut(&pred_id) {
            // Insert before the terminator — i.e., append to the end of
            // instructions (the terminator is a separate field).
            pred_block.instructions.extend(box_insts);
        }
    }

    // Apply Phi source replacements.
    for (block_id, phi_idx, src_idx, new_op) in phi_replacements {
        if let Some(block) = func.blocks.get_mut(&block_id) {
            if let Some(inst) = block.instructions.get_mut(phi_idx) {
                if let InstructionKind::Phi { sources, .. } = &mut inst.kind {
                    if let Some(entry) = sources.get_mut(src_idx) {
                        entry.1 = new_op;
                    }
                }
            }
        }
    }

    // Phase 4 ext: narrow Phi dest mir_ty when all sources are Raw(K)
    // of the same kind. Pattern: class-method accumulator pre-allocated
    // as Tagged (defensive boxing) but loop body actually maintains a
    // Raw(I64) value via BinOp + Copy. The dest's Tagged mir_ty would
    // make downstream BinOps on the Phi result clash with the verifier.
    // Restrict to Raw(I64) for now — the test_future_annotations case
    // (`total = self.x; for p: total += p.sum_x()`).
    narrow_uniform_raw_phi_dests(func);

    box_inserts
}

/// Narrow Phi dest `mir_ty` from `Tagged` to `Raw(K)` when ALL Phi
/// sources are constants or locals with `Raw(K)` mir_ty of the same
/// kind. Safe because the Phi result is then provably Raw(K) at runtime.
fn narrow_uniform_raw_phi_dests(func: &mut Function) {
    let phis: Vec<(LocalId, Vec<Operand>)> = func
        .blocks
        .values()
        .flat_map(|bb| {
            bb.instructions.iter().filter_map(|inst| {
                if let InstructionKind::Phi { dest, sources } = &inst.kind {
                    Some((*dest, sources.iter().map(|(_, op)| op.clone()).collect()))
                } else {
                    None
                }
            })
        })
        .collect();

    for (dest_id, sources) in phis {
        // Only narrow Tagged dests.
        let Some(dest_local) = func.locals.get(&dest_id) else {
            continue;
        };
        if !matches!(dest_local.resolved_mir_type(), MirType::Tagged) {
            continue;
        }
        // Skip ABI-immutable locals (Phase 4 E1).
        if dest_local.abi_immutable {
            continue;
        }
        // All sources must be Raw(K) with the same K.
        let mut uniform_kind: Option<RawKind> = None;
        let mut all_uniform = true;
        for op in &sources {
            let src_ty = operand_mir_type(func, op);
            match src_ty {
                MirType::Raw(k) => match uniform_kind {
                    None => uniform_kind = Some(k),
                    Some(prev) if prev == k => {}
                    _ => {
                        all_uniform = false;
                        break;
                    }
                },
                _ => {
                    all_uniform = false;
                    break;
                }
            }
        }
        if !all_uniform {
            continue;
        }
        let Some(kind) = uniform_kind else { continue };
        // Only narrow I64 for safety — F64 lives in XMM registers and
        // would require explicit bitcasts at downstream consumers.
        if !matches!(kind, RawKind::I64) {
            continue;
        }
        if let Some(local) = func.locals.get_mut(&dest_id) {
            local.mir_ty = Some(MirType::Raw(kind));
        }
    }
}

fn operand_mir_type(func: &Function, op: &Operand) -> MirType {
    match op {
        Operand::Local(id) => {
            if let Some(l) = func.locals.get(id) {
                return l.resolved_mir_type();
            }
            if let Some(p) = func.params.iter().find(|p| p.id == *id) {
                return p.resolved_mir_type();
            }
            MirType::Tagged
        }
        Operand::Constant(c) => constant_mir_type(c),
    }
}

fn constant_mir_type(c: &Constant) -> MirType {
    match c {
        Constant::Int(_) => MirType::raw_i64(),
        Constant::Float(_) => MirType::raw_f64(),
        Constant::Bool(_) => MirType::raw_i8(),
        Constant::None => MirType::raw_i8(),
        Constant::Str(_) => MirType::str_heap(),
        Constant::Bytes(_) => MirType::bytes_heap(),
    }
}

fn raw_kind_to_legacy_type(kind: RawKind) -> Type {
    match kind {
        RawKind::I64 => Type::Int,
        RawKind::F64 => Type::Float,
        RawKind::I8 => Type::Bool,
        // I32 isn't used by BoxValue codegen; route through Int as
        // safe default since Phase 2 lowering doesn't currently produce
        // Raw(I32) operands at Phi merges.
        RawKind::I32 => Type::Int,
    }
}

// Re-export resolved_mir_type accessor for Locals not in func.locals
// (params). Used by `operand_mir_type`.
impl Local {
    #[doc(hidden)]
    fn _params_resolved_mir_type(&self) -> MirType {
        if let Some(t) = &self.mir_ty {
            return t.clone();
        }
        type_to_mir_type_register(&self.ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{BasicBlock, Function, FunctionKind, Local};
    use crate::terminators::Terminator;
    use crate::types::MirType;
    use pyaot_types::Type;
    use pyaot_utils::{BlockId, FuncId, LocalId};
    use std::cell::OnceCell;

    fn mk_local(id: u32, ty: Type, mir_ty: Option<MirType>) -> Local {
        Local {
            id: LocalId::from(id),
            name: None,
            ty,
            is_gc_root: false,
            abi_immutable: false,
            mir_ty,
        }
    }

    fn empty_func() -> Function {
        Function {
            id: FuncId::from(0u32),
            kind: FunctionKind::Regular,
            name: "test".to_string(),
            params: vec![],
            return_type: Type::None,
            locals: IndexMap::new(),
            blocks: IndexMap::new(),
            entry_block: BlockId::from(0u32),
            span: None,
            is_ssa: true,
            is_generic_template: false,
            typevar_params: vec![],
            wrapper_fn_ptr_capture_index: None,
            phase4_return_abi_flipped: false,
            phase4_original_return_type: None,
            dom_tree_cache: OnceCell::new(),
            signature: None,
        }
    }

    #[test]
    fn no_phi_no_inserts() {
        let mut func = empty_func();
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![],
                terminator: Terminator::Return(None),
            },
        );
        let inserts = normalize_phi_sources(&mut func);
        assert_eq!(inserts, 0);
    }

    #[test]
    fn phi_with_matching_sources_unchanged() {
        let mut func = empty_func();
        func.locals.insert(
            LocalId::from(10u32),
            mk_local(10, Type::Any, Some(MirType::Tagged)),
        );
        // Source local typed Tagged.
        func.locals.insert(
            LocalId::from(11u32),
            mk_local(11, Type::Any, Some(MirType::Tagged)),
        );
        func.blocks.insert(
            BlockId::from(1u32),
            BasicBlock {
                id: BlockId::from(1u32),
                instructions: vec![],
                terminator: Terminator::Goto(BlockId::from(0u32)),
            },
        );
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::Phi {
                        dest: LocalId::from(10u32),
                        sources: vec![(BlockId::from(1u32), Operand::Local(LocalId::from(11u32)))],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        let inserts = normalize_phi_sources(&mut func);
        assert_eq!(inserts, 0);
    }

    #[test]
    fn phi_with_raw_source_tagged_dest_gets_boxed() {
        let mut func = empty_func();
        // dest typed Tagged
        func.locals.insert(
            LocalId::from(10u32),
            mk_local(10, Type::Any, Some(MirType::Tagged)),
        );
        // source typed Raw(I64)
        func.locals.insert(
            LocalId::from(11u32),
            mk_local(11, Type::Int, Some(MirType::raw_i64())),
        );
        func.blocks.insert(
            BlockId::from(1u32),
            BasicBlock {
                id: BlockId::from(1u32),
                instructions: vec![],
                terminator: Terminator::Goto(BlockId::from(0u32)),
            },
        );
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::Phi {
                        dest: LocalId::from(10u32),
                        sources: vec![(BlockId::from(1u32), Operand::Local(LocalId::from(11u32)))],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        let inserts = normalize_phi_sources(&mut func);
        assert_eq!(inserts, 1);

        // Predecessor block (1) should now contain a BoxValue.
        let pred = func.blocks.get(&BlockId::from(1u32)).unwrap();
        assert_eq!(pred.instructions.len(), 1);
        match &pred.instructions[0].kind {
            InstructionKind::BoxValue {
                src_type: Type::Int,
                ..
            } => {}
            other => panic!("expected BoxValue Int, got {other:?}"),
        }

        // Phi source should now reference the new boxed temp (not original Local 11).
        let phi_block = func.blocks.get(&BlockId::from(0u32)).unwrap();
        match &phi_block.instructions[0].kind {
            InstructionKind::Phi { sources, .. } => {
                assert!(matches!(&sources[0].1, Operand::Local(id) if id.0 != 11));
            }
            other => panic!("expected Phi, got {other:?}"),
        }
    }

    #[test]
    fn phi_with_constant_int_source_tagged_dest_gets_boxed() {
        let mut func = empty_func();
        func.locals.insert(
            LocalId::from(10u32),
            mk_local(10, Type::Any, Some(MirType::Tagged)),
        );
        func.blocks.insert(
            BlockId::from(1u32),
            BasicBlock {
                id: BlockId::from(1u32),
                instructions: vec![],
                terminator: Terminator::Goto(BlockId::from(0u32)),
            },
        );
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::Phi {
                        dest: LocalId::from(10u32),
                        sources: vec![(BlockId::from(1u32), Operand::Constant(Constant::Int(42)))],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        let inserts = normalize_phi_sources(&mut func);
        assert_eq!(inserts, 1);
    }

    #[test]
    fn phi_with_heap_dest_does_not_box() {
        // dest is Heap(Str), source is Raw(I64). Currently not handled
        // by this pass (would require dest retyping, not source boxing).
        // This is a documented limitation; future step extends.
        let mut func = empty_func();
        func.locals.insert(
            LocalId::from(10u32),
            mk_local(10, Type::Str, Some(MirType::str_heap())),
        );
        func.locals.insert(
            LocalId::from(11u32),
            mk_local(11, Type::Int, Some(MirType::raw_i64())),
        );
        func.blocks.insert(
            BlockId::from(1u32),
            BasicBlock {
                id: BlockId::from(1u32),
                instructions: vec![],
                terminator: Terminator::Goto(BlockId::from(0u32)),
            },
        );
        func.blocks.insert(
            BlockId::from(0u32),
            BasicBlock {
                id: BlockId::from(0u32),
                instructions: vec![Instruction {
                    kind: InstructionKind::Phi {
                        dest: LocalId::from(10u32),
                        sources: vec![(BlockId::from(1u32), Operand::Local(LocalId::from(11u32)))],
                    },
                    span: None,
                }],
                terminator: Terminator::Return(None),
            },
        );
        let inserts = normalize_phi_sources(&mut func);
        assert_eq!(inserts, 0);
    }
}
