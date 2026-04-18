//! SSA property checker for MIR functions.
//!
//! Validates the structural SSA invariants listed in
//! `ARCHITECTURE_REFACTOR.md` § 0.3:
//!
//! 1. Every MIR `LocalId` has exactly one static defining instruction.
//! 2. Every use of a `LocalId` is dominated by its definition.
//! 3. Every `BasicBlock` has a valid `Terminator` (targets reference blocks
//!    that actually exist — no dangling jumps, no implicit fallthrough).
//! 4. Every φ-node has exactly as many incoming values as predecessors,
//!    each source's `BlockId` is an actual predecessor, and Phi instructions
//!    appear only at the **head** of their basic block (before any non-Phi).
//!
//! In Phase 0 the checker is a no-op on legacy MIR: it only runs when
//! `Function::is_ssa == true`. Phase 1 flips individual functions to SSA one
//! by one after rewriting them in proper SSA form; the checker is invoked on
//! those functions and must report zero violations.

use std::collections::{HashMap, HashSet};
use std::fmt;

use pyaot_utils::{BlockId, LocalId};

use crate::dom_tree::{terminator_successors, DomTree};
use crate::{Function, InstructionKind, Operand, Terminator};

/// A single SSA invariant violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsaViolation {
    /// A `LocalId` is defined by more than one instruction (or by both a
    /// parameter slot and an instruction).
    MultipleDefinitions {
        local: LocalId,
        first_block: BlockId,
        second_block: BlockId,
    },

    /// A `LocalId` is used but no defining instruction or parameter slot was
    /// found for it anywhere in the function.
    UseWithoutDef { local: LocalId, use_block: BlockId },

    /// A `LocalId` is used in a block that the defining block does not
    /// dominate (or, for intra-block uses, before its defining instruction).
    UseNotDominated {
        local: LocalId,
        def_block: BlockId,
        use_block: BlockId,
    },

    /// A terminator references a block that is not present in the function.
    InvalidTerminatorTarget { block: BlockId, target: BlockId },

    /// A φ-node's source list length disagrees with the block's predecessor
    /// count, or contains a `BlockId` that is not an actual predecessor.
    PhiArityMismatch {
        block: BlockId,
        local: LocalId,
        expected_preds: Vec<BlockId>,
        got_preds: Vec<BlockId>,
    },

    /// A `Phi` instruction appears after a non-Phi instruction in its block.
    /// Phi nodes must occupy a contiguous prefix of `block.instructions`.
    PhiNotAtBlockHead { block: BlockId, local: LocalId },
}

impl fmt::Display for SsaViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SsaViolation::MultipleDefinitions {
                local,
                first_block,
                second_block,
            } => write!(
                f,
                "SSA violation: {} has multiple defining sites (first in {}, again in {})",
                local, first_block, second_block
            ),
            SsaViolation::UseWithoutDef { local, use_block } => write!(
                f,
                "SSA violation: {} used in {} without any prior definition or parameter slot",
                local, use_block
            ),
            SsaViolation::UseNotDominated {
                local,
                def_block,
                use_block,
            } => write!(
                f,
                "SSA violation: {} used in {} but its definition in {} does not dominate the use site",
                local, use_block, def_block
            ),
            SsaViolation::InvalidTerminatorTarget { block, target } => write!(
                f,
                "SSA violation: terminator of {} jumps to {}, which is not a block in this function",
                block, target
            ),
            SsaViolation::PhiArityMismatch {
                block,
                local,
                expected_preds,
                got_preds,
            } => write!(
                f,
                "SSA violation: φ-node {} in {} has sources {:?} but block predecessors are {:?}",
                local, block, got_preds, expected_preds
            ),
            SsaViolation::PhiNotAtBlockHead { block, local } => write!(
                f,
                "SSA violation: φ-node {} in {} appears after a non-Phi instruction; φs must be at block head",
                local, block
            ),
        }
    }
}

