//! Pass management infrastructure for MIR optimization.
//!
//! Defines the `OptimizationPass` trait and `PassManager` that orchestrate
//! optimization passes over MIR modules. Each pass implements `run_once`
//! which performs a single iteration; the PassManager handles fixpoint
//! iteration for passes that require it.

use pyaot_mir::Module;
use pyaot_utils::StringInterner;

use crate::OptimizeConfig;

/// Trait for optimization passes that transform MIR modules.
///
/// Implementations must preserve `Instruction::span` when transforming
/// instructions — only `InstructionKind` should be modified.
pub trait OptimizationPass {
    /// Human-readable name for logging and debugging.
    fn name(&self) -> &str;

    /// Run one iteration of the pass. Returns `true` if any changes were made.
    fn run_once(&mut self, module: &mut Module, interner: &mut StringInterner) -> bool;

    /// Maximum number of fixpoint iterations (default: 10).
    fn max_iterations(&self) -> usize {
        10
    }

    /// Whether the PassManager should iterate this pass to fixpoint.
    /// If `false`, `run_once` is called exactly once.
    fn is_fixpoint(&self) -> bool {
        true
    }
}

/// Orchestrates a sequence of optimization passes over a MIR module.
pub struct PassManager {
    passes: Vec<Box<dyn OptimizationPass>>,
}

impl PassManager {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    /// Add a pass to the end of the pipeline.
    pub fn add_pass(&mut self, pass: Box<dyn OptimizationPass>) {
        self.passes.push(pass);
    }

    /// Number of registered passes.
    pub fn len(&self) -> usize {
        self.passes.len()
    }

    /// Whether the pipeline has no passes.
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    /// Run all registered passes sequentially.
    /// Fixpoint passes iterate until stable or `max_iterations` is reached.
    pub fn run(&mut self, module: &mut Module, interner: &mut StringInterner) {
        for pass in &mut self.passes {
            if pass.is_fixpoint() {
                let max = pass.max_iterations();
                for _ in 0..max {
                    if !pass.run_once(module, interner) {
                        break;
                    }
                }
            } else {
                pass.run_once(module, interner);
            }
        }
    }

    /// Returns the names of all registered passes in order.
    pub fn pass_names(&self) -> Vec<&str> {
        self.passes.iter().map(|p| p.name()).collect()
    }
}

