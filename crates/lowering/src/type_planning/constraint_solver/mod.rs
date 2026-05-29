//! Constraint-based type inference solver.
//!
//! This module is the sole type planner (it replaced the legacy
//! `build_lowering_seed_info` 22-pass fixpoint). It runs a three-phase
//! pipeline:
//!
//! 1. **Collect** ([`collect`], one HIR walk): a single walker emits a flat
//!    [`Vec<Constraint>`] plus a reverse-index `TypeKey →
//!    IndexSet<ConstraintId>` (dependents map). No type resolution
//!    happens here.
//!
//! 2. **Solve** ([`Solver::run`], worklist + monotone JOIN): each `env[k]`
//!    only ever moves upward in the `TypeLattice`. Because the lattice has
//!    finite height (with a depth cap on container/union nesting), the
//!    worklist converges without the legacy `cap=10` iteration hack.
//!
//! 3. **Materialize** ([`materialize`]) + **wire-in** ([`wire_in::run`]):
//!    write solved env values into the contract outputs `LoweringSeedInfo`
//!    exposes (`expr.ty`, `func_return_types`, `base_var_types`,
//!    `lambda_param_type_hints`, `closure_capture_types`), preserving the
//!    existing "don't cache Union/Any" gate. The per-function local view is
//!    owned by the post-desugar prescan, not this module — see the note in
//!    [`wire_in::apply_to_lowering`].

pub mod collect;
pub mod env;
pub mod key;
pub mod materialize;
pub mod solve;
pub mod vocab;
pub mod wire_in;

// `run_constraint_solver` is the module's only cross-module entry point
// (called by `type_planning::build_lowering_seed_info`). All other solver
// types are accessed by sibling modules via their defining-module paths
// (`super::vocab::…`, `super::solve::…`), so no further re-exports are needed.
pub(crate) use wire_in::run as run_constraint_solver;
