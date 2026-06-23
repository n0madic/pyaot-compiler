//! Dunder method classification — single source of truth for the Python
//! operator overloading protocol and data-model methods.
//!
//! Ported from the previous compiler's `types/dunders.rs` (name-level tables
//! only). Both `typeck` (operator → dunder resolution, reflected fallback)
//! and `lowering` (dunder lookup, `NotImplemented` handling) should consume
//! this module instead of hardcoding name lists.
//!
//! See [CPython Data Model §3.3.8](https://docs.python.org/3/reference/datamodel.html#emulating-numeric-types)
//! for the authoritative behaviour spec.
//!
//! # The numeric tower and the `other` parameter
//!
//! For binary numeric dunders (`__add__`, `__mul__`, …) CPython does not
//! constrain the type of the `other` parameter: the dunder inspects `other`
//! at runtime, produces a result if it knows how, or returns `NotImplemented`
//! so the interpreter can try the reflected dunder on the right operand.
//! Patterns like `2.5 * V(3.0)` (which calls `V.__rmul__(2.5)`) are valid.
//!
//! The previous compiler encoded that rule as a table-level helper
//! (`polymorphic_other_type`) that synthesized `Union[Self, int, float, bool]`
//! for an unannotated `other`. It was **deliberately not ported**: blindly
//! injecting `Self` into the `other` Union was the root cause of the microgpt
//! `loss=NaN` bug there. In this compiler, typing `other` is the solver's job
//! (`typeck`): seed it from observed call sites and dunder bodies, and let an
//! unannotated `other` widen to `Dyn` rather than to a synthetic Self-Union.

/// Classification of a Python dunder method by role and arity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DunderKind {
    /// Binary arithmetic: `__add__`, `__sub__`, `__mul__`, `__truediv__`,
    /// `__floordiv__`, `__mod__`, `__pow__`, `__matmul__` (and all reflected
    /// `__r*__` counterparts).
    BinaryNumeric,
    /// Binary bitwise: `__and__`, `__or__`, `__xor__`, `__lshift__`,
    /// `__rshift__` (and reflected).
    BinaryBitwise,
    /// Rich comparison: `__eq__`, `__ne__`, `__lt__`, `__le__`, `__gt__`,
    /// `__ge__`. These are special: CPython guarantees they never raise;
    /// `other` can be literally anything.
    Comparison,
    /// Unary arithmetic: `__neg__`, `__pos__`, `__abs__`, `__invert__`.
    /// Single `self` parameter, no `other`.
    Unary,
    /// Value conversion: `__bool__`, `__int__`, `__float__`, `__str__`,
    /// `__repr__`, `__hash__`, `__index__`, `__len__`, `__format__`.
    Conversion,
    /// Container protocol: `__getitem__`, `__setitem__`, `__delitem__`,
    /// `__contains__`, `__iter__`, `__next__`, `__call__`.
    Container,
    /// Object lifecycle: `__init__`, `__new__`, `__del__`, `__copy__`,
    /// `__deepcopy__`.
    Lifecycle,
}

