//! Universal tagged value type — the single 64-bit word that every pyaot
//! runtime value flows through.
//!
//! See [`crate::tag`] for the bit layout and rationale. This module builds the
//! [`Value`] API on top of those constants: constructors, predicates,
//! extractors, and a primitive-only `runtime_type` helper.
//!
//! # Primitive vs. full runtime type
//!
//! [`Value::primitive_type`] returns `Some(TypeTagKind)` for Int / Bool / None
//! and `None` for pointers. A full `runtime_type` for pointers requires
//! dereferencing the `ObjHeader` in the runtime crate — that deref cannot live
//! in `core-defs`, which forbids `unsafe`. The runtime crate exposes a
//! `type_of(Value) -> TypeTagKind` helper for the pointer case; see
//! `ARCHITECTURE_REFACTOR.md` §2.2 (amended for this split).
//!
//! # Float
//!
//! There is no `from_float` / `unwrap_float`. Floats are always boxed as
//! `*mut FloatObj` and flow through [`Value::from_ptr`]. Escape analysis in a
//! later phase may stack-allocate the box where liveness permits.

use core::fmt;

use crate::tag::{int_fits, BOOL_SHIFT, BOOL_TAG, INT_SHIFT, INT_TAG, NONE_TAG, PTR_TAG, TAG_MASK};
use crate::tag_kinds::TypeTagKind;

/// A 64-bit tagged value. Transparent over `u64` so Cranelift lowers it as a
/// plain `I64` without any wrapping overhead.
///
/// The tuple field is `pub` on purpose: codegen in later Phase 2 milestones
/// needs to emit raw-bit operations (tag arithmetic, fast-path inlining)
/// without paying for method-call indirection. Prefer the constructors and
/// accessors below — direct field access is a codegen escape hatch, not a
/// general API.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Value(pub u64);

impl Value {
    /// The singleton `None`.
    pub const NONE: Value = Value(NONE_TAG);

    /// `False` — Bool tag with payload bit cleared.
    pub const FALSE: Value = Value(BOOL_TAG);

    /// `True` — Bool tag with payload bit set.
    pub const TRUE: Value = Value((1u64 << BOOL_SHIFT) | BOOL_TAG);

    /// Tag `i` as an immediate Int. Panics in debug if `i` exceeds the
    /// 61-bit representable range (`INT_MIN..=INT_MAX`); silently truncates
    /// in release, matching the documented non-goal of arbitrary-precision
    /// integers.
    #[inline]
    pub const fn from_int(i: i64) -> Value {
        debug_assert!(int_fits(i), "int overflow for tagged Value");
        Value(((i as u64) << INT_SHIFT) | INT_TAG)
    }

    /// Tag `b` as an immediate Bool. Returns [`Self::TRUE`] or [`Self::FALSE`].
    #[inline]
    pub const fn from_bool(b: bool) -> Value {
        if b {
            Self::TRUE
        } else {
            Self::FALSE
        }
    }

    /// Wrap a heap pointer. The caller is responsible for the pointer being
    /// either null or 8-byte aligned — if not, [`Self::is_ptr`] may
    /// misclassify the value.
    #[inline]
    pub fn from_ptr<T>(p: *mut T) -> Value {
        Value(p as u64)
    }

    #[inline]
    pub const fn is_int(self) -> bool {
        self.0 & TAG_MASK == INT_TAG
    }

