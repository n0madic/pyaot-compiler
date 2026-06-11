//! Representation legalization: the one and only coercion-insertion point.
//!
//! Given a value with representation `have` at a use-site needing `need`,
//! [`coerce`] reports the bridging op to insert (box / unbox / tag / untag /
//! int→float / heap→tagged), or `None` if the pair is not a legal coercion. No
//! other part of the compiler may emit a boxing coercion.
//!
//! Three layers share one legality table, each with its own enforcement role:
//! the *table* lives in `pyaot-mir` ([`pyaot_mir::classify_coercion`]); the
//! [`CoerceInst`] *constructors* turn it into type-level coercion — an illegal
//! `MirInst::Coerce` cannot be built outside `mir` at all; and the MIR
//! *verifier* re-checks at every pass boundary as defense-in-depth. [`coerce`]
//! is the thin, by-value front door the lowering walk calls for legality
//! queries that don't construct an instruction.

use pyaot_types::Repr;

pub use pyaot_mir::{CoerceInst, Coercion};

/// Classify the coercion from `have` to `need`, or `None` if illegal.
pub fn coerce(have: Repr, need: Repr) -> Option<Coercion> {
    pyaot_mir::classify_coercion(&have, &need)
}