/// Single source of truth for the Python dunder protocol. The `dunder_table!`
/// invocation below is the *one* place every recognized dunder is listed; the
/// macro expands it into three O(1) `match`-based lookups ([`dunder_kind`],
/// [`canonical_dunder_name`], [`reflected_name`]) plus the test-only
/// `DUNDER_ROWS` projection. The recognized name set, each name's kind, its
/// canonical `'static` spelling, and its reflected counterpart therefore can
/// never drift apart, and no surface re-lists the names (the previous design
/// kept the kind table, the reflected map, and the test name lists as three
/// parallel hand-written copies). A `reflect "<name>"` clause names a forward
/// binary / comparison dunder's reflected partner (CPython data model §3.3.8);
/// rows without it have no reflection (the `__r*__` forms, unary, conversion,
/// container, lifecycle).
macro_rules! dunder_table {
    (
        $( $name:literal => $kind:ident $(, reflect $refl:literal )? ; )*
    ) => {
        /// Map a dunder method name to its classification. Returns `None` for
        /// names that are not recognized dunders (plain methods with accidental
        /// double-underscore names like `__myhelper__` fall through here). An
        /// O(1) string `match` (compiler-lowered jump table), not a table scan.
        pub fn dunder_kind(name: &str) -> Option<DunderKind> {
            use DunderKind::*;
            match name {
                $( $name => Some($kind), )*
                _ => None,
            }
        }

        /// If `name` is a recognized dunder, returns the canonical `&'static str`
        /// spelling of that name (e.g. `"__add__"`), otherwise `None`. The
        /// `'static` lifetime makes it safe to store directly in maps keyed by
        /// `&'static str` without additional interning.
        pub fn canonical_dunder_name(name: &str) -> Option<&'static str> {
            match name {
                $( $name => Some($name), )*
                _ => None,
            }
        }

        /// Given a forward binary or comparison dunder name (e.g. `"__add__"`),
        /// return its reflected counterpart (e.g. `"__radd__"`). Comparison
        /// dunders reflect to their symmetric pair (`__lt__` ↔ `__gt__`,
        /// `__le__` ↔ `__ge__`); `__eq__` / `__ne__` are self-reflected. Returns
        /// `None` for any input that is not a forward binary / comparison dunder,
        /// including the already-reflected `__r*__` forms (no double reflection).
        pub fn reflected_name(forward: &str) -> Option<&'static str> {
            match forward {
                $( $( $name => Some($refl), )? )*
                _ => None,
            }
        }

        /// Every recognized dunder as `(name, kind, reflected)` — the test-only
        /// projection of the table, so the test suite drives off the single
        /// source instead of re-listing names.
        #[cfg(test)]
        const DUNDER_ROWS: &[(&str, DunderKind, Option<&str>)] = {
            use DunderKind::*;
            &[ $( ($name, $kind, dunder_table!(@refl $( $refl )?)), )* ]
        };
    };
    // Internal: lift an optional `reflect` clause to an `Option<&str>` literal.
    (@refl $refl:literal) => { Some($refl) };
    (@refl) => { None };
}

dunder_table! {
    // Binary numeric (forward → reflected)
    "__add__"      => BinaryNumeric, reflect "__radd__";
    "__sub__"      => BinaryNumeric, reflect "__rsub__";
    "__mul__"      => BinaryNumeric, reflect "__rmul__";
    "__truediv__"  => BinaryNumeric, reflect "__rtruediv__";
    "__floordiv__" => BinaryNumeric, reflect "__rfloordiv__";
    "__mod__"      => BinaryNumeric, reflect "__rmod__";
    "__pow__"      => BinaryNumeric, reflect "__rpow__";
    "__matmul__"   => BinaryNumeric, reflect "__rmatmul__";
    // Binary numeric (reflected — no further reflection)
    "__radd__"      => BinaryNumeric;
    "__rsub__"      => BinaryNumeric;
    "__rmul__"      => BinaryNumeric;
    "__rtruediv__"  => BinaryNumeric;
    "__rfloordiv__" => BinaryNumeric;
    "__rmod__"      => BinaryNumeric;
    "__rpow__"      => BinaryNumeric;
    "__rmatmul__"   => BinaryNumeric;
    // Binary bitwise (forward → reflected)
    "__and__"    => BinaryBitwise, reflect "__rand__";
    "__or__"     => BinaryBitwise, reflect "__ror__";
    "__xor__"    => BinaryBitwise, reflect "__rxor__";
    "__lshift__" => BinaryBitwise, reflect "__rlshift__";
    "__rshift__" => BinaryBitwise, reflect "__rrshift__";
    // Binary bitwise (reflected — no further reflection)
    "__rand__"    => BinaryBitwise;
    "__ror__"     => BinaryBitwise;
    "__rxor__"    => BinaryBitwise;
    "__rlshift__" => BinaryBitwise;
    "__rrshift__" => BinaryBitwise;
    // Rich comparison (reflect to the symmetric pair; eq/ne self-reflected)
    "__eq__" => Comparison, reflect "__eq__";
    "__ne__" => Comparison, reflect "__ne__";
    "__lt__" => Comparison, reflect "__gt__";
    "__le__" => Comparison, reflect "__ge__";
    "__gt__" => Comparison, reflect "__lt__";
    "__ge__" => Comparison, reflect "__le__";
    // Unary
    "__neg__"    => Unary;
    "__pos__"    => Unary;
    "__abs__"    => Unary;
    "__invert__" => Unary;
    // Conversion
    "__bool__"   => Conversion;
    "__int__"    => Conversion;
    "__float__"  => Conversion;
    "__str__"    => Conversion;
    "__repr__"   => Conversion;
    "__hash__"   => Conversion;
    "__index__"  => Conversion;
    "__len__"    => Conversion;
    "__format__" => Conversion;
    // Container
    "__getitem__"  => Container;
    "__setitem__"  => Container;
    "__delitem__"  => Container;
    "__contains__" => Container;
    "__iter__"     => Container;
    "__next__"     => Container;
    "__call__"     => Container;
    // Lifecycle
    "__init__"     => Lifecycle;
    "__new__"      => Lifecycle;
    "__del__"      => Lifecycle;
    "__copy__"     => Lifecycle;
    "__deepcopy__" => Lifecycle;
}

