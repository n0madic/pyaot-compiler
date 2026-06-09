//! # lowering — HIR → MIR (mechanical)
//!
//! **Scaffold.** Types are already solved by `pyaot_typeck`, so lowering is
//! purely mechanical: translate HIR nodes to MIR, asking [`pyaot_types::repr_of`]
//! for each slot's representation. Two things deliberately do NOT exist here:
//!
//! * **No type inference** — it finished in `typeck`.
//! * **No ABI-repair stage** — a function's ABI is a deterministic function of
//!   its parameters' `Repr`, so call sites are correct by construction; there is
//!   no need to rewrite signatures after the fact.
//!
//! [`legalize`] is the SINGLE place coercions are inserted (box/unbox/tag/untag/
//! numeric-widen). One rule — `coerce(have: Repr, need: Repr)` — subsumes every
//! per-case boxing decision (see PITFALLS A5).

#![forbid(unsafe_code)]

pub mod legalize {
    //! Representation legalization: the one and only coercion-insertion pass.
    //! Given a value with representation `have` at a use-site needing `need`,
    //! insert exactly the bridging op (box / unbox / tag / untag / int→float).
    //! No other part of the compiler may emit a boxing coercion.
}
