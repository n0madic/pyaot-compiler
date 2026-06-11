//! # Two-layer type system
//!
//! The single most important architectural decision of this compiler. A type
//! system that lets **one** lattice answer both "what Python type is this?" and,
//! implicitly, "how is it stored in a slot/register?" forces a
//! representation-ambiguous `Any` type and a dual logical/physical field — the
//! deep trap described in `PITFALLS.md` (A1). We keep the two questions in two
//! types that never merge:
//!
//! * [`SemTy`] — the **semantic** (Python-level) type. Used by type inference,
//!   dispatch decisions, and diagnostics. Gradual-typing's "dynamic" is the
//!   explicit [`SemTy::Dyn`] — there is no `Any`/`HeapAny` pair.
//!
//! * [`Repr`] — the **physical representation**. Used by lowering, the MIR
//!   verifier, and codegen. It is **mandatory** (never `Option`): every value
//!   has exactly one representation, computed by the single boundary function
//!   [`repr_of`].
//!
//! ## Invariants (the constitution)
//!
//! 1. Representation is a *function of* `SemTy`, computed at one boundary
//!    ([`repr_of`]), never re-derived ad hoc downstream.
//! 2. The default representation is always *correct*: anything unproven
//!    ([`SemTy::Dyn`], [`SemTy::Union`], [`SemTy::Var`], bignum-capable
//!    [`SemTy::Int`]) maps to [`Repr::Tagged`]. Unboxed [`Repr::Raw`] is an
//!    *optimization* applied only when typeck proves it safe — never a default
//!    that can silently corrupt memory.
//! 3. GC-rootness is derived purely from `Repr` (see [`Repr::is_gc_root`]);
//!    there is no separate, drift-prone `is_gc_root` flag.

#![forbid(unsafe_code)]

pub mod builtin_classes;
pub mod lattice;
pub mod repr;
pub mod sem;

pub use lattice::{ClassHierarchy, NoClasses, TypeLattice};
pub use repr::{repr_of, sig_repr, HeapShape, RawKind, Repr, SigRepr, RAW_I64_NARROW_BOUND};
pub use sem::{SemTy, Sig};