/// Run the SSA property checker on `func`.
///
/// When `func.is_ssa == false` this returns `Ok(())` immediately (legacy
/// non-SSA MIR is explicitly allowed). Otherwise, every invariant is checked
/// and all violations are collected into a single `Err` so the caller can
/// surface them in one pass instead of iterating fix/re-run cycles.
pub fn check(func: &Function) -> Result<(), Vec<SsaViolation>> {
    if !func.is_ssa {
        return Ok(());
    }

    let mut violations = Vec::new();

    check_terminator_targets(func, &mut violations);
    check_definitions_and_dominance(func, &mut violations);
    check_phi_structure(func, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn check_terminator_targets(func: &Function, violations: &mut Vec<SsaViolation>) {
    for (&bid, block) in &func.blocks {
        for succ in terminator_successors(&block.terminator) {
            if !func.blocks.contains_key(&succ) {
                violations.push(SsaViolation::InvalidTerminatorTarget {
                    block: bid,
                    target: succ,
                });
            }
        }
    }
}

fn check_phi_structure(func: &Function, violations: &mut Vec<SsaViolation>) {
    // Build the reverse CFG once: block → its predecessors.
    let mut predecessors: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for (&bid, block) in &func.blocks {
        for succ in terminator_successors(&block.terminator) {
            predecessors.entry(succ).or_default().push(bid);
        }
    }

    for (&bid, block) in &func.blocks {
        let expected: Vec<BlockId> = predecessors.get(&bid).cloned().unwrap_or_default();
        let expected_set: HashSet<BlockId> = expected.iter().copied().collect();

        let mut seen_non_phi = false;
        for inst in &block.instructions {
            match &inst.kind {
                InstructionKind::Phi { dest, sources } => {
                    if seen_non_phi {
                        violations.push(SsaViolation::PhiNotAtBlockHead {
                            block: bid,
                            local: *dest,
                        });
                        // Keep scanning — further Phis in the same block also
                        // belong at the head.
                    }
                    let got: Vec<BlockId> = sources.iter().map(|(b, _)| *b).collect();
                    let got_set: HashSet<BlockId> = got.iter().copied().collect();
                    if got.len() != expected.len() || got_set != expected_set {
                        violations.push(SsaViolation::PhiArityMismatch {
                            block: bid,
                            local: *dest,
                            expected_preds: expected.clone(),
                            got_preds: got,
                        });
                    }
                }
                _ => {
                    seen_non_phi = true;
                }
            }
        }
    }
}

fn check_definitions_and_dominance(func: &Function, violations: &mut Vec<SsaViolation>) {
    // Record the first defining block for each local. Parameters count as
    // defined at the entry block.
    let mut def_block: HashMap<LocalId, BlockId> = HashMap::new();
    for p in &func.params {
        if let Some(prev) = def_block.insert(p.id, func.entry_block) {
            violations.push(SsaViolation::MultipleDefinitions {
                local: p.id,
                first_block: prev,
                second_block: func.entry_block,
            });
        }
    }
    for (&bid, block) in &func.blocks {
        for instr in &block.instructions {
            if let Some(d) = instruction_def(&instr.kind) {
                if let Some(prev) = def_block.insert(d, bid) {
                    violations.push(SsaViolation::MultipleDefinitions {
                        local: d,
                        first_block: prev,
                        second_block: bid,
                    });
                }
            }
        }
    }

    let dom = func.dom_tree();

    for (&bid, block) in &func.blocks {
        // Locals that have been defined earlier within this block (or, for
        // the entry block, parameters) are visible to subsequent instructions
        // regardless of the block-level dominance relation.
        let mut defined_in_block: HashSet<LocalId> = HashSet::new();
        if bid == func.entry_block {
            for p in &func.params {
                defined_in_block.insert(p.id);
            }
        }

        for instr in &block.instructions {
            // Phi uses are special: each source (pred_bb, op) is semantically
            // "used at the end of pred_bb", not inside the phi's own block.
            // Classical SSA dominance requires the defining block to dominate
            // the predecessor — never the phi block itself, which is often a
            // merge point that no individual predecessor dominates.
            if let InstructionKind::Phi { sources, .. } = &instr.kind {
                for (pred_bb, op) in sources {
                    if let Operand::Local(local) = op {
                        check_phi_source_use(*local, *pred_bb, &def_block, dom, violations);
                    }
                }
            } else {
                for u in instruction_uses(&instr.kind) {
                    check_one_use(u, bid, &def_block, &defined_in_block, dom, violations);
                }
            }
            if let Some(d) = instruction_def(&instr.kind) {
                defined_in_block.insert(d);
            }
        }
        for u in terminator_uses(&block.terminator) {
            check_one_use(u, bid, &def_block, &defined_in_block, dom, violations);
        }
    }
}

/// Phi-source dominance check: the value flows from the end of `pred_block`
/// into the phi at its merge block. The definition must reach the predecessor
/// block's terminator — either defined somewhere inside `pred_block` itself,
/// or in a block that strictly dominates `pred_block`.
fn check_phi_source_use(
    local: LocalId,
    pred_block: BlockId,
    def_block: &HashMap<LocalId, BlockId>,
    dom: &DomTree,
    violations: &mut Vec<SsaViolation>,
) {
    let Some(&db) = def_block.get(&local) else {
        violations.push(SsaViolation::UseWithoutDef {
            local,
            use_block: pred_block,
        });
        return;
    };
    if db == pred_block {
        return;
    }
    if !dom.dominates(db, pred_block) {
        violations.push(SsaViolation::UseNotDominated {
            local,
            def_block: db,
            use_block: pred_block,
        });
    }
}

fn check_one_use(
    local: LocalId,
    use_block: BlockId,
    def_block: &HashMap<LocalId, BlockId>,
    defined_in_block: &HashSet<LocalId>,
    dom: &DomTree,
    violations: &mut Vec<SsaViolation>,
) {
    let Some(&db) = def_block.get(&local) else {
        violations.push(SsaViolation::UseWithoutDef { local, use_block });
        return;
    };
    if db == use_block {
        if !defined_in_block.contains(&local) {
            violations.push(SsaViolation::UseNotDominated {
                local,
                def_block: db,
                use_block,
            });
        }
    } else if !dom.dominates(db, use_block) {
        violations.push(SsaViolation::UseNotDominated {
            local,
            def_block: db,
            use_block,
        });
    }
}

fn push_op(op: &Operand, out: &mut Vec<LocalId>) {
    if let Operand::Local(id) = op {
        out.push(*id);
    }
}

fn terminator_uses(t: &Terminator) -> Vec<LocalId> {
    let mut out = Vec::new();
    match t {
        Terminator::Return(op) => {
            if let Some(op) = op {
                push_op(op, &mut out);
            }
        }
        Terminator::Goto(_) | Terminator::Unreachable | Terminator::Reraise => {}
        Terminator::Branch { cond, .. } => push_op(cond, &mut out),
        Terminator::TrySetjmp { frame_local, .. } => out.push(*frame_local),
        Terminator::Raise { message, cause, .. } => {
            if let Some(op) = message {
                push_op(op, &mut out);
            }
            if let Some(cause) = cause {
                if let Some(op) = &cause.message {
                    push_op(op, &mut out);
                }
            }
        }
        Terminator::RaiseCustom {
            message, instance, ..
        } => {
            if let Some(op) = message {
                push_op(op, &mut out);
            }
            if let Some(op) = instance {
                push_op(op, &mut out);
            }
        }
        Terminator::RaiseInstance { instance } => push_op(instance, &mut out),
    }
    out
}

fn instruction_def(kind: &InstructionKind) -> Option<LocalId> {
    use InstructionKind::*;
    match kind {
        // Void RuntimeCalls use `dest` as a side-effect placeholder;
        // their codegen leaves the Cranelift slot untouched. Treating
        // them as defs would flag legitimate multi-use of a placeholder
        // (e.g. multiple `rt_string_builder_append` calls in a loop
        // body) as SSA violations.
        RuntimeCall { dest, func, .. } => {
            if crate::ssa_construct::runtime_call_is_void(func) {
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
        GcPush { .. }
        | GcPop
        | ExcPushFrame { .. }
        | ExcPopFrame
        | ExcClear
        | ExcStartHandling
        | ExcEndHandling => None,
    }
}

fn instruction_uses(kind: &InstructionKind) -> Vec<LocalId> {
    use InstructionKind::*;
    let mut out = Vec::new();
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
            push_op(left, &mut out);
            push_op(right, &mut out);
        }
        UnOp { operand, .. } => push_op(operand, &mut out),
        Copy { src, .. }
        | FloatToInt { src, .. }
        | BoolToInt { src, .. }
        | IntToFloat { src, .. }
        | FloatBits { src, .. }
        | IntBitsToFloat { src, .. }
        | FloatAbs { src, .. } => push_op(src, &mut out),
        Call { func, args, .. } => {
            push_op(func, &mut out);
            for a in args {
                push_op(a, &mut out);
            }
        }
        CallDirect { args, .. } | CallNamed { args, .. } | RuntimeCall { args, .. } => {
            for a in args {
                push_op(a, &mut out);
            }
        }
        CallVirtual { obj, args, .. } | CallVirtualNamed { obj, args, .. } => {
            push_op(obj, &mut out);
            for a in args {
                push_op(a, &mut out);
            }
        }
        GcPush { frame } => out.push(*frame),
        ExcPushFrame { frame_local } => out.push(*frame_local),
        Phi { sources, .. } => {
            for (_, op) in sources {
                push_op(op, &mut out);
            }
        }
        Refine { src, .. } => push_op(src, &mut out),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use pyaot_types::Type;
    use pyaot_utils::FuncId;

    use crate::{BasicBlock, Constant, Function, Instruction, Local};

    fn local(id: u32, ty: Type) -> Local {
        Local {
            id: LocalId::from(id),
            name: None,
            ty,
            is_gc_root: false,
        }
    }

    fn mk_instr(kind: InstructionKind) -> Instruction {
        Instruction { kind, span: None }
    }

    /// Build a simple SSA-valid function:
    ///   bb0: l1 = 1; l2 = 2; l3 = l1 + l2; return l3
    fn valid_linear_ssa() -> Function {
        let l1 = LocalId::from(1u32);
        let l2 = LocalId::from(2u32);
        let l3 = LocalId::from(3u32);
        let bb0 = BlockId::from(0u32);

        let mut locals = IndexMap::new();
        locals.insert(l1, local(1, Type::Int));
        locals.insert(l2, local(2, Type::Int));
        locals.insert(l3, local(3, Type::Int));

        let block = BasicBlock {
            id: bb0,
            instructions: vec![
                mk_instr(InstructionKind::Const {
                    dest: l1,
                    value: Constant::Int(1),
                }),
                mk_instr(InstructionKind::Const {
                    dest: l2,
                    value: Constant::Int(2),
                }),
                mk_instr(InstructionKind::BinOp {
                    dest: l3,
                    op: crate::BinOp::Add,
                    left: Operand::Local(l1),
                    right: Operand::Local(l2),
                }),
            ],
            terminator: Terminator::Return(Some(Operand::Local(l3))),
        };

        let mut blocks = IndexMap::new();
        blocks.insert(bb0, block);

        Function {
            id: FuncId::from(0u32),
            name: "valid".to_string(),
            params: Vec::new(),
            return_type: Type::Int,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        }
    }

    #[test]
    fn accepts_valid_linear_ssa() {
        let func = valid_linear_ssa();
        assert_eq!(check(&func), Ok(()));
    }

    #[test]
    fn skips_legacy_non_ssa_function() {
        // A function that trivially violates SSA — double-defined local —
        // is accepted when is_ssa = false.
        let mut func = valid_linear_ssa();
        func.is_ssa = false;

        let l1 = LocalId::from(1u32);
        let bb0 = func.entry_block;
        func.blocks
            .get_mut(&bb0)
            .unwrap()
            .instructions
            .push(mk_instr(InstructionKind::Const {
                dest: l1,
                value: Constant::Int(99),
            }));

        assert_eq!(check(&func), Ok(()));
    }

    #[test]
    fn detects_multiple_definitions() {
        let mut func = valid_linear_ssa();
        let l1 = LocalId::from(1u32);
        let bb0 = func.entry_block;
        func.blocks
            .get_mut(&bb0)
            .unwrap()
            .instructions
            .push(mk_instr(InstructionKind::Const {
                dest: l1,
                value: Constant::Int(42),
            }));

        let err = check(&func).unwrap_err();
        assert!(matches!(
            err[0],
            SsaViolation::MultipleDefinitions { local, .. } if local == l1
        ));
    }

    #[test]
    fn detects_use_without_def() {
        let bb0 = BlockId::from(0u32);
        let l_missing = LocalId::from(99u32);
        let l_dest = LocalId::from(1u32);

        let mut locals = IndexMap::new();
        locals.insert(l_dest, local(1, Type::Int));

        let block = BasicBlock {
            id: bb0,
            instructions: vec![mk_instr(InstructionKind::Copy {
                dest: l_dest,
                src: Operand::Local(l_missing),
            })],
            terminator: Terminator::Return(None),
        };

        let mut blocks = IndexMap::new();
        blocks.insert(bb0, block);

        let func = Function {
            id: FuncId::from(0u32),
            name: "bad".to_string(),
            params: Vec::new(),
            return_type: Type::None,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        };

        let err = check(&func).unwrap_err();
        assert!(matches!(
            err[0],
            SsaViolation::UseWithoutDef { local, .. } if local == l_missing
        ));
    }

    #[test]
    fn detects_use_before_def_in_same_block() {
        let bb0 = BlockId::from(0u32);
        let a = LocalId::from(1u32);
        let b = LocalId::from(2u32);

        let mut locals = IndexMap::new();
        locals.insert(a, local(1, Type::Int));
        locals.insert(b, local(2, Type::Int));

        // Use `a` before it is defined, within the same block.
        let block = BasicBlock {
            id: bb0,
            instructions: vec![
                mk_instr(InstructionKind::Copy {
                    dest: b,
                    src: Operand::Local(a),
                }),
                mk_instr(InstructionKind::Const {
                    dest: a,
                    value: Constant::Int(1),
                }),
            ],
            terminator: Terminator::Return(None),
        };

        let mut blocks = IndexMap::new();
        blocks.insert(bb0, block);

        let func = Function {
            id: FuncId::from(0u32),
            name: "bad".to_string(),
            params: Vec::new(),
            return_type: Type::None,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        };

        let err = check(&func).unwrap_err();
        assert!(matches!(
            err[0],
            SsaViolation::UseNotDominated { local, .. } if local == a
        ));
    }

    #[test]
    fn detects_dangling_terminator_target() {
        let mut func = valid_linear_ssa();
        let bb0 = func.entry_block;
        let missing = BlockId::from(42u32);
        func.blocks.get_mut(&bb0).unwrap().terminator = Terminator::Goto(missing);

        let err = check(&func).unwrap_err();
        assert!(matches!(
            err[0],
            SsaViolation::InvalidTerminatorTarget { target, .. } if target == missing
        ));
    }

    /// Two-block diamond CFG:
    ///   bb0: l1 = 1; goto bb1
    ///   bb1: l2 = l1 + l1; return l2
    /// Use of l1 in bb1 is dominated by its def in bb0. Valid.
    #[test]
    fn accepts_cross_block_dominated_use() {
        let l1 = LocalId::from(1u32);
        let l2 = LocalId::from(2u32);
        let bb0 = BlockId::from(0u32);
        let bb1 = BlockId::from(1u32);

        let mut locals = IndexMap::new();
        locals.insert(l1, local(1, Type::Int));
        locals.insert(l2, local(2, Type::Int));

        let block0 = BasicBlock {
            id: bb0,
            instructions: vec![mk_instr(InstructionKind::Const {
                dest: l1,
                value: Constant::Int(1),
            })],
            terminator: Terminator::Goto(bb1),
        };
        let block1 = BasicBlock {
            id: bb1,
            instructions: vec![mk_instr(InstructionKind::BinOp {
                dest: l2,
                op: crate::BinOp::Add,
                left: Operand::Local(l1),
                right: Operand::Local(l1),
            })],
            terminator: Terminator::Return(Some(Operand::Local(l2))),
        };

        let mut blocks = IndexMap::new();
        blocks.insert(bb0, block0);
        blocks.insert(bb1, block1);

        let func = Function {
            id: FuncId::from(0u32),
            name: "dom".to_string(),
            params: Vec::new(),
            return_type: Type::Int,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        };

        assert_eq!(check(&func), Ok(()));
    }

    /// Diamond CFG where a local defined only on one branch is used in the
    /// join block without a φ-node — classic "def does not dominate use".
    ///   bb0: branch cond -> bb1 | bb2
    ///   bb1: l1 = 1; goto bb3
    ///   bb2: goto bb3
    ///   bb3: return l1
    #[test]
    fn detects_non_dominating_cross_block_use() {
        let cond = LocalId::from(0u32);
        let l1 = LocalId::from(1u32);
        let bb0 = BlockId::from(0u32);
        let bb1 = BlockId::from(1u32);
        let bb2 = BlockId::from(2u32);
        let bb3 = BlockId::from(3u32);

        let mut locals = IndexMap::new();
        locals.insert(cond, local(0, Type::Bool));
        locals.insert(l1, local(1, Type::Int));

        let block0 = BasicBlock {
            id: bb0,
            instructions: vec![mk_instr(InstructionKind::Const {
                dest: cond,
                value: Constant::Bool(true),
            })],
            terminator: Terminator::Branch {
                cond: Operand::Local(cond),
                then_block: bb1,
                else_block: bb2,
            },
        };
        let block1 = BasicBlock {
            id: bb1,
            instructions: vec![mk_instr(InstructionKind::Const {
                dest: l1,
                value: Constant::Int(1),
            })],
            terminator: Terminator::Goto(bb3),
        };
        let block2 = BasicBlock {
            id: bb2,
            instructions: vec![],
            terminator: Terminator::Goto(bb3),
        };
        let block3 = BasicBlock {
            id: bb3,
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Local(l1))),
        };

        let mut blocks = IndexMap::new();
        blocks.insert(bb0, block0);
        blocks.insert(bb1, block1);
        blocks.insert(bb2, block2);
        blocks.insert(bb3, block3);

        let func = Function {
            id: FuncId::from(0u32),
            name: "nodom".to_string(),
            params: Vec::new(),
            return_type: Type::Int,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        };

        let err = check(&func).unwrap_err();
        assert!(err.iter().any(|v| matches!(
            v,
            SsaViolation::UseNotDominated { local, def_block, use_block }
                if *local == l1 && *def_block == bb1 && *use_block == bb3
        )));
    }

    /// Diamond CFG with a valid 2-source Phi at the merge block.
    ///
    ///    bb0 ──▶ bb1 ──┐
    ///      \           ▼
    ///       ──▶ bb2 ──▶ bb3: m = φ((bb1, l1), (bb2, l2)); return m
    #[test]
    fn valid_phi_at_diamond_merge_accepts() {
        let l1 = LocalId::from(1u32);
        let l2 = LocalId::from(2u32);
        let m = LocalId::from(3u32);
        let cond = LocalId::from(4u32);
        let bb0 = BlockId::from(0u32);
        let bb1 = BlockId::from(1u32);
        let bb2 = BlockId::from(2u32);
        let bb3 = BlockId::from(3u32);

        let mut locals = IndexMap::new();
        locals.insert(l1, local(1, Type::Int));
        locals.insert(l2, local(2, Type::Int));
        locals.insert(m, local(3, Type::Int));
        locals.insert(cond, local(4, Type::Bool));

        let mut blocks = IndexMap::new();
        blocks.insert(
            bb0,
            BasicBlock {
                id: bb0,
                instructions: vec![mk_instr(InstructionKind::Const {
                    dest: cond,
                    value: Constant::Bool(true),
                })],
                terminator: Terminator::Branch {
                    cond: Operand::Local(cond),
                    then_block: bb1,
                    else_block: bb2,
                },
            },
        );
        blocks.insert(
            bb1,
            BasicBlock {
                id: bb1,
                instructions: vec![mk_instr(InstructionKind::Const {
                    dest: l1,
                    value: Constant::Int(10),
                })],
                terminator: Terminator::Goto(bb3),
            },
        );
        blocks.insert(
            bb2,
            BasicBlock {
                id: bb2,
                instructions: vec![mk_instr(InstructionKind::Const {
                    dest: l2,
                    value: Constant::Int(20),
                })],
                terminator: Terminator::Goto(bb3),
            },
        );
        blocks.insert(
            bb3,
            BasicBlock {
                id: bb3,
                instructions: vec![mk_instr(InstructionKind::Phi {
                    dest: m,
                    sources: vec![(bb1, Operand::Local(l1)), (bb2, Operand::Local(l2))],
                })],
                terminator: Terminator::Return(Some(Operand::Local(m))),
            },
        );

        let func = Function {
            id: FuncId::from(0u32),
            name: "test".to_string(),
            params: Vec::new(),
            return_type: Type::Int,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        };

        assert!(check(&func).is_ok());
    }

    /// Phi with source count that doesn't match predecessor count is rejected.
    #[test]
    fn detects_phi_arity_mismatch() {
        let l1 = LocalId::from(1u32);
        let m = LocalId::from(2u32);
        let bb0 = BlockId::from(0u32);
        let bb1 = BlockId::from(1u32);

        let mut locals = IndexMap::new();
        locals.insert(l1, local(1, Type::Int));
        locals.insert(m, local(2, Type::Int));

        let mut blocks = IndexMap::new();
        blocks.insert(
            bb0,
            BasicBlock {
                id: bb0,
                instructions: vec![mk_instr(InstructionKind::Const {
                    dest: l1,
                    value: Constant::Int(1),
                })],
                terminator: Terminator::Goto(bb1),
            },
        );
        // bb1 has one predecessor (bb0) but the Phi lists two sources.
        blocks.insert(
            bb1,
            BasicBlock {
                id: bb1,
                instructions: vec![mk_instr(InstructionKind::Phi {
                    dest: m,
                    sources: vec![
                        (bb0, Operand::Local(l1)),
                        (bb1, Operand::Local(l1)), // spurious
                    ],
                })],
                terminator: Terminator::Return(Some(Operand::Local(m))),
            },
        );

        let func = Function {
            id: FuncId::from(0u32),
            name: "test".to_string(),
            params: Vec::new(),
            return_type: Type::Int,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        };

        let err = check(&func).unwrap_err();
        assert!(err
            .iter()
            .any(|v| matches!(v, SsaViolation::PhiArityMismatch { local, .. } if *local == m)));
    }

    /// Phi placed after a non-Phi instruction is rejected.
    #[test]
    fn detects_phi_not_at_block_head() {
        let l1 = LocalId::from(1u32);
        let l2 = LocalId::from(2u32);
        let m = LocalId::from(3u32);
        let bb0 = BlockId::from(0u32);
        let bb1 = BlockId::from(1u32);

        let mut locals = IndexMap::new();
        locals.insert(l1, local(1, Type::Int));
        locals.insert(l2, local(2, Type::Int));
        locals.insert(m, local(3, Type::Int));

        let mut blocks = IndexMap::new();
        blocks.insert(
            bb0,
            BasicBlock {
                id: bb0,
                instructions: vec![mk_instr(InstructionKind::Const {
                    dest: l1,
                    value: Constant::Int(1),
                })],
                terminator: Terminator::Goto(bb1),
            },
        );
        blocks.insert(
            bb1,
            BasicBlock {
                id: bb1,
                instructions: vec![
                    // Non-Phi first…
                    mk_instr(InstructionKind::Const {
                        dest: l2,
                        value: Constant::Int(2),
                    }),
                    // …then a Phi — invalid ordering.
                    mk_instr(InstructionKind::Phi {
                        dest: m,
                        sources: vec![(bb0, Operand::Local(l1))],
                    }),
                ],
                terminator: Terminator::Return(Some(Operand::Local(m))),
            },
        );

        let func = Function {
            id: FuncId::from(0u32),
            name: "test".to_string(),
            params: Vec::new(),
            return_type: Type::Int,
            locals,
            blocks,
            entry_block: bb0,
            span: None,
            is_ssa: true,
            dom_tree_cache: std::cell::OnceCell::new(),
        };

        let err = check(&func).unwrap_err();
        assert!(err
            .iter()
            .any(|v| matches!(v, SsaViolation::PhiNotAtBlockHead { local, .. } if *local == m)));
    }
}
