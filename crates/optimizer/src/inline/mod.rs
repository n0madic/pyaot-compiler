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

/// Perform function inlining on the MIR module
///
/// # Arguments
/// * `module` - The MIR module to optimize
/// * `threshold` - Maximum instruction count for inlining consideration
pub fn inline_functions(module: &mut Module, threshold: usize) {
    let config = InlineConfig::with_threshold(threshold);
    transform::inline_pass(module, &config);
}
