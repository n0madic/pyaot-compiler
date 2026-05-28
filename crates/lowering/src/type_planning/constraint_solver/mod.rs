//! Constraint-based type inference solver — S1 skeleton.
//!
//! This module is the foundation of the type-planner rewrite. It replaces
//! the legacy `build_lowering_seed_info` fixpoint loop with a three-phase
//! pipeline:
//!
//! 1. **Collect** (one HIR walk): a single walker emits a flat
//!    [`Vec<Constraint>`] plus a reverse-index `TypeKey →
//!    IndexSet<ConstraintId>` (dependents map). No type resolution
//!    happens here.
//!
//! 2. **Solve** (worklist + monotone JOIN): each `env[k]` only ever moves
//!    upward in the `TypeLattice`. Because the lattice has finite height,
//!    the worklist converges without the legacy `cap=10` hack.
//!
//! 3. **Materialize**: write solved env values into the 5 contract outputs
//!    `LoweringSeedInfo` exposes (`expr.ty`, `func_return_types`,
//!    `base_var_types`, `lambda_param_type_hints`, `closure_capture_types`),
//!    preserving the existing "don't cache Union/Any" gate.
//!
//! ## S1 scope (current commit)
//!
//! - Vocabulary types: [`TypeKey`], [`Constraint`], [`CalleeRef`],
//!   [`ContainerKind`], [`CompTemp`], [`ConstraintId`], [`BuiltinId`].
//! - Solver environment ([`Env`]) with monotone `join_into` and unit-tested
//!   lattice properties (monotonicity, idempotency, bottom-stability,
//!   numeric-tower, container covariance, union construction).
//! - Solver struct ([`Solver`]) with constraint storage, dependents map,
//!   and `add()` API. `run()` is `unimplemented!()` and is filled in
//!   during S2.
//!
//! ## Not in scope for S1
//!
//! - HIR walker (`collect.rs`) — S2.
//! - Reducer implementations (`evaluate_constraint`) — S2/S3.
//! - Materialization (`materialize.rs`) — S4.
//! - Wire-in to `build_lowering_seed_info` — S5.
//! - Deletion of the legacy passes — S5.

// S1 scaffolding: most variants/fields are unused until S2-S5 wire in the
// collector, reducers, and materialization. Suppress dead-code and
// unused-import warnings at the module root rather than annotating every
// item individually. The `pub use` re-exports below define the solver's
// public API surface for the rest of the compiler — they're "unused"
// only because their callers don't exist yet.
#![allow(dead_code, unused_imports)]

pub mod collect;
pub mod env;
pub mod key;
pub mod materialize;
pub mod solve;
pub mod vocab;
pub mod wire_in;

pub use collect::{collect, Collector};
pub use env::Env;
pub use key::{CompTemp, TypeKey};
pub use materialize::{materialize, MaterializeOutput};
pub use solve::{PermissiveCtx, ReducerCtx, Solver};
pub use vocab::{BuiltinId, CalleeRef, Constraint, ConstraintId, ContainerKind};
pub(crate) use wire_in::run as run_constraint_solver;
