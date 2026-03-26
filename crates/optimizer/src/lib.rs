//! MIR Optimizer
//!
//! Provides optimization passes for MIR before codegen.
//! Implements function inlining, constant folding & propagation,
//! and dead code elimination.

#![forbid(unsafe_code)]

pub mod constfold;
pub mod dce;
pub mod inline;

use pyaot_mir::Module;
use pyaot_utils::StringInterner;

/// Configuration for optimization passes
#[derive(Debug, Clone)]
pub struct OptimizeConfig {
    /// Enable function inlining
    pub inline: bool,
    /// Maximum instruction count for inlining consideration
    pub inline_threshold: usize,
    /// Enable dead code elimination
    pub dce: bool,
    /// Enable constant folding and propagation
    pub constfold: bool,
}

impl Default for OptimizeConfig {
    fn default() -> Self {
        Self {
            inline: true,
            inline_threshold: 50,
            dce: true,
            constfold: true,
        }
    }
}

/// Run all enabled optimization passes on the MIR module.
///
/// Pass order: inline → constfold → dce
/// - Inlining exposes constant expressions across function boundaries
/// - Constant folding simplifies expressions and branches
/// - DCE cleans up dead code left by folding and inlining
pub fn optimize_module(
    module: &mut Module,
    config: &OptimizeConfig,
    interner: &mut StringInterner,
) {
    if config.inline {
        inline::inline_functions(module, config.inline_threshold);
    }
    if config.constfold {
        constfold::fold_constants(module, interner);
    }
    if config.dce {
        dce::eliminate_dead_code(module);
    }
}
