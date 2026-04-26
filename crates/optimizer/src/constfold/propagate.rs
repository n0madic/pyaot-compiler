//! Constant and copy propagation.
//!
//! Replaces uses of locals that are aliases for a constant or for another
//! local. Under SSA (§1.3 / S1.6) every local has exactly one definition,
//! so the single-def restriction is implicit — no `def_count` tracking is
//! needed. Also simplifies `Branch` on a constant condition to `Goto`.

use std::collections::{HashMap, HashSet};

use pyaot_mir::{Constant, Function, InstructionKind, Operand, Terminator};
use pyaot_utils::LocalId;

/// What a local aliases, if anything. Used by `build_propagation_map`.
///
/// * `Const` — local was defined by `Const` (or by `Copy` of a constant);
///   substitute its literal value.
/// * `Alias` — local was defined by `Copy { src: Local(s) }`; every use
///   can be replaced with `s` (possibly itself an alias; resolved
///   transitively in `resolve_operand`).
#[derive(Debug, Clone)]
enum PropValue {
    Const(Constant),
    Alias(LocalId),
}

/// Build the propagation map:
/// * `Const { dest, value }` → `dest ↦ Const(value)`
/// * `Copy { dest, src: Local(s) }` → `dest ↦ Alias(s)`
/// * `Copy { dest, src: Constant(c) }` → `dest ↦ Const(c)`
///
/// Under SSA every dest appears at most once, so last-writer-wins
/// semantics here are safe.
fn build_propagation_map(func: &Function) -> HashMap<LocalId, PropValue> {
    let mut props: HashMap<LocalId, PropValue> = HashMap::new();
    for block in func.blocks.values() {
        for inst in &block.instructions {
            match &inst.kind {
                InstructionKind::Const { dest, value } => {
                    props.insert(*dest, PropValue::Const(value.clone()));
                }
                InstructionKind::Copy {
                    dest,
                    src: Operand::Constant(c),
                } => {
                    props.insert(*dest, PropValue::Const(c.clone()));
                }
                InstructionKind::Copy {
                    dest,
                    src: Operand::Local(s),
                } => {
                    // Self-copies shouldn't occur but guard anyway.
                    if dest != s {
                        props.insert(*dest, PropValue::Alias(*s));
                    }
                }
                _ => {}
            }
        }
    }
    props
}

/// Resolve an operand through the propagation map.
/// * Returns `Some(Operand::Constant(_))` if the local chain ends at a
///   constant.
/// * Returns `Some(Operand::Local(_))` if the local chain ends at a
///   non-aliased local that differs from the input.
/// * Returns `None` if the operand is unchanged (constant, or local with
///   no alias entry).
///
/// Cycle guard via a visited set. Cycles shouldn't occur in well-formed
/// SSA (every Copy's src dominates its dest), but cheap to defend.
fn resolve_operand(op: &Operand, props: &HashMap<LocalId, PropValue>) -> Option<Operand> {
    let Operand::Local(mut id) = *op else {
        return None;
    };
    let mut visited: HashSet<LocalId> = HashSet::new();
    loop {
        if !visited.insert(id) {
            return None;
        }
        match props.get(&id) {
            Some(PropValue::Const(c)) => return Some(Operand::Constant(c.clone())),
            Some(PropValue::Alias(s)) => {
                id = *s;
            }
            None => {
                return if let Operand::Local(orig) = *op {
                    if id == orig {
                        None
                    } else {
                        Some(Operand::Local(id))
                    }
                } else {
                    None
                };
            }
        }
    }
}

/// Substitute propagated values into an operand. Returns true if changed.
fn substitute_operand(op: &mut Operand, props: &HashMap<LocalId, PropValue>) -> bool {
    if let Some(new_op) = resolve_operand(op, props) {
        *op = new_op;
        true
    } else {
        false
    }
}

