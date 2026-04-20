//! Match statement lowering from HIR to MIR
//!
//! Desugars match statements into if/elif chains. Each match case is converted into
//! a conditional check that tests whether the pattern matches, binds any captured
//! variables, and executes the case body if the pattern matches.
//!
//! Split into focused submodules:
//! - `patterns`: Pattern check generation (sequence, mapping, class, or, value)
//! - `binding`: Equality checks and variable binding helpers

mod binding;
mod patterns;

use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

/// Result type for pattern check: (condition_operand, bindings)
/// Bindings are (VarId, Operand, Type) tuples to be assigned
pub(crate) type PatternCheckResult = (mir::Operand, Vec<(pyaot_utils::VarId, mir::Operand, Type)>);

/// Context for pattern checking, grouping common parameters
pub(super) struct PatternContext<'a> {
    pub(super) subject: mir::Operand,
    pub(super) subject_type: &'a Type,
    pub(super) hir_module: &'a hir::Module,
}
