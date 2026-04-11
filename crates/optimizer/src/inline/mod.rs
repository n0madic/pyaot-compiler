//! Function inlining optimization pass
//!
//! Inlines small functions at call sites to reduce call overhead.
//! Particularly beneficial for leaf functions with few instructions.

mod analysis;
mod remap;
mod transform;

#[cfg(test)]
mod tests;

use pyaot_mir::Module;
use pyaot_utils::StringInterner;

use crate::pass::OptimizationPass;

/// Configuration for inlining
#[derive(Debug, Clone)]
pub struct InlineConfig {
    /// Maximum instruction count for inlining consideration
    pub max_inline_size: usize,
    /// Always inline if instruction count is at or below this threshold
    pub always_inline_threshold: usize,
    /// Maximum number of transitive inlining iterations
    pub max_iterations: usize,
}

impl Default for InlineConfig {
    fn default() -> Self {
        Self {
            max_inline_size: 50,
            always_inline_threshold: 10,
            max_iterations: 3,
        }
    }
}

impl InlineConfig {
    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            max_inline_size: threshold,
            always_inline_threshold: threshold.min(10),
            ..Default::default()
        }
    }
}

/// Perform function inlining on the MIR module.
/// Returns `true` if any inlining was performed.
pub fn inline_functions(module: &mut Module, threshold: usize) -> bool {
    let config = InlineConfig::with_threshold(threshold);
    transform::inline_pass(module, &config)
}

/// Pass wrapper for function inlining.
pub struct InlinePass {
    threshold: usize,
}

impl InlinePass {
    pub fn new(threshold: usize) -> Self {
        Self { threshold }
    }
}

impl OptimizationPass for InlinePass {
    fn name(&self) -> &str {
        "inline"
    }

    fn run_once(&mut self, module: &mut Module, _interner: &mut StringInterner) -> bool {
        let config = InlineConfig::with_threshold(self.threshold);
        transform::inline_pass(module, &config)
    }

    fn is_fixpoint(&self) -> bool {
        false
    }
}
