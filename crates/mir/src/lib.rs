//! # MIR — Mid-level IR (CFG), representation-typed
//!
//! **Scaffold.** Every MIR value carries a [`pyaot_types::Repr`] **by value, not
//! `Option`**: there is exactly one representation field and it is total. (A dual
//! logical/physical type field with an optional, dual-meaning sentinel is the
//! anti-pattern this design exists to prevent — see PITFALLS A1.)
//!
//! Responsibilities:
//! * parameterized "kind" enums (`PrintKind`, `CompareKind`, …) so runtime ops
//!   don't explode into one `RuntimeFunc` variant each;
//! * `Instruction` / `Terminator` shapes;
//! * [`verify`] — mandatory from commit #1, run in debug at *every* pass
//!   boundary; rejects any instruction whose operand/result `Repr`s violate its
//!   typed signature.

#![forbid(unsafe_code)]

pub mod verify {
    //! MIR verifier — checks `Repr` consistency at every pass boundary. `Repr`
    //! is the sole representation type, so there are no widening exceptions to
    //! bridge a second (logical) type field.
}