    #[inline]
    pub const fn is_bool(self) -> bool {
        self.0 & TAG_MASK == BOOL_TAG
    }

    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == NONE_TAG
    }

    /// True if the low bit is clear. This includes the null pointer
    /// (`Value(0)`); callers that need to distinguish null explicitly should
    /// call `unwrap_ptr::<T>().is_null()`.
    #[inline]
    pub const fn is_ptr(self) -> bool {
        self.0 & 1 == PTR_TAG
    }

    /// Extract the Int payload via arithmetic right shift (sign-extending).
    /// Panics in debug if the tag is not Int.
    #[inline]
    pub const fn unwrap_int(self) -> i64 {
        debug_assert!(self.is_int());
        (self.0 as i64) >> INT_SHIFT
    }

    /// Extract the Bool payload. Panics in debug if the tag is not Bool.
    #[inline]
    pub const fn unwrap_bool(self) -> bool {
        debug_assert!(self.is_bool());
        (self.0 >> BOOL_SHIFT) != 0
    }

    /// Extract the raw pointer. Panics in debug if the tag is not a pointer.
    /// The resulting pointer may be null.
    #[inline]
    pub fn unwrap_ptr<T>(self) -> *mut T {
        debug_assert!(self.is_ptr());
        self.0 as *mut T
    }

    /// Runtime type for immediate (non-pointer) tags. Returns `None` for
    /// pointers — the concrete `TypeTagKind` lives in the object header and
    /// must be read by the runtime crate.
    #[inline]
    pub const fn primitive_type(self) -> Option<TypeTagKind> {
        match self.0 & TAG_MASK {
            INT_TAG => Some(TypeTagKind::Int),
            BOOL_TAG => Some(TypeTagKind::Bool),
            NONE_TAG => Some(TypeTagKind::None),
            _ => None,
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            write!(f, "Value::None")
        } else if self.is_bool() {
            write!(f, "Value::Bool({})", self.unwrap_bool())
        } else if self.is_int() {
            write!(f, "Value::Int({})", self.unwrap_int())
        } else {
            write!(f, "Value::Ptr({:#018x})", self.0)
        }
    }
}

