//! Constant propagation
//!
//! Replaces uses of single-definition constant locals with their constant values.
//! Also simplifies constant branches to unconditional jumps.

use std::collections::HashMap;

use pyaot_mir::{Constant, Function, InstructionKind, Operand, Terminator};
use pyaot_utils::LocalId;

/// Build a map of locals that are defined exactly once via a `Const` instruction.
fn build_constant_map(func: &Function) -> HashMap<LocalId, Constant> {
    // Count definitions per local
    let mut def_count: HashMap<LocalId, usize> = HashMap::new();
    let mut const_defs: HashMap<LocalId, Constant> = HashMap::new();

    for block in func.blocks.values() {
        for inst in &block.instructions {
            if let Some(dest) = crate::dce::instruction_dest(&inst.kind) {
                *def_count.entry(dest).or_insert(0) += 1;
                if let InstructionKind::Const { value, .. } = &inst.kind {
                    const_defs.insert(dest, value.clone());
                }
            }
        }
    }

    // Keep only locals with exactly one definition that is a Const
    const_defs.retain(|id, _| def_count.get(id) == Some(&1));
    const_defs
}

/// Substitute known constants into an operand. Returns true if changed.
fn substitute_operand(op: &mut Operand, constants: &HashMap<LocalId, Constant>) -> bool {
    if let Operand::Local(id) = op {
        if let Some(c) = constants.get(id) {
            *op = Operand::Constant(c.clone());
            return true;
        }
    }
    false
}

/// Run constant propagation on a function. Returns true if any changes were made.
pub fn propagate_constants(func: &mut Function) -> bool {
    let constants = build_constant_map(func);
    if constants.is_empty() {
        return false;
    }

    let mut changed = false;

    for block in func.blocks.values_mut() {
        // Propagate into instruction operands
        for inst in &mut block.instructions {
            changed |= substitute_instruction_operands(&mut inst.kind, &constants);
        }

        // Propagate into terminator operands and simplify constant branches
        changed |= substitute_terminator(&mut block.terminator, &constants);
    }

    changed
}

/// Substitute constants into instruction operands. Returns true if changed.
fn substitute_instruction_operands(
    kind: &mut InstructionKind,
    constants: &HashMap<LocalId, Constant>,
) -> bool {
    let mut changed = false;
    match kind {
        InstructionKind::BinOp { left, right, .. } => {
            changed |= substitute_operand(left, constants);
            changed |= substitute_operand(right, constants);
        }
        InstructionKind::UnOp { operand, .. } => {
            changed |= substitute_operand(operand, constants);
        }
        InstructionKind::Copy { src, .. } => {
            changed |= substitute_operand(src, constants);
        }
        InstructionKind::Call { func, args, .. } => {
            changed |= substitute_operand(func, constants);
            for arg in args {
                changed |= substitute_operand(arg, constants);
            }
        }
        InstructionKind::CallDirect { args, .. } | InstructionKind::CallNamed { args, .. } => {
            for arg in args {
                changed |= substitute_operand(arg, constants);
            }
        }
        InstructionKind::CallVirtual { obj, args, .. }
        | InstructionKind::CallVirtualNamed { obj, args, .. } => {
            changed |= substitute_operand(obj, constants);
            for arg in args {
                changed |= substitute_operand(arg, constants);
            }
        }
        InstructionKind::RuntimeCall { args, .. } => {
            for arg in args {
                changed |= substitute_operand(arg, constants);
            }
        }
        InstructionKind::FloatToInt { src, .. }
        | InstructionKind::BoolToInt { src, .. }
        | InstructionKind::IntToFloat { src, .. }
        | InstructionKind::FloatBits { src, .. }
        | InstructionKind::IntBitsToFloat { src, .. }
        | InstructionKind::FloatAbs { src, .. } => {
            changed |= substitute_operand(src, constants);
        }

        // No operands to substitute
        InstructionKind::Const { .. }
        | InstructionKind::FuncAddr { .. }
        | InstructionKind::BuiltinAddr { .. }
        | InstructionKind::GcPush { .. }
        | InstructionKind::GcPop
        | InstructionKind::GcAlloc { .. }
        | InstructionKind::ExcPushFrame { .. }
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
    changed
}

/// Substitute constants into terminator operands and simplify constant branches.
fn substitute_terminator(term: &mut Terminator, constants: &HashMap<LocalId, Constant>) -> bool {
    let mut changed = false;

    match term {
        Terminator::Return(Some(op)) => {
            changed |= substitute_operand(op, constants);
        }
        Terminator::Branch {
            cond,
            then_block,
            else_block,
        } => {
            changed |= substitute_operand(cond, constants);

            // Simplify constant branches to unconditional jumps
            if let Operand::Constant(c) = cond {
                let is_truthy = match c {
                    Constant::Bool(b) => Some(*b),
                    Constant::Int(n) => Some(*n != 0),
                    _ => None,
                };
                if let Some(truthy) = is_truthy {
                    let target = if truthy { *then_block } else { *else_block };
                    *term = Terminator::Goto(target);
                    changed = true;
                }
            }
        }
        Terminator::Raise { message, cause, .. } => {
            if let Some(op) = message {
                changed |= substitute_operand(op, constants);
            }
            if let Some(c) = cause {
                if let Some(op) = &mut c.message {
                    changed |= substitute_operand(op, constants);
                }
            }
        }
        Terminator::RaiseCustom {
            message, instance, ..
        } => {
            if let Some(op) = message {
                changed |= substitute_operand(op, constants);
            }
            if let Some(op) = instance {
                changed |= substitute_operand(op, constants);
            }
        }
        Terminator::RaiseInstance { instance } => {
            changed |= substitute_operand(instance, constants);
        }
        Terminator::Return(None)
        | Terminator::Goto(_)
        | Terminator::Unreachable
        | Terminator::TrySetjmp { .. }
        | Terminator::Reraise => {}
    }

    changed
}
