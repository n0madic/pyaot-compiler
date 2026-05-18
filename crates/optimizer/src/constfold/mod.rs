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

use std::collections::HashMap;

use pyaot_mir::{Constant, Function, InstructionKind, Module, Operand, RuntimeFunc};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId, StringInterner};

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

        // Phase 3: Fold rt_format(constant_value, constant_spec) → Const(Str)
        changed |= fold_format_calls(func, interner);
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
        InstructionKind::Phi { dest, sources } => {
            // If every incoming source is the same Constant, the phi is
            // degenerate — replace with a `Const`. Enabled by SSA + const
            // propagation: propagate may rewrite Local sources to
            // Constants, after which a diamond CFG producing the same
            // literal on both arms folds to a single Const.
            if sources.is_empty() {
                return false;
            }
            let Operand::Constant(first) = &sources[0].1 else {
                return false;
            };
            let first = first.clone();
            let all_same = sources
                .iter()
                .all(|(_, op)| matches!(op, Operand::Constant(c) if *c == first));
            if !all_same {
                return false;
            }
            let dest = *dest;
            *kind = InstructionKind::Const { dest, value: first };
            true
        }
        InstructionKind::Refine { dest, src, .. } => {
            // A Refine carries a type annotation but no value computation —
            // if propagation has narrowed `src` to a constant, the Refine
            // collapses to a Const of that literal (the declared type is
            // implied by the literal).
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

/// Fold `rt_format(constant_value, constant_spec)` → `Const(Str)` at compile time.
///
/// Requires full-function SSA context to see through `ValueFromInt`/`ValueFromBool`
/// boxing instructions that propagation doesn't track.
fn fold_format_calls(func: &mut Function, interner: &mut StringInterner) -> bool {
    use pyaot_core_defs::runtime_func_def::RT_FORMAT;

    // Map each LocalId to its defining InstructionKind (cloned, so we can mutate later).
    let def_map: HashMap<LocalId, InstructionKind> = func
        .blocks
        .values()
        .flat_map(|b| b.instructions.iter())
        .filter_map(|inst| {
            let dest = match &inst.kind {
                InstructionKind::BoxValue { dest, .. }
                | InstructionKind::RuntimeCall { dest, .. } => *dest,
                _ => return None,
            };
            Some((dest, inst.kind.clone()))
        })
        .collect();

    let mut changed = false;

    for block in func.blocks.values_mut() {
        for inst in &mut block.instructions {
            let (dest, args) = match &inst.kind {
                InstructionKind::RuntimeCall {
                    dest,
                    func: RuntimeFunc::Call(def),
                    args,
                } if std::ptr::eq(*def, &RT_FORMAT) && args.len() == 2 => (*dest, args.clone()),
                _ => continue,
            };

            // spec must be a constant str (args[1])
            let spec_interned = match &args[1] {
                Operand::Constant(Constant::Str(s)) => *s,
                _ => continue,
            };
            let spec_str = interner.resolve(spec_interned).to_string();

            // Resolve the value operand (args[0]) to a primitive constant
            let result: Option<String> = match &args[0] {
                // Str is never boxed through ValueFromInt — passed as pointer directly
                Operand::Constant(Constant::Str(s)) => {
                    let s = interner.resolve(*s).to_string();
                    pyaot_format_shared::format_str_spec(&s, &spec_str).ok()
                }
                Operand::Local(lid) => match def_map.get(lid) {
                    // Int was boxed via BoxValue { src: Constant::Int(n), src_type: Int }
                    Some(InstructionKind::BoxValue {
                        src: Operand::Constant(Constant::Int(n)),
                        src_type: Type::Int,
                        ..
                    }) => pyaot_format_shared::format_int_spec(*n, &spec_str).ok(),
                    // Bool was boxed via BoxValue { src: Constant::Bool(b), src_type: Bool }
                    Some(InstructionKind::BoxValue {
                        src: Operand::Constant(Constant::Bool(b)),
                        src_type: Type::Bool,
                        ..
                    }) => pyaot_format_shared::format_bool_spec(*b, &spec_str).ok(),
                    // Float was boxed via BoxValue { src: Constant::Float(f), src_type: Float }
                    Some(InstructionKind::BoxValue {
                        src: Operand::Constant(Constant::Float(f)),
                        ..
                    }) => pyaot_format_shared::format_float_spec(*f, &spec_str).ok(),
                    // Float was boxed via RT_BOX_FLOAT { args: [Constant::Float(f)] } (legacy)
                    Some(InstructionKind::RuntimeCall {
                        func: RuntimeFunc::Call(def),
                        args: box_args,
                        ..
                    }) if std::ptr::eq(*def, &pyaot_core_defs::runtime_func_def::RT_BOX_FLOAT)
                        && box_args.len() == 1 =>
                    {
                        if let Operand::Constant(Constant::Float(f)) = &box_args[0] {
                            pyaot_format_shared::format_float_spec(*f, &spec_str).ok()
                        } else {
                            None
                        }
                    }
                    _ => None,
                },
                _ => None,
            };

            if let Some(result_str) = result {
                let interned = interner.intern(&result_str);
                inst.kind = InstructionKind::Const {
                    dest,
                    value: Constant::Str(interned),
                };
                changed = true;
            }
        }
    }

    changed
}
