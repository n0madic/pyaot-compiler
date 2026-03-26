//! MIR Optimizer
//!
//! Provides optimization passes for MIR before codegen.
//! Currently implements function inlining and dead code elimination.

#![forbid(unsafe_code)]

pub mod dce;
pub mod inline;

use pyaot_mir::Module;

/// Configuration for optimization passes
#[derive(Debug, Clone)]
pub struct OptimizeConfig {
    /// Enable function inlining
    pub inline: bool,
    /// Maximum instruction count for inlining consideration
    pub inline_threshold: usize,
    /// Enable dead code elimination
    pub dce: bool,
}

impl Default for OptimizeConfig {
    fn default() -> Self {
        Self {
            inline: true,
            inline_threshold: 50,
            dce: true,
        }
    }
}

/// Run all enabled optimization passes on the MIR module
pub fn optimize_module(module: &mut Module, config: &OptimizeConfig) {
    if config.inline {
        inline::inline_functions(module, config.inline_threshold);
    }
    if config.dce {
        dce::eliminate_dead_code(module);
    }
}
