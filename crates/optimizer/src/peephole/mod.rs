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
use pyaot_utils::FuncId;

/// Maximum iterations to prevent pathological cases.
const MAX_ITERATIONS: usize = 10;

/// Run peephole optimizations on all functions in the module.
pub fn run_peephole(module: &mut Module) {
    let func_ids: Vec<FuncId> = module.functions.keys().copied().collect();

    for func_id in func_ids {
        let func = match module.functions.get_mut(&func_id) {
            Some(f) => f,
            None => continue,
        };
        optimize_function(func);
    }
}

fn optimize_function(func: &mut Function) {
    for _ in 0..MAX_ITERATIONS {
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

        if !changed {
            break;
        }
    }
}
