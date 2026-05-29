//! Constraint solver key namespace — every solvable type-bearing entity is
//! addressable through a [`TypeKey`]. The solver's environment is a map
//! `TypeKey → Type`, and every constraint is a relation between keys.
//!
//! `TypeKey` is intentionally a closed enum: every variant identifies a
//! location whose type the solver computes. Internal anonymous metavariables
//! use [`TypeKey::Meta`] (a u32 counter), and are strictly separate from
//! user-level `Type::Var(InternedString)` TypeVars (S3.3a).

use pyaot_hir::ExprId;
use pyaot_utils::{ClassId, FuncId, InternedString, VarId};

/// Identifier for every solvable type-bearing entity in the program.
///
/// The solver materializes its `Env<TypeKey, Type>` map into the downstream
/// `LoweringSeedInfo` contract:
/// - [`TypeKey::Expr`] → `hir::Expr.ty` (filtered by the existing cache gate)
/// - [`TypeKey::Var`] → `LoweringSeedInfo::base_var_types`
/// - [`TypeKey::FuncReturn`] → `Lowering::func_return_types`
/// - [`TypeKey::LambdaParam`] → `Lowering::closures.lambda_param_type_hints`
/// - [`TypeKey::Capture`] → `Lowering::closures.closure_capture_types`
/// - [`TypeKey::ClassField`] → `LoweringSeedInfo::refined_class_field_types`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKey {
    /// The type of an HIR expression node.
    Expr(ExprId),
    /// The type of a named local or module-level variable.
    Var(VarId),
    /// The return type of a function (a single key per function — return
    /// statements join into this key).
    FuncReturn(FuncId),
    /// The yield type of a generator function — distinct from return type
    /// so the Phase-A mini-solver (D4) can collect it independently before
    /// generator desugaring runs.
    FuncYield(FuncId),
    /// A class field's type, refined across all observed stores.
    ClassField(ClassId, InternedString),
    /// A captured upvalue slot inside a closure body.
    Capture(FuncId, usize),
    /// A lambda parameter — populated by call-site hints feeding back
    /// into the lambda body's parameter type.
    LambdaParam(FuncId, usize),
    /// Anonymous internal metavariable. The solver allocates these for
    /// intermediate results that have no HIR address (e.g. the LHS of a
    /// chained reducer). Strictly distinct from user-level
    /// `Type::Var(InternedString)` TypeVars — Meta keys are erased by
    /// materialization and never surface in downstream IR.
    Meta(u32),
}

impl TypeKey {
    /// True iff this key is solver-internal and must NOT be materialized
    /// into any downstream contract output. Test-only: materialization
    /// discriminates internal keys structurally via its `match` rather than
    /// this predicate.
    #[cfg(test)]
    pub fn is_internal(self) -> bool {
        matches!(self, TypeKey::Meta(_))
    }
}