// Cranelift lowers Value as I64. Preserve that invariant.
const _: () = assert!(core::mem::size_of::<Value>() == 8);
const _: () = assert!(core::mem::align_of::<Value>() == 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag::{INT_MAX, INT_MIN};

    fn sample_ints() -> [i64; 9] {
        [
            INT_MIN,
            INT_MIN + 1,
            -(1 << 40),
            -1,
            0,
            1,
            1 << 40,
            INT_MAX - 1,
            INT_MAX,
        ]
    }

    fn sample_pointers() -> [*mut u8; 5] {
        [
            core::ptr::null_mut(),
            0x8usize as *mut u8,
            0x10usize as *mut u8,
            0xDEAD_BEEF_0000_0000u64 as usize as *mut u8,
            0xFFFF_FFFF_FFFF_FFF8u64 as usize as *mut u8,
        ]
    }

    #[test]
    fn size_and_align() {
        assert_eq!(core::mem::size_of::<Value>(), 8);
        assert_eq!(core::mem::align_of::<Value>(), 8);
    }

    #[test]
    fn int_round_trip() {
        for i in sample_ints() {
            let v = Value::from_int(i);
            assert!(v.is_int(), "sample {i} should tag as Int");
            assert!(!v.is_bool());
            assert!(!v.is_none());
            assert!(!v.is_ptr());
            assert_eq!(v.unwrap_int(), i);
        }
    }

    #[test]
    fn bool_round_trip() {
        assert_eq!(Value::from_bool(true), Value::TRUE);
        assert_eq!(Value::from_bool(false), Value::FALSE);
        assert!(Value::TRUE.is_bool());
        assert!(Value::FALSE.is_bool());
        assert!(Value::TRUE.unwrap_bool());
        assert!(!Value::FALSE.unwrap_bool());
    }

    #[test]
    fn none_is_distinct() {
        assert_ne!(Value::NONE, Value::FALSE);
        assert_ne!(Value::NONE, Value::TRUE);
        assert_ne!(Value::NONE, Value::from_int(0));
        assert_ne!(Value::NONE, Value::from_ptr::<u8>(core::ptr::null_mut()));
        assert!(Value::NONE.is_none());
        assert!(!Value::NONE.is_int());
        assert!(!Value::NONE.is_bool());
        assert!(!Value::NONE.is_ptr());
    }

    #[test]
    fn ptr_round_trip() {
        for p in sample_pointers() {
            let v = Value::from_ptr(p);
            assert!(v.is_ptr(), "pointer {p:?} should tag as Ptr");
            assert!(!v.is_int());
            assert!(!v.is_bool());
            // A null pointer is a valid pointer; `is_none` remains false.
            assert!(!v.is_none());
            assert_eq!(v.unwrap_ptr::<u8>(), p);
        }
    }

    #[test]
    fn predicate_exhaustiveness() {
        let primitives = [
            Value::NONE,
            Value::TRUE,
            Value::FALSE,
            Value::from_int(0),
            Value::from_int(-1),
            Value::from_int(42),
        ];
        for v in primitives {
            let count = [v.is_int(), v.is_bool(), v.is_none(), v.is_ptr()]
                .iter()
                .filter(|b| **b)
                .count();
            assert_eq!(count, 1, "value {v:?} matched {count} predicates");
        }
        for p in sample_pointers() {
            let v = Value::from_ptr(p);
            let count = [v.is_int(), v.is_bool(), v.is_none(), v.is_ptr()]
                .iter()
                .filter(|b| **b)
                .count();
            assert_eq!(count, 1, "pointer value {v:?} matched {count} predicates");
        }
    }

    #[test]
    fn primitive_type_classification() {
        assert_eq!(Value::from_int(0).primitive_type(), Some(TypeTagKind::Int));
        assert_eq!(
            Value::from_int(INT_MAX).primitive_type(),
            Some(TypeTagKind::Int),
        );
        assert_eq!(Value::TRUE.primitive_type(), Some(TypeTagKind::Bool));
        assert_eq!(Value::FALSE.primitive_type(), Some(TypeTagKind::Bool));
        assert_eq!(Value::NONE.primitive_type(), Some(TypeTagKind::None));
        for p in sample_pointers() {
            assert_eq!(
                Value::from_ptr(p).primitive_type(),
                None,
                "pointer primitive_type must be None"
            );
        }
    }

    #[test]
    fn debug_format_shapes() {
        assert_eq!(format!("{:?}", Value::NONE), "Value::None");
        assert_eq!(format!("{:?}", Value::TRUE), "Value::Bool(true)");
        assert_eq!(format!("{:?}", Value::FALSE), "Value::Bool(false)");
        assert_eq!(format!("{:?}", Value::from_int(42)), "Value::Int(42)");
        assert_eq!(format!("{:?}", Value::from_int(-1)), "Value::Int(-1)");
        let ptr = 0x8usize as *mut u8;
        assert_eq!(
            format!("{:?}", Value::from_ptr(ptr)),
            "Value::Ptr(0x0000000000000008)",
        );
    }

    #[test]
    fn bit_layout_sanity() {
        assert_eq!(Value::NONE.0, NONE_TAG);
        assert_eq!(Value::FALSE.0 & TAG_MASK, BOOL_TAG);
        assert_eq!(Value::TRUE.0 & TAG_MASK, BOOL_TAG);
        assert_eq!(Value::from_int(0).0 & TAG_MASK, INT_TAG);
        assert_eq!(Value::from_int(-1).0 as i64 >> INT_SHIFT, -1);
    }

    // Overflow detection only fires under debug_assertions. Release builds
    // silently truncate (documented behavior); the corresponding test cannot
    // run under `cargo test --release` because `panic = abort` in the release
    // profile turns `#[should_panic]` into an abort instead of a catch.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "int overflow for tagged Value")]
    fn int_overflow_panics_in_debug() {
        let _ = Value::from_int(i64::MAX);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn unwrap_int_on_bool_panics_in_debug() {
        let _ = Value::TRUE.unwrap_int();
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn unwrap_bool_on_int_panics_in_debug() {
        let _ = Value::from_int(1).unwrap_bool();
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn unwrap_ptr_on_int_panics_in_debug() {
        let _: *mut u8 = Value::from_int(1).unwrap_ptr();
    }
}