/// Run constant + copy propagation on a function. Returns true if any
/// changes were made.
pub fn propagate_constants(func: &mut Function) -> bool {
    let props = build_propagation_map(func);
    if props.is_empty() {
        return false;
    }

    let mut changed = false;

    for block in func.blocks.values_mut() {
        for inst in &mut block.instructions {
            changed |= substitute_instruction_operands(&mut inst.kind, &props);
        }
        changed |= substitute_terminator(&mut block.terminator, &props);
    }

    changed
}

/// Substitute propagated values into instruction operands. Returns true
/// if changed.
fn substitute_instruction_operands(
    kind: &mut InstructionKind,
    props: &HashMap<LocalId, PropValue>,
) -> bool {
    let mut changed = false;
    match kind {
        InstructionKind::BinOp { left, right, .. } => {
            changed |= substitute_operand(left, props);
            changed |= substitute_operand(right, props);
        }
        InstructionKind::UnOp { operand, .. } => {
            changed |= substitute_operand(operand, props);
        }
        InstructionKind::Copy { src, .. } => {
            changed |= substitute_operand(src, props);
        }
        InstructionKind::Call { func, args, .. } => {
            changed |= substitute_operand(func, props);
            for arg in args {
                changed |= substitute_operand(arg, props);
            }
        }
        InstructionKind::CallDirect { args, .. } | InstructionKind::CallNamed { args, .. } => {
            for arg in args {
                changed |= substitute_operand(arg, props);
            }
        }
        InstructionKind::CallVirtual { obj, args, .. }
        | InstructionKind::CallVirtualNamed { obj, args, .. } => {
            changed |= substitute_operand(obj, props);
            for arg in args {
                changed |= substitute_operand(arg, props);
            }
        }
        InstructionKind::RuntimeCall { args, .. } => {
            for arg in args {
                changed |= substitute_operand(arg, props);
            }
        }
        InstructionKind::FloatToInt { src, .. }
        | InstructionKind::BoolToInt { src, .. }
        | InstructionKind::IntToFloat { src, .. }
        | InstructionKind::FloatBits { src, .. }
        | InstructionKind::IntBitsToFloat { src, .. }
        | InstructionKind::ValueFromInt { src, .. }
        | InstructionKind::UnwrapValueInt { src, .. }
        | InstructionKind::ValueFromBool { src, .. }
        | InstructionKind::UnwrapValueBool { src, .. }
        | InstructionKind::FloatAbs { src, .. } => {
            changed |= substitute_operand(src, props);
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
        InstructionKind::Phi { sources, .. } => {
            for (_, op) in sources.iter_mut() {
                changed |= substitute_operand(op, props);
            }
        }
        InstructionKind::Refine { src, .. } => {
            changed |= substitute_operand(src, props);
        }
    }
    changed
}

/// Substitute propagated values into terminator operands and simplify
/// constant branches.
fn substitute_terminator(term: &mut Terminator, props: &HashMap<LocalId, PropValue>) -> bool {
    let mut changed = false;

    match term {
        Terminator::Return(Some(op)) => {
            changed |= substitute_operand(op, props);
        }
        Terminator::Branch {
            cond,
            then_block,
            else_block,
        } => {
            changed |= substitute_operand(cond, props);

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
                changed |= substitute_operand(op, props);
            }
            if let Some(c) = cause {
                if let Some(op) = &mut c.message {
                    changed |= substitute_operand(op, props);
                }
            }
        }
        Terminator::RaiseCustom {
            message, instance, ..
        } => {
            if let Some(op) = message {
                changed |= substitute_operand(op, props);
            }
            if let Some(op) = instance {
                changed |= substitute_operand(op, props);
            }
        }
        Terminator::RaiseInstance { instance } => {
            changed |= substitute_operand(instance, props);
        }
        Terminator::Return(None)
        | Terminator::Goto(_)
        | Terminator::Unreachable
        | Terminator::TrySetjmp { .. }
        | Terminator::Reraise => {}
    }

    changed
}
