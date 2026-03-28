//! Dead Code Elimination optimization pass
//!
//! Removes unreachable blocks, dead instructions, and unused locals.
//! Runs after inlining to clean up dead code introduced by function inlining.

pub(crate) mod liveness;
pub(crate) mod reachability;

#[cfg(test)]
mod tests;

use pyaot_mir::{InstructionKind, Module, Operand, Terminator};
use pyaot_utils::{BlockId, FuncId, LocalId};

/// Perform dead code elimination on all functions in the module.
pub fn eliminate_dead_code(module: &mut Module) {
    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();

    for func_id in func_ids {
        let func = match module.functions.get_mut(&func_id) {
            Some(f) => f,
            None => continue,
        };

        // Iterate to fixpoint: removing dead instructions may make other instructions dead
        loop {
            let mut changed = false;
            changed |= reachability::eliminate_unreachable_blocks(func);
            changed |= liveness::eliminate_dead_instructions(func);
            changed |= liveness::eliminate_dead_locals(func);
            if !changed {
                break;
            }
        }
    }
}

// ==================== Shared helper functions ====================

fn collect_operand_locals(op: &Operand, out: &mut Vec<LocalId>) {
    if let Operand::Local(id) = op {
        out.push(*id);
    }
}

/// Extract the destination local defined by an instruction, if any.
pub(crate) fn instruction_dest(kind: &InstructionKind) -> Option<LocalId> {
    match kind {
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
        | InstructionKind::ExcGetType { dest, .. }
        | InstructionKind::ExcHasException { dest, .. }
        | InstructionKind::ExcGetCurrent { dest, .. }
        | InstructionKind::ExcCheckType { dest, .. }
        | InstructionKind::ExcCheckClass { dest, .. } => Some(*dest),

        // These instructions have no destination (side-effect only)
        InstructionKind::GcPush { .. }
        | InstructionKind::GcPop
        | InstructionKind::ExcPushFrame { .. }
        | InstructionKind::ExcPopFrame
        | InstructionKind::ExcClear
        | InstructionKind::ExcStartHandling
        | InstructionKind::ExcEndHandling => None,
    }
}

/// Returns true if the instruction is pure (no side effects).
/// Pure instructions can be removed if their dest is never used.
///
/// Note: BinOp and UnOp are NOT pure because this compiler uses i64 arithmetic
/// which can raise OverflowError (Add, Sub, Mul, Pow) or ZeroDivisionError
/// (Div, FloorDiv, Mod). FloatToInt can also trap on NaN/infinity.
pub(crate) fn instruction_is_pure(kind: &InstructionKind) -> bool {
    matches!(
        kind,
        InstructionKind::Const { .. }
            | InstructionKind::Copy { .. }
            | InstructionKind::FuncAddr { .. }
            | InstructionKind::BuiltinAddr { .. }
            | InstructionKind::BoolToInt { .. }
            | InstructionKind::IntToFloat { .. }
            | InstructionKind::FloatBits { .. }
            | InstructionKind::IntBitsToFloat { .. }
            | InstructionKind::FloatAbs { .. }
    )
}

/// Collect all LocalIds read by an instruction.
pub(crate) fn instruction_used_locals(kind: &InstructionKind) -> Vec<LocalId> {
    let mut locals = Vec::new();
    match kind {
        InstructionKind::Const { .. } => {}
        InstructionKind::BinOp { left, right, .. } => {
            collect_operand_locals(left, &mut locals);
            collect_operand_locals(right, &mut locals);
        }
        InstructionKind::UnOp { operand, .. } => {
            collect_operand_locals(operand, &mut locals);
        }
        InstructionKind::Call { func, args, .. } => {
            collect_operand_locals(func, &mut locals);
            for arg in args {
                collect_operand_locals(arg, &mut locals);
            }
        }
        InstructionKind::CallDirect { args, .. } | InstructionKind::CallNamed { args, .. } => {
            for arg in args {
                collect_operand_locals(arg, &mut locals);
            }
        }
        InstructionKind::CallVirtual { obj, args, .. }
        | InstructionKind::CallVirtualNamed { obj, args, .. } => {
            collect_operand_locals(obj, &mut locals);
            for arg in args {
                collect_operand_locals(arg, &mut locals);
            }
        }
        InstructionKind::RuntimeCall { args, .. } => {
            for arg in args {
                collect_operand_locals(arg, &mut locals);
            }
        }
        InstructionKind::Copy { src, .. } => {
            collect_operand_locals(src, &mut locals);
        }
        InstructionKind::GcPush { frame } => {
            locals.push(*frame);
        }
        InstructionKind::ExcPushFrame { frame_local } => {
            locals.push(*frame_local);
        }
        InstructionKind::FloatToInt { src, .. }
        | InstructionKind::BoolToInt { src, .. }
        | InstructionKind::IntToFloat { src, .. }
        | InstructionKind::FloatBits { src, .. }
        | InstructionKind::IntBitsToFloat { src, .. }
        | InstructionKind::FloatAbs { src, .. } => {
            collect_operand_locals(src, &mut locals);
        }
        InstructionKind::FuncAddr { .. }
        | InstructionKind::BuiltinAddr { .. }
        | InstructionKind::GcPop
        | InstructionKind::GcAlloc { .. }
        | InstructionKind::ExcPopFrame
        | InstructionKind::ExcGetType { .. }
        | InstructionKind::ExcClear
        | InstructionKind::ExcHasException { .. }
        | InstructionKind::ExcGetCurrent { .. }
        | InstructionKind::ExcCheckType { .. }
        | InstructionKind::ExcCheckClass { .. }
        | InstructionKind::ExcStartHandling
        | InstructionKind::ExcEndHandling => {}
    }
    locals
}

/// Collect all LocalIds read by a terminator.
pub(crate) fn terminator_used_locals(term: &Terminator) -> Vec<LocalId> {
    let mut locals = Vec::new();
    match term {
        Terminator::Return(Some(op)) => collect_operand_locals(op, &mut locals),
        Terminator::Return(None) | Terminator::Goto(_) | Terminator::Unreachable => {}
        Terminator::Branch { cond, .. } => collect_operand_locals(cond, &mut locals),
        Terminator::TrySetjmp { frame_local, .. } => locals.push(*frame_local),
        Terminator::Raise { message, cause, .. } => {
            if let Some(op) = message {
                collect_operand_locals(op, &mut locals);
            }
            if let Some(c) = cause {
                if let Some(op) = &c.message {
                    collect_operand_locals(op, &mut locals);
                }
            }
        }
        Terminator::RaiseCustom {
            message, instance, ..
        } => {
            if let Some(op) = message {
                collect_operand_locals(op, &mut locals);
            }
            if let Some(op) = instance {
                collect_operand_locals(op, &mut locals);
            }
        }
        Terminator::Reraise => {}
        Terminator::RaiseInstance { instance } => {
            collect_operand_locals(instance, &mut locals);
        }
    }
    locals
}

/// Extract successor block IDs from a terminator.
pub(crate) fn terminator_successors(term: &Terminator) -> Vec<BlockId> {
    match term {
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
        | Terminator::RaiseInstance { .. }
        | Terminator::Reraise => vec![],
    }
}
