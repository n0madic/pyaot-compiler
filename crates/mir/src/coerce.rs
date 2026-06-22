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

/// The B18 legality rule for a CHECKED (runtime-validated) coercion: `true`
/// iff `from` is `Tagged` and `to` is one of the guard-backed shapes:
///
/// - **Raw unbox** (Phase 8H, D3): `Raw(F64)` / `Raw(I64)` / `Raw(I8)` —
///   `rt_unbox_float` / `rt_unbox_int` / `rt_unbox_bool` (`runtime/src/boxing.rs`).
/// - **Heap shape guard**: `Heap(shape)` for any `shape` whose
///   [`HeapShape::dyn_check`](pyaot_types::HeapShape::dyn_check) is `Some`
///   (builtin containers + class instances + stdlib runtime objects) —
///   `rt_check_heap_kind` / `rt_check_instance` / `rt_check_runtime_obj`. The
///   rare guard-less shapes (`BigInt`/`Iterator`) keep the unchecked reinterpret.
///
/// Single source of truth for both the validating constructor
/// ([`CoerceInst::new_checked`]) and the verifier's defense-in-depth re-check,
/// so the constructor and verifier can never disagree on what is admissible.
/// Never widen this set without adding the matching `rt_*` guard first — doing
/// so reopens the Phase 8B–8F gradual-seam SEGV family (a wrong-shape `Value`
/// blind-cast to a typed register/heap pointer in a frozen `rt_*`). See
/// PITFALLS B18.
pub(crate) fn is_legal_checked_coercion(from: &Repr, to: &Repr) -> bool {
    *from == Repr::Tagged
        && match to {
            Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I64) | Repr::Raw(RawKind::I8) => true,
            Repr::Heap(shape) => shape.dyn_check().is_some(),
            _ => false,
        }
}

impl CoerceInst {
    /// An unchecked coercion — `Some` iff the legality table
    /// ([`classify_coercion`]) accepts `(from, to)`.
    pub fn new(dst: LocalId, src: Operand, from: Repr, to: Repr) -> Option<Self> {
        classify_coercion(&from, &to)?;
        Some(Self {
            dst,
            src,
            from,
            to,
            checked: false,
        })
    }

    /// A CHECKED (runtime-validated) coercion across a gradual seam — `Some`
    /// iff [`is_legal_checked_coercion`] accepts `(from, to)` (the guard-backed
    /// gradual seams: `Tagged → Raw(F64|I64|I8)` and `Tagged → Heap(shape)` for
    /// any `dyn_check`-guarded shape). See that predicate for the full B18 rule.
    pub fn new_checked(dst: LocalId, src: Operand, from: Repr, to: Repr) -> Option<Self> {
        if !is_legal_checked_coercion(&from, &to) {
            return None;
        }
        Some(Self {
            dst,
            src,
            from,
            to,
            checked: true,
        })
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
            (Repr::Raw(RawKind::F64), Repr::Tagged),    // BoxFloat
            (Repr::Tagged, Repr::Raw(RawKind::F64)),    // UnboxFloat
            (Repr::Tagged, Repr::Raw(RawKind::I64)),    // UntagInt
            (Repr::Heap(HeapShape::Str), Repr::Tagged), // HeapToTagged
            (Repr::Tagged, Repr::Heap(HeapShape::Str)), // TaggedToHeap
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
    fn checked_admits_only_guard_backed_shapes() {
        // The three Raw unbox shapes, each backed by `rt_unbox_*`.
        assert!(
            CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Raw(RawKind::F64))
                .is_some_and(|c| c.checked())
        );
        assert!(
            CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Raw(RawKind::I64))
                .is_some_and(|c| c.checked())
        );
        // The third shape: `Tagged → Raw(I8)`, backed by `rt_unbox_bool`.
        assert!(
            CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Raw(RawKind::I8))
                .is_some_and(|c| c.checked())
        );
        // Heap-arg guard: guard-backed Heap shapes (builtin containers + class
        // instances + stdlib runtime objects) are now also admissible —
        // `rt_check_heap_kind` / `rt_check_instance` / `rt_check_runtime_obj`.
        for shape in [
            HeapShape::Str,
            HeapShape::Bytes,
            HeapShape::List(Box::new(Repr::Tagged)),
            HeapShape::Dict(Box::new(Repr::Tagged), Box::new(Repr::Tagged)),
            HeapShape::Set(Box::new(Repr::Tagged)),
            HeapShape::Tuple(vec![Repr::Tagged]),
            HeapShape::TupleVar(Box::new(Repr::Tagged)),
            HeapShape::Class(pyaot_utils::ClassId::new(3)),
            HeapShape::RuntimeObj(pyaot_core_defs::TypeTagKind::HttpResponse),
        ] {
            assert!(
                CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Heap(shape.clone()))
                    .is_some_and(|c| c.checked()),
                "checked Tagged -> Heap({shape:?}) must be admissible"
            );
        }
        // A guard-LESS Heap shape (no `dyn_check`) stays unchecked-only (B18).
        assert!(
            CoerceInst::new_checked(l(1), op(0), Repr::Tagged, Repr::Heap(HeapShape::BigInt))
                .is_none()
        );
        // Wrong source is unrepresentable.
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
