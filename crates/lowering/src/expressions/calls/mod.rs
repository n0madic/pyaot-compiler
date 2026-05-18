//! Call expression lowering: function calls and class instantiation
//!
//! Organized into submodules by call kind:
//! - `args`: argument expansion and lowering helpers
//! - `direct`: main `lower_call` dispatcher
//! - `closure`: closure, wrapper, and indirect calls
//! - `class`: class instantiation and imported function calls

mod args;
mod class;
mod closure;
mod direct;

/// Represents an expanded call argument.
/// Used to track whether an argument needs runtime unpacking.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ExpandedArg {
    /// Regular argument - lower normally
    Regular(pyaot_hir::ExprId),
    /// Runtime tuple unpacking - extract elements at runtime
    RuntimeUnpackTuple(pyaot_hir::ExprId),
    /// Runtime list unpacking - extract elements at runtime
    RuntimeUnpackList(pyaot_hir::ExprId),
}
