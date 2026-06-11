//! The validated [`Coerce`](crate::MirInst::Coerce) payload.
//!
//! Enum-variant fields in Rust are always as public as the enum, so a bare
//! `MirInst::Coerce { .. }` variant could be constructed with an illegal
//! `(from, to)` pair from anywhere. [`CoerceInst`] closes that hole by the
//! type system: its fields are `pub(crate)` and the only public constructors
//! are [`CoerceInst::new`] / [`CoerceInst::new_checked`], which admit exactly
//! the pairs the legality table ([`classify_coercion`]) / the checked-unbox
//! rule (Phase 8H, D3) accept. Outside this crate an illegal coercion is
//! *unrepresentable*; inside, `verify` keeps re-checking as defense-in-depth
//! (and its negative tests build illegal payloads directly).

use pyaot_types::{RawKind, Repr};
use pyaot_utils::LocalId;

use crate::{classify_coercion, Operand};

/// A representation bridge `src: from → dst: to`, constructible only through
/// the validating constructors. See [`crate::MirInst::Coerce`] for semantics.
#[derive(Debug, Clone)]
pub struct CoerceInst {
    pub(crate) dst: LocalId,
    pub(crate) src: Operand,
    pub(crate) from: Repr,
    pub(crate) to: Repr,
    pub(crate) checked: bool,
}

impl CoerceInst {
    /// An unchecked coercion — `Some` iff the legality table
    /// ([`classify_coercion`]) accepts `(from, to)`.
    pub fn new(dst: LocalId, src: Operand, from: Repr, to: Repr) -> Option<Self> {
        classify_coercion(&from, &to)?;
        Some(Self { dst, src, from, to, checked: false })
    }

    /// A CHECKED (runtime-validated) unbox at a stdlib raw-ABI boundary
    /// (Phase 8H, D3) — `Some` iff `from` is `Tagged` and `to` is `Raw(F64)`
    /// or `Raw(I64)` (the `rt_unbox_float` / `rt_unbox_int` shapes).
    ///
    /// These two shapes (`Tagged→Raw(F64)`, `Tagged→Raw(I64)`) are the only
    /// checked admissions because each has a matching runtime guard that raises
    /// `TypeError` instead of SEGV (`rt_unbox_float` / `rt_unbox_int`,
    /// `runtime/src/boxing.rs`). Never widen this set without adding the matching
    /// `rt_*` guard first — doing so reopens the Phase 8B–8F gradual-seam SEGV
    /// family (a wrong-shape `Value` blind-cast to a typed heap pointer in a
    /// frozen `rt_*`). See PITFALLS B18.
    pub fn new_checked(dst: LocalId, src: Operand, from: Repr, to: Repr) -> Option<Self> {
        let legal = from == Repr::Tagged
            && matches!(to, Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I64));
        if !legal {
            return None;
        }
        Some(Self { dst, src, from, to, checked: true })
    }

    pub fn dst(&self) -> LocalId {
        self.dst
    }

    pub fn src(&self) -> &Operand {
        &self.src
    }

    pub fn from(&self) -> &Repr {
        &self.from
    }

    pub fn to(&self) -> &Repr {
        &self.to
    }

    pub fn checked(&self) -> bool {
        self.checked
    }
}

#[cfg(test)]
mod tests {
    use pyaot_types::HeapShape;

    use super::*;

    fn l(i: u32) -> LocalId {
        LocalId::new(i)
    }

    fn op(i: u32) -> Operand {
        Operand::Local(l(i))
    }

    #[test]
    fn legal_pairs_construct() {
        for (from, to) in [
            (Repr::Raw(RawKind::F64), Repr::Tagged),              // BoxFloat
            (Repr::Tagged, Repr::Raw(RawKind::F64)),              // UnboxFloat
            (Repr::Tagged, Repr::Raw(RawKind::I64)),              // UntagInt
            (Repr::Heap(HeapShape::Str), Repr::Tagged),           // HeapToTagged
            (Repr::Tagged, Repr::Heap(HeapShape::Str)),           // TaggedToHeap
        ] {
            let c = CoerceInst::new(l(1), op(0), from.clone(), to.clone())
                .unwrap_or_else(|| panic!("{from:?} -> {to:?} must be legal"));
            assert!(!c.checked());
            assert_eq!(c.from(), &from);
            assert_eq!(c.to(), &to);
        }
    }

    #[test]
    fn illegal_pairs_rejected() {
        for (from, to) in [
            (Repr::Raw(RawKind::F64), Repr::Raw(RawKind::I64)),
            (Repr::Raw(RawKind::I8), Repr::Raw(RawKind::I64)),
            (
                Repr::Heap(HeapShape::Str),
                Repr::Heap(HeapShape::List(Box::new(Repr::Tagged))),
            ),
        ] {
            assert!(
                CoerceInst::new(l(1), op(0), from.clone(), to.clone()).is_none(),
                "{from:?} -> {to:?} must be rejected"
            );
        }
    }

    #[test]
    fn checked_admits_only_the_two_unbox_shapes() {
        assert!(CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Raw(RawKind::F64))
            .is_some_and(|c| c.checked()));
        assert!(CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Raw(RawKind::I64))
            .is_some_and(|c| c.checked()));
        // Wrong target / wrong source are unrepresentable.
        assert!(CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Raw(RawKind::I8))
            .is_none());
        assert!(CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Tagged).is_none());
        assert!(CoerceInst::new_checked(
            l(1),
            op(0),
            Repr::Raw(RawKind::F64),
            Repr::Raw(RawKind::I64)
        )
        .is_none());
    }
}
