//! Function cost computation for inlining decisions.
//!
//! The call graph itself is the canonical `crate::call_graph::CallGraph`
//! — inlining doesn't keep its own copy. `FunctionCost::compute` takes
//! the graph as an arg and queries `is_recursive` / direct-callee
//! iteration through it.

use pyaot_mir::{Function, InstructionKind, Terminator};

use super::InlineConfig;
use crate::call_graph::CallGraph;

/// Cost metrics for a function to determine inlining eligibility
#[derive(Debug, Clone)]
pub struct FunctionCost {
    /// Total instruction count across all blocks
    pub instruction_count: usize,
    /// Number of basic blocks
    pub block_count: usize,
    /// Whether the function has GC roots
    pub has_gc_roots: bool,
    /// Whether the function has exception handling
    pub has_exception_handling: bool,
    /// Whether the function is recursive
    pub is_recursive: bool,
    /// Whether the function is a generator (has $resume suffix)
    pub is_generator: bool,
    /// Whether the function has calls that cannot be inlined (CallNamed, CallVirtual)
    pub has_uninlinable_calls: bool,
}

impl FunctionCost {
    /// Compute cost metrics for a function
    pub fn compute(func: &Function, call_graph: &CallGraph) -> Self {
        let mut instruction_count = 0;
        let mut has_gc_roots = false;
        let mut has_exception_handling = false;
        let mut has_uninlinable_calls = false;

        // Check for GC roots in locals
        for local in func.locals.values() {
            if local.is_gc_root {
                has_gc_roots = true;
                break;
            }
        }

        // Check parameters for GC roots
        for param in &func.params {
            if param.is_gc_root {
                has_gc_roots = true;
                break;
            }
        }

        // Analyze blocks
        for block in func.blocks.values() {
            instruction_count += block.instructions.len();

            for instr in &block.instructions {
                match &instr.kind {
                    // Exception handling instructions
                    InstructionKind::ExcPushFrame { .. }
                    | InstructionKind::ExcPopFrame
                    | InstructionKind::ExcGetType { .. }
                    | InstructionKind::ExcClear
                    | InstructionKind::ExcHasException { .. }
                    | InstructionKind::ExcGetCurrent { .. }
                    | InstructionKind::ExcCheckType { .. }
                    | InstructionKind::ExcCheckClass { .. }
                    | InstructionKind::ExcStartHandling
                    | InstructionKind::ExcEndHandling => {
                        has_exception_handling = true;
                    }
                    // GC instructions indicate heap allocation
                    InstructionKind::GcPush { .. } | InstructionKind::GcPop => {
                        has_gc_roots = true;
                    }
                    // Uninlinable call types
                    InstructionKind::CallNamed { .. }
                    | InstructionKind::CallVirtual { .. }
                    | InstructionKind::CallVirtualNamed { .. } => {
                        has_uninlinable_calls = true;
                    }
                    _ => {}
                }
            }

            // Check terminator for exception handling
            if matches!(
                block.terminator,
                Terminator::Raise { .. }
                    | Terminator::RaiseCustom { .. }
                    | Terminator::RaiseInstance { .. }
                    | Terminator::Reraise
                    | Terminator::TrySetjmp { .. }
            ) {
                has_exception_handling = true;
            }
        }

        let is_recursive = call_graph.is_recursive(func.id);
        let is_generator = func.name.ends_with("$resume");

        FunctionCost {
            instruction_count,
            block_count: func.blocks.len(),
            has_gc_roots,
            has_exception_handling,
            is_recursive,
            is_generator,
            has_uninlinable_calls,
        }
    }

    /// Determine if function should be inlined based on cost metrics and config
    pub fn should_inline(&self, config: &InlineConfig) -> InlineDecision {
        // Never inline generators
        if self.is_generator {
            return InlineDecision::Never("generator function");
        }

        // Never inline recursive functions
        if self.is_recursive {
            return InlineDecision::Never("recursive function");
        }

        // Never inline functions with exception handling (too complex)
        if self.has_exception_handling {
            return InlineDecision::Never("has exception handling");
        }

        // Never inline if too large
        if self.instruction_count > config.max_inline_size {
            return InlineDecision::Never("too many instructions");
        }

        // Always inline small leaf functions without uninlinable calls
        if self.instruction_count <= config.always_inline_threshold
            && self.block_count == 1
            && !self.has_gc_roots
            && !self.has_uninlinable_calls
        {
            return InlineDecision::Always;
        }

        // Consider inlining medium-sized functions without GC
        if self.instruction_count <= config.max_inline_size && !self.has_gc_roots {
            return InlineDecision::Consider;
        }

        // Consider inlining small functions even with GC
        if self.instruction_count <= config.always_inline_threshold {
            return InlineDecision::Consider;
        }

        InlineDecision::Never("has GC roots and exceeds always-inline threshold")
    }
}

/// Decision on whether to inline a function
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineDecision {
    /// Always inline this function
    Always,
    /// Consider inlining (based on call site context)
    Consider,
    /// Never inline (with reason)
    Never(&'static str),
}
