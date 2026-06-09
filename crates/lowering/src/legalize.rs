//! Representation legalization: the one and only coercion-insertion point.
//!
//! Given a value with representation `have` at a use-site needing `need`,
//! [`coerce`] reports the bridging op to insert (box / unbox / tag / untag /
//! int→float / heap→tagged), or `None` if the pair is not a legal coercion. No
//! other part of the compiler may emit a boxing coercion.
//!
//! The legality *table* itself lives in `pyaot-mir` ([`pyaot_mir::classify_coercion`])
//! so the MIR verifier can enforce it without `mir` depending on `lowering`;
//! [`coerce`] is the thin, by-value front door the lowering walk calls.

use pyaot_types::Repr;

pub use pyaot_mir::Coercion;

/// Classify the coercion from `have` to `need`, or `None` if illegal.
pub fn coerce(have: Repr, need: Repr) -> Option<Coercion> {
    pyaot_mir::classify_coercion(&have, &need)
}
