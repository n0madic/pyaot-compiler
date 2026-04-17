//! SSA property checker for MIR functions.
//!
//! Validates the structural SSA invariants listed in
//! `ARCHITECTURE_REFACTOR.md` § 0.3:
//!
//! 1. Every MIR `LocalId` has exactly one static defining instruction.
//! 2. Every use of a `LocalId` is dominated by its definition.
//! 3. Every `BasicBlock` has a valid `Terminator` (targets reference blocks
//!    that actually exist — no dangling jumps, no implicit fallthrough).
//! 4. Every φ-node has exactly as many incoming values as predecessors.
//!
//! In Phase 0 the checker is a no-op on legacy MIR: it only runs when
//! `Function::is_ssa == true`. Phase 1 flips individual functions to SSA one
//! by one after rewriting them in proper SSA form; the checker is invoked on
//! those functions and must report zero violations.
//!
//! Invariant (4) is currently unreachable because MIR has no `Phi` variant
//! yet — Phase 1 will introduce it. The check is left here as an explicit
//! no-op to mark the integration point.

use std::collections::{HashMap, HashSet};
use std::fmt;

use pyaot_utils::{BlockId, LocalId};

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
    // Phi arity: InstructionKind::Phi does not exist yet. Phase 1 introduces
    // it; at that point this block will iterate phi nodes and compare
    // `incoming.len()` to `predecessors(phi_block).len()`. Kept as an
    // explicit no-op so the integration point is greppable.

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

    let dominators = compute_dominators(func);

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
            for u in instruction_uses(&instr.kind) {
                check_one_use(
                    u,
                    bid,
                    &def_block,
                    &defined_in_block,
                    &dominators,
                    violations,
                );
            }
            if let Some(d) = instruction_def(&instr.kind) {
                defined_in_block.insert(d);
            }
        }
        for u in terminator_uses(&block.terminator) {
            check_one_use(
                u,
                bid,
                &def_block,
                &defined_in_block,
                &dominators,
                violations,
            );
        }
    }
}

fn check_one_use(
    local: LocalId,
    use_block: BlockId,
    def_block: &HashMap<LocalId, BlockId>,
    defined_in_block: &HashSet<LocalId>,
    dominators: &HashMap<BlockId, HashSet<BlockId>>,
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
    } else if !dominates(dominators, db, use_block) {
        violations.push(SsaViolation::UseNotDominated {
            local,
            def_block: db,
            use_block,
        });
    }
}

fn compute_dominators(func: &Function) -> HashMap<BlockId, HashSet<BlockId>> {
    let all_blocks: Vec<BlockId> = func.blocks.keys().copied().collect();
    let all_set: HashSet<BlockId> = all_blocks.iter().copied().collect();
    let preds = compute_predecessors(func);

    let mut dom: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
    for &b in &all_blocks {
        if b == func.entry_block {
            let mut s = HashSet::new();
            s.insert(b);
            dom.insert(b, s);
        } else {
            dom.insert(b, all_set.clone());
        }
    }

    // Classical iterative data-flow: dom(n) = {n} ∪ (∩ dom(p) for p ∈ preds(n)).
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &all_blocks {
            if b == func.entry_block {
                continue;
            }
            let empty = Vec::new();
            let block_preds = preds.get(&b).unwrap_or(&empty);
            let mut new_dom: Option<HashSet<BlockId>> = None;
            for p in block_preds {
                let p_dom = match dom.get(p) {
                    Some(d) => d.clone(),
                    None => continue,
                };
                new_dom = Some(match new_dom {
                    None => p_dom,
                    Some(existing) => existing.intersection(&p_dom).copied().collect(),
                });
            }
            let mut new_dom = new_dom.unwrap_or_default();
            new_dom.insert(b);
            if dom.get(&b) != Some(&new_dom) {
                dom.insert(b, new_dom);
                changed = true;
            }
        }
    }
    dom
}

fn dominates(
    dominators: &HashMap<BlockId, HashSet<BlockId>>,
    dominator: BlockId,
    dominatee: BlockId,
) -> bool {
    dominators
        .get(&dominatee)
        .map(|s| s.contains(&dominator))
        .unwrap_or(false)
}

fn compute_predecessors(func: &Function) -> HashMap<BlockId, Vec<BlockId>> {
    let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for (&bid, block) in &func.blocks {
        for succ in terminator_successors(&block.terminator) {
            preds.entry(succ).or_default().push(bid);
        }
    }
    preds
}

fn terminator_successors(t: &Terminator) -> Vec<BlockId> {
    match t {
        Terminator::Goto(b) => vec![*b],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::TrySetjmp {
            try_body,
            handler_entry,
            ..
        } => vec![*try_body, *handler_entry],
        Terminator::Return(_)
        | Terminator::Unreachable
        | Terminator::Raise { .. }
        | Terminator::RaiseCustom { .. }
        | Terminator::Reraise
        | Terminator::RaiseInstance { .. } => Vec::new(),
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
        | ExcCheckClass { dest, .. } => Some(*dest),
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
        };

        let err = check(&func).unwrap_err();
        assert!(err.iter().any(|v| matches!(
            v,
            SsaViolation::UseNotDominated { local, def_block, use_block }
                if *local == l1 && *def_block == bb1 && *use_block == bb3
        )));
    }
}
