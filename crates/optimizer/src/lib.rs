//! MIR Optimizer
//!
//! Provides optimization passes for MIR before codegen.
//! Implements devirtualization, property flattening, function inlining,
//! constant folding & propagation, and dead code elimination.

#![forbid(unsafe_code)]

pub mod constfold;
pub mod dce;
pub mod devirtualize;
pub mod flatten_properties;
pub mod inline;
pub mod peephole;

use pyaot_mir::Module;
use pyaot_utils::StringInterner;

/// Configuration for optimization passes
#[derive(Debug, Clone)]
pub struct OptimizeConfig {
    /// Enable devirtualization (replace virtual calls with direct calls)
    pub devirtualize: bool,
    /// Enable property flattening (inline trivial @property getters)
    pub flatten_properties: bool,
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
            devirtualize: true,
            flatten_properties: true,
            inline: true,
            inline_threshold: 50,
            dce: true,
            constfold: true,
        }
    }
}

/// Run all enabled optimization passes on the MIR module.
///
/// Pass order: devirtualize → flatten_properties → inline → constfold → peephole → dce
/// - Devirtualization converts virtual calls to direct calls when receiver type is known
/// - Property flattening inlines trivial getters as field accesses
/// - Inlining exposes constant expressions across function boundaries
/// - Constant folding simplifies expressions and branches
/// - Peephole simplifies local patterns (identity ops, strength reduction, box/unbox)
/// - DCE cleans up dead code left by earlier passes
pub fn optimize_module(
    module: &mut Module,
    config: &OptimizeConfig,
    interner: &mut StringInterner,
) {
    if config.devirtualize {
        devirtualize::devirtualize(module);
    }
    if config.flatten_properties {
        flatten_properties::flatten_property_getters(module);
    }
    if config.inline {
        inline::inline_functions(module, config.inline_threshold);
    }
    if config.constfold {
        constfold::fold_constants(module, interner);
    }
    if config.constfold || config.inline {
        // Peephole runs unconditionally when any optimization is active —
        // it's lightweight and cleans up patterns from both constfold and inlining
        peephole::run_peephole(module);
    }
    if config.dce {
        dce::eliminate_dead_code(module);
    }
}
