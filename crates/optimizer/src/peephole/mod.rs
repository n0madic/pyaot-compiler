//! Peephole Optimizations
//!
//! Local pattern-based simplifications on MIR instructions:
//! - **Identity elimination**: `x + 0`, `x - 0`, `x * 1`, `x // 1`, `x | 0`, `x & -1`,
//!   `x ^ 0`, `x << 0`, `x >> 0` → `x`
//! - **Zero/absorbing**: `x * 0` → `0`, `x & 0` → `0`, `x | -1` → `-1`
//! - **Strength reduction**: `x * 2` → `x + x` (avoids multiply),
//!   `x * 2^n` → `x << n`, `x // 2^n` → `x >> n` (for positive divisors)
//! - **Double negation**: `--x` → `x`, `not not x` → `x`, `~~x` → `x`
//! - **Redundant conversion**: `BoolToInt(IntToFloat(...))` chains, `FloatBits(IntBitsToFloat(x))` → `x`
//! - **Box/unbox elimination**: `UnboxInt(BoxInt(x))` → `x`, same for Float/Bool

pub(crate) mod patterns;

#[cfg(test)]
mod tests;

use pyaot_mir::{Function, Module};
use pyaot_utils::{FuncId, StringInterner};

use crate::pass::OptimizationPass;

/// Maximum iterations to prevent pathological cases.
const MAX_ITERATIONS: usize = 10;

/// Run one iteration of peephole optimizations on all functions.
/// Returns `true` if any changes were made.
pub(crate) fn peephole_once(module: &mut Module) -> bool {
    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();
    let mut changed = false;

    for func_id in func_ids {
        let func = match module.functions.get_mut(&func_id) {
            Some(f) => f,
            None => continue,
        };
        changed |= optimize_function_once(func);
    }

    changed
}

/// Run peephole optimizations on all functions in the module.
pub fn run_peephole(module: &mut Module) {
    for _ in 0..MAX_ITERATIONS {
        if !peephole_once(module) {
            break;
        }
    }
}

fn optimize_function_once(func: &mut Function) -> bool {
    let mut changed = false;

    // Single-instruction peepholes
    for block in func.blocks.values_mut() {
        for inst in &mut block.instructions {
            changed |= patterns::simplify_instruction(&mut inst.kind);
        }
    }

    // Two-instruction peepholes (box/unbox, double negation, redundant conversion)
    for block in func.blocks.values_mut() {
        changed |= patterns::simplify_pairs(&mut block.instructions);
    }

    changed
}

/// Pass wrapper for peephole optimizations.
pub struct PeepholePass;

impl OptimizationPass for PeepholePass {
    fn name(&self) -> &str {
        "peephole"
    }

    fn run_once(&mut self, module: &mut Module, _interner: &mut StringInterner) -> bool {
        peephole_once(module)
    }

    fn max_iterations(&self) -> usize {
        MAX_ITERATIONS
    }
}
