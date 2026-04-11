//! Constant Folding & Propagation optimization pass
//!
//! 1. **Constant propagation**: Replace uses of single-definition constant locals
//!    with their constant values. Simplify constant branches to unconditional jumps.
//! 2. **Constant folding**: Evaluate binary/unary ops and type conversions on
//!    constant operands at compile time.
//! 3. Iterate to fixpoint (propagation may enable new folding and vice versa).

pub(crate) mod fold;
pub(crate) mod propagate;

#[cfg(test)]
mod tests;

use pyaot_mir::{InstructionKind, Module, Operand};
use pyaot_utils::{FuncId, StringInterner};

use crate::pass::OptimizationPass;

use fold::{
    try_fold_binop, try_fold_bool_to_int, try_fold_float_abs, try_fold_float_to_int,
    try_fold_int_to_float, try_fold_unop,
};

/// Maximum iterations to prevent pathological cases.
const MAX_ITERATIONS: usize = 10;

/// Run one iteration of constant folding and propagation on all functions.
/// Returns `true` if any changes were made.
pub(crate) fn fold_constants_once(module: &mut Module, interner: &mut StringInterner) -> bool {
    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();
    let mut changed = false;

    for func_id in func_ids {
        let func = match module.functions.get_mut(&func_id) {
            Some(f) => f,
            None => continue,
        };

        // Phase 1: Propagate known constants into operands
        changed |= propagate::propagate_constants(func);

        // Phase 2: Fold constant expressions in instructions
        for block in func.blocks.values_mut() {
            for inst in &mut block.instructions {
                changed |= try_fold_instruction(&mut inst.kind, interner);
            }
        }
    }

    changed
}

/// Run constant folding and propagation on all functions in the module.
pub fn fold_constants(module: &mut Module, interner: &mut StringInterner) {
    for _ in 0..MAX_ITERATIONS {
        if !fold_constants_once(module, interner) {
            break;
        }
    }
}

/// Pass wrapper for constant folding and propagation.
pub struct ConstantFoldPass;

impl OptimizationPass for ConstantFoldPass {
    fn name(&self) -> &str {
        "constfold"
    }

    fn run_once(&mut self, module: &mut Module, interner: &mut StringInterner) -> bool {
        fold_constants_once(module, interner)
    }

    fn max_iterations(&self) -> usize {
        MAX_ITERATIONS
    }
}

/// Try to fold a single instruction into a Const if all operands are constant.
/// Returns true if the instruction was replaced.
fn try_fold_instruction(kind: &mut InstructionKind, interner: &mut StringInterner) -> bool {
    match kind {
        InstructionKind::BinOp {
            dest,
            op,
            left,
            right,
        } => {
            if let (Operand::Constant(lc), Operand::Constant(rc)) = (left, right) {
                if let Some(result) = try_fold_binop(*op, lc, rc, interner) {
                    let dest = *dest;
                    *kind = InstructionKind::Const {
                        dest,
                        value: result,
                    };
                    return true;
                }
            }
            false
        }
        InstructionKind::UnOp { dest, op, operand } => {
            if let Operand::Constant(c) = operand {
                if let Some(result) = try_fold_unop(*op, c) {
                    let dest = *dest;
                    *kind = InstructionKind::Const {
                        dest,
                        value: result,
                    };
                    return true;
                }
            }
            false
        }
        InstructionKind::BoolToInt { dest, src } => {
            if let Operand::Constant(c) = src {
                if let Some(result) = try_fold_bool_to_int(c) {
                    let dest = *dest;
                    *kind = InstructionKind::Const {
                        dest,
                        value: result,
                    };
                    return true;
                }
            }
            false
        }
        InstructionKind::IntToFloat { dest, src } => {
            if let Operand::Constant(c) = src {
                if let Some(result) = try_fold_int_to_float(c) {
                    let dest = *dest;
                    *kind = InstructionKind::Const {
                        dest,
                        value: result,
                    };
                    return true;
                }
            }
            false
        }
        InstructionKind::FloatToInt { dest, src } => {
            if let Operand::Constant(c) = src {
                if let Some(result) = try_fold_float_to_int(c) {
                    let dest = *dest;
                    *kind = InstructionKind::Const {
                        dest,
                        value: result,
                    };
                    return true;
                }
            }
            false
        }
        InstructionKind::FloatAbs { dest, src } => {
            if let Operand::Constant(c) = src {
                if let Some(result) = try_fold_float_abs(c) {
                    let dest = *dest;
                    *kind = InstructionKind::Const {
                        dest,
                        value: result,
                    };
                    return true;
                }
            }
            false
        }
        InstructionKind::Copy { dest, src } => {
            // Copy of a constant → Const
            if let Operand::Constant(c) = src {
                let dest = *dest;
                let value = c.clone();
                *kind = InstructionKind::Const { dest, value };
                return true;
            }
            false
        }
        _ => false,
    }
}
