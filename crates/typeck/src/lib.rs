//! # typeck — one constraint-based type inference
//!
//! **Scaffold.** Type inference is ONE algorithm in three phases — never a
//! fixpoint of mutually recursive monotone passes:
//!
//! 1. **collect** — a bidirectional walk over HIR emits [`pyaot_types::SemTy`]
//!    constraints (equality / subtype / `consistent` for gradual `Dyn`).
//! 2. **solve** — a single union-find / worklist solver over
//!    [`pyaot_types::TypeLattice`].
//! 3. **materialize** — write solved `SemTy` back onto HIR nodes.
//!
//! Inference finishes BEFORE lowering and does not leak into it. Representation
//! is NOT decided here — that is `repr_of` at the lowering boundary. Because the
//! tagged baseline is always correct, inference precision is a performance lever,
//! not a correctness requirement: an underpowered solver yields slower code, not
//! wrong code.

#![forbid(unsafe_code)]