impl Default for PassManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the standard optimization pipeline based on configuration flags.
///
/// Pass order: devirtualize → flatten_properties → inline → constfold → peephole → dce
pub fn build_pass_pipeline(config: &OptimizeConfig) -> PassManager {
    let mut pm = PassManager::new();
    if config.devirtualize {
        pm.add_pass(Box::new(crate::devirtualize::DevirtualizePass));
    }
    if config.flatten_properties {
        pm.add_pass(Box::new(crate::flatten_properties::FlattenPropertiesPass));
    }
    if config.inline {
        pm.add_pass(Box::new(crate::inline::InlinePass::new(
            config.inline_threshold,
        )));
    }
    if config.constfold {
        pm.add_pass(Box::new(crate::constfold::ConstantFoldPass));
    }
    if config.constfold || config.inline {
        pm.add_pass(Box::new(crate::peephole::PeepholePass));
    }
    if config.dce {
        pm.add_pass(Box::new(crate::dce::DcePass));
    }
    pm
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Mock pass that counts invocations and returns `changed` for a configurable
    /// number of iterations before returning `false`.
    struct MockPass {
        name: &'static str,
        changes_remaining: Arc<Mutex<usize>>,
        call_count: Arc<Mutex<usize>>,
        fixpoint: bool,
        max_iter: usize,
    }

    impl MockPass {
        fn new(
            name: &'static str,
            changes_remaining: usize,
            fixpoint: bool,
            max_iter: usize,
        ) -> (Self, Arc<Mutex<usize>>) {
            let call_count = Arc::new(Mutex::new(0));
            let pass = Self {
                name,
                changes_remaining: Arc::new(Mutex::new(changes_remaining)),
                call_count: Arc::clone(&call_count),
                fixpoint,
                max_iter,
            };
            (pass, call_count)
        }
    }

    impl OptimizationPass for MockPass {
        fn name(&self) -> &str {
            self.name
        }

        fn run_once(&mut self, _module: &mut Module, _interner: &mut StringInterner) -> bool {
            *self.call_count.lock().unwrap() += 1;
            let mut remaining = self.changes_remaining.lock().unwrap();
            if *remaining > 0 {
                *remaining -= 1;
                true
            } else {
                false
            }
        }

        fn max_iterations(&self) -> usize {
            self.max_iter
        }

        fn is_fixpoint(&self) -> bool {
            self.fixpoint
        }
    }

    #[test]
    fn test_empty_pass_manager_is_noop() {
        let mut pm = PassManager::new();
        let mut module = Module::new();
        let mut interner = StringInterner::new();
        pm.run(&mut module, &mut interner);
        assert!(pm.is_empty());
        assert_eq!(pm.len(), 0);
    }

    #[test]
    fn test_non_fixpoint_pass_runs_once() {
        let (pass, call_count) = MockPass::new("single-shot", 5, false, 1);
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(pass));

        let mut module = Module::new();
        let mut interner = StringInterner::new();
        pm.run(&mut module, &mut interner);

        assert_eq!(*call_count.lock().unwrap(), 1);
    }

    #[test]
    fn test_fixpoint_pass_iterates_until_stable() {
        // Pass reports changes for 3 iterations, then converges
        let (pass, call_count) = MockPass::new("converging", 3, true, 10);
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(pass));

        let mut module = Module::new();
        let mut interner = StringInterner::new();
        pm.run(&mut module, &mut interner);

        // 3 iterations returning true + 1 returning false = 4 calls
        assert_eq!(*call_count.lock().unwrap(), 4);
    }

    #[test]
    fn test_fixpoint_pass_respects_max_iterations() {
        // Pass always reports changes — should stop at max_iterations
        let (pass, call_count) = MockPass::new("never-converges", 100, true, 5);
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(pass));

        let mut module = Module::new();
        let mut interner = StringInterner::new();
        pm.run(&mut module, &mut interner);

        assert_eq!(*call_count.lock().unwrap(), 5);
    }

    #[test]
    fn test_pass_names() {
        let (p1, _) = MockPass::new("alpha", 0, false, 1);
        let (p2, _) = MockPass::new("beta", 0, true, 10);
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(p1));
        pm.add_pass(Box::new(p2));

        assert_eq!(pm.pass_names(), vec!["alpha", "beta"]);
        assert_eq!(pm.len(), 2);
    }

    #[test]
    fn test_build_pipeline_all_enabled() {
        let config = OptimizeConfig::default();
        let pm = build_pass_pipeline(&config);

        assert_eq!(
            pm.pass_names(),
            vec![
                "devirtualize",
                "flatten-properties",
                "inline",
                "constfold",
                "peephole",
                "dce"
            ]
        );
    }

    #[test]
    fn test_build_pipeline_all_disabled() {
        let config = OptimizeConfig {
            devirtualize: false,
            flatten_properties: false,
            inline: false,
            inline_threshold: 50,
            dce: false,
            constfold: false,
        };
        let pm = build_pass_pipeline(&config);

        assert!(pm.is_empty());
    }

    #[test]
    fn test_build_pipeline_peephole_with_inline_only() {
        let config = OptimizeConfig {
            devirtualize: false,
            flatten_properties: false,
            inline: true,
            inline_threshold: 50,
            dce: false,
            constfold: false,
        };
        let pm = build_pass_pipeline(&config);

        // inline + peephole (peephole auto-enabled when inline is active)
        assert_eq!(pm.pass_names(), vec!["inline", "peephole"]);
    }

    #[test]
    fn test_build_pipeline_peephole_with_constfold_only() {
        let config = OptimizeConfig {
            devirtualize: false,
            flatten_properties: false,
            inline: false,
            inline_threshold: 50,
            dce: false,
            constfold: true,
        };
        let pm = build_pass_pipeline(&config);

        // constfold + peephole
        assert_eq!(pm.pass_names(), vec!["constfold", "peephole"]);
    }
}