/// Returns `true` iff `name` is a recognized dunder method.
pub fn is_dunder(name: &str) -> bool {
    dunder_kind(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every lookup agrees with the table row it came from — `dunder_kind`,
    /// `canonical_dunder_name`, and `reflected_name` can never drift from the
    /// single source, since they and `DUNDER_ROWS` all expand from it.
    #[test]
    fn lookups_agree_with_the_table() {
        for &(name, kind, reflected) in DUNDER_ROWS {
            assert_eq!(dunder_kind(name), Some(kind), "kind for {name}");
            assert_eq!(canonical_dunder_name(name), Some(name), "canonical for {name}");
            assert_eq!(reflected_name(name), reflected, "reflected for {name}");
            assert!(is_dunder(name), "is_dunder for {name}");
        }
    }

    /// Reflection is an involution on forward binary dunders (each maps to a
    /// distinct `__r*__` form that does not itself reflect) and a symmetric
    /// pairing on comparisons. Only rows that declare a `reflect` partner.
    #[test]
    fn reflection_structure_holds_for_every_reflecting_row() {
        for &(name, kind, reflected) in DUNDER_ROWS {
            let Some(r) = reflected else { continue };
            // Both ends classify to the same kind.
            assert_eq!(dunder_kind(name), dunder_kind(r), "{name} vs {r}");
            match kind {
                DunderKind::BinaryNumeric | DunderKind::BinaryBitwise => {
                    assert!(r.starts_with("__r"), "{r}");
                    // No double reflection: the `__r*__` form does not reflect.
                    assert_eq!(reflected_name(r), None, "double reflection of {r}");
                }
                DunderKind::Comparison => {
                    // The symmetric pair reflects back to `name`.
                    assert_eq!(reflected_name(r), Some(name), "{name} <-> {r}");
                }
                other => panic!("unexpected reflecting kind {other:?} for {name}"),
            }
        }
    }

    /// A few spot checks, in case the macro expansion itself ever regresses.
    #[test]
    fn reflected_name_spot_checks() {
        assert_eq!(reflected_name("__add__"), Some("__radd__"));
        assert_eq!(reflected_name("__rshift__"), Some("__rrshift__"));
        assert_eq!(reflected_name("__lt__"), Some("__gt__"));
        assert_eq!(reflected_name("__gt__"), Some("__lt__"));
        assert_eq!(reflected_name("__eq__"), Some("__eq__"));
        assert_eq!(reflected_name("__ne__"), Some("__ne__"));
    }

    #[test]
    fn rejects_non_dunder_names() {
        assert_eq!(dunder_kind("foo"), None);
        assert_eq!(dunder_kind("__private"), None);
        assert_eq!(dunder_kind("_single_underscore"), None);
        assert_eq!(dunder_kind("__mycustom__"), None);
        assert_eq!(canonical_dunder_name("__mycustom__"), None);
        // Unary / conversion / unknown names have no reflection.
        assert_eq!(reflected_name("__neg__"), None);
        assert_eq!(reflected_name("__str__"), None);
        assert_eq!(reflected_name("random"), None);
        assert!(!is_dunder("regular_method"));
    }

    /// Guard against an accidental row deletion in the single-source table.
    #[test]
    fn table_covers_every_recognized_dunder() {
        assert_eq!(DUNDER_ROWS.len(), 57);
    }
}
