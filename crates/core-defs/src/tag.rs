//! Low-bit tag scheme for the universal [`Value`](crate::Value) representation.
//!
//! Every runtime value in pyaot fits in a single 64-bit word. The low three
//! bits carry the tag; the remaining bits carry the payload. Heap pointers are
//! always 8-byte aligned, so their low three bits are zero — that property
//! doubles as the "is pointer" discriminator (see [`PTR_TAG`]).
//!
//! ```text
//! Bit 0 = 0 → pointer (bits 1..63 = address; low three bits are zero)
//! Bit 0 = 1 → non-pointer:
//!     Bits 1..2 = 00 → Int     (low three bits = 0b001, payload = bits 3..63, 61-bit signed)
//!     Bits 1..2 = 01 → Bool    (low three bits = 0b011, payload = bit 3)
//!     Bits 1..2 = 10 → None    (low three bits = 0b101, payload = 0)
//!     Bits 1..2 = 11 → Reserved (low three bits = 0b111; kept for future tags)
//! ```
//!
//! Rationale for low-bit tagging (vs. NaN-boxing or high-bit tagging) is
//! documented in `ARCHITECTURE_REFACTOR.md` §2.1. Summary: portable across
//! x86_64 and ARM64, Int retains 61 bits (sufficient for Python-compatible
//! arithmetic given arbitrary-precision is an explicit non-goal), Float is
//! heap-boxed.

/// Bit mask covering the three tag bits.
pub const TAG_MASK: u64 = 0b111;

/// Tag value for heap pointers. Valid heap addresses have `addr & TAG_MASK == 0`
/// because the allocator guarantees 8-byte alignment, so the pointer bit pattern
/// is its own tag.
pub const PTR_TAG: u64 = 0b000;

/// Non-pointer tag for immediate 61-bit signed integers.
pub const INT_TAG: u64 = 0b001;

/// Non-pointer tag for Bool. Payload is a single bit at position [`BOOL_SHIFT`].
pub const BOOL_TAG: u64 = 0b011;

/// Non-pointer tag for the singleton None.
pub const NONE_TAG: u64 = 0b101;

/// Non-pointer tag reserved for a future immediate category (e.g. small floats,
/// characters). Unused in Phase 2; documented here so test assertions can
/// verify the tag space stays exhaustive.
pub const RESERVED_TAG: u64 = 0b111;

/// Bit-shift amount that places an Int payload above the three tag bits.
pub const INT_SHIFT: u32 = 3;

/// Bit-shift amount that places the Bool payload above the three tag bits.
pub const BOOL_SHIFT: u32 = 3;

/// Smallest integer representable as a tagged Int.
pub const INT_MIN: i64 = i64::MIN >> INT_SHIFT;

/// Largest integer representable as a tagged Int.
pub const INT_MAX: i64 = i64::MAX >> INT_SHIFT;

/// True if `i` round-trips through the 61-bit Int encoding without truncation.
#[inline]
pub const fn int_fits(i: i64) -> bool {
    i >= INT_MIN && i <= INT_MAX
}

// Compile-time invariants — any violation breaks tag extraction.
const _: () = assert!(TAG_MASK == 0b111);
const _: () = assert!(PTR_TAG & 1 == 0, "pointer tag must have bit 0 clear");
const _: () = assert!(INT_TAG & 1 == 1, "non-pointer tags must have bit 0 set");
const _: () = assert!(BOOL_TAG & 1 == 1);
const _: () = assert!(NONE_TAG & 1 == 1);
const _: () = assert!(RESERVED_TAG & 1 == 1);
// Non-pointer tags are pairwise disjoint.
const _: () = assert!(INT_TAG != BOOL_TAG);
const _: () = assert!(INT_TAG != NONE_TAG);
const _: () = assert!(INT_TAG != RESERVED_TAG);
const _: () = assert!(BOOL_TAG != NONE_TAG);
const _: () = assert!(BOOL_TAG != RESERVED_TAG);
const _: () = assert!(NONE_TAG != RESERVED_TAG);
// 61-bit Int range round-trips through the encoding.
const _: () = assert!(((INT_MAX << INT_SHIFT) >> INT_SHIFT) == INT_MAX);
const _: () = assert!(((INT_MIN << INT_SHIFT) >> INT_SHIFT) == INT_MIN);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_mask_covers_low_three_bits() {
        assert_eq!(TAG_MASK, 0b111);
    }

    #[test]
    fn non_pointer_tags_have_bit0_set() {
        for tag in [INT_TAG, BOOL_TAG, NONE_TAG, RESERVED_TAG] {
            assert_eq!(tag & 1, 1, "tag {tag:#b} should mark non-pointer");
        }
    }

    #[test]
    fn pointer_tag_has_bit0_clear() {
        assert_eq!(PTR_TAG & 1, 0);
    }

    #[test]
    fn non_pointer_tags_are_pairwise_disjoint() {
        let tags = [INT_TAG, BOOL_TAG, NONE_TAG, RESERVED_TAG];
        for (i, a) in tags.iter().enumerate() {
            for b in &tags[i + 1..] {
                assert_ne!(a, b, "duplicate tag {a:#b}");
            }
        }
    }

    #[test]
    fn int_range_round_trips() {
        for i in [INT_MIN, INT_MIN + 1, -1, 0, 1, INT_MAX - 1, INT_MAX] {
            assert!(int_fits(i));
            let encoded = (i as u64) << INT_SHIFT;
            let decoded = (encoded as i64) >> INT_SHIFT;
            assert_eq!(decoded, i);
        }
    }

    #[test]
    fn int_fits_rejects_overflow() {
        assert!(!int_fits(INT_MAX + 1));
        assert!(!int_fits(INT_MIN - 1));
        assert!(!int_fits(i64::MAX));
        assert!(!int_fits(i64::MIN));
    }
}
