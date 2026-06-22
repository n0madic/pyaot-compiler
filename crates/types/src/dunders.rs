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

/// Single source of truth: every recognized dunder name paired with its
/// classification. Both [`dunder_kind`] and [`canonical_dunder_name`] derive
/// from this one table, so the name set and its `'static` spellings can never
/// drift apart (the previous design re-listed all names in two parallel
/// matches, guarded only by a test).
const DUNDER_TABLE: &[(&str, DunderKind)] = {
    use DunderKind::*;
    &[
        // Binary numeric (forward)
        ("__add__", BinaryNumeric),
        ("__sub__", BinaryNumeric),
        ("__mul__", BinaryNumeric),
        ("__truediv__", BinaryNumeric),
        ("__floordiv__", BinaryNumeric),
        ("__mod__", BinaryNumeric),
        ("__pow__", BinaryNumeric),
        ("__matmul__", BinaryNumeric),
        // Binary numeric (reflected)
        ("__radd__", BinaryNumeric),
        ("__rsub__", BinaryNumeric),
        ("__rmul__", BinaryNumeric),
        ("__rtruediv__", BinaryNumeric),
        ("__rfloordiv__", BinaryNumeric),
        ("__rmod__", BinaryNumeric),
        ("__rpow__", BinaryNumeric),
        ("__rmatmul__", BinaryNumeric),
        // Binary bitwise (forward)
        ("__and__", BinaryBitwise),
        ("__or__", BinaryBitwise),
        ("__xor__", BinaryBitwise),
        ("__lshift__", BinaryBitwise),
        ("__rshift__", BinaryBitwise),
        // Binary bitwise (reflected)
        ("__rand__", BinaryBitwise),
        ("__ror__", BinaryBitwise),
        ("__rxor__", BinaryBitwise),
        ("__rlshift__", BinaryBitwise),
        ("__rrshift__", BinaryBitwise),
        // Rich comparison
        ("__eq__", Comparison),
        ("__ne__", Comparison),
        ("__lt__", Comparison),
        ("__le__", Comparison),
        ("__gt__", Comparison),
        ("__ge__", Comparison),
        // Unary
        ("__neg__", Unary),
        ("__pos__", Unary),
        ("__abs__", Unary),
        ("__invert__", Unary),
        // Conversion
        ("__bool__", Conversion),
        ("__int__", Conversion),
        ("__float__", Conversion),
        ("__str__", Conversion),
        ("__repr__", Conversion),
        ("__hash__", Conversion),
        ("__index__", Conversion),
        ("__len__", Conversion),
        ("__format__", Conversion),
        // Container
        ("__getitem__", Container),
        ("__setitem__", Container),
        ("__delitem__", Container),
        ("__contains__", Container),
        ("__iter__", Container),
        ("__next__", Container),
        ("__call__", Container),
        // Lifecycle
        ("__init__", Lifecycle),
        ("__new__", Lifecycle),
        ("__del__", Lifecycle),
        ("__copy__", Lifecycle),
        ("__deepcopy__", Lifecycle),
    ]
};

/// Map a dunder method name to its classification. Returns `None` for names
/// that are not recognized dunders (plain methods with accidental double
/// underscore names like `__myhelper__` fall through here).
pub fn dunder_kind(name: &str) -> Option<DunderKind> {
    DUNDER_TABLE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, kind)| *kind)
}

/// Returns `true` iff `name` is a recognized dunder method.
pub fn is_dunder(name: &str) -> bool {
    dunder_kind(name).is_some()
}

/// If `name` is a recognized dunder method, returns the canonical `&'static str`
/// spelling of that name (e.g. `"__add__"`), otherwise returns `None`.
///
/// The returned reference has `'static` lifetime, which makes it safe to store
/// directly in maps keyed by `&'static str` without additional interning.
pub fn canonical_dunder_name(name: &str) -> Option<&'static str> {
    // Return the table's `'static` spelling of `name` (or `None`); shares
    // `DUNDER_TABLE` with `dunder_kind`, so the two can never disagree.
    DUNDER_TABLE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, _)| *n)
}

/// Given a forward binary dunder name (e.g. `"__add__"`), return the name of
/// its reflected counterpart (e.g. `"__radd__"`). Returns `None` for any
/// input that is not a forward binary dunder.
///
/// Comparison dunders reflect to their symmetric pair per the data model
/// (`__lt__` ↔ `__gt__`, `__le__` ↔ `__ge__`); `__eq__` / `__ne__` are
/// self-reflected.
pub fn reflected_name(forward: &str) -> Option<&'static str> {
    Some(match forward {
        // Binary numeric
        "__add__" => "__radd__",
        "__sub__" => "__rsub__",
        "__mul__" => "__rmul__",
        "__truediv__" => "__rtruediv__",
        "__floordiv__" => "__rfloordiv__",
        "__mod__" => "__rmod__",
        "__pow__" => "__rpow__",
        "__matmul__" => "__rmatmul__",
        // Binary bitwise
        "__and__" => "__rand__",
        "__or__" => "__ror__",
        "__xor__" => "__rxor__",
        "__lshift__" => "__rlshift__",
        "__rshift__" => "__rrshift__",
        // Comparison (per Python data model — symmetric pairs)
        "__lt__" => "__gt__",
        "__gt__" => "__lt__",
        "__le__" => "__ge__",
        "__ge__" => "__le__",
        "__eq__" => "__eq__",
        "__ne__" => "__ne__",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_every_binary_numeric_forward_and_reflected() {
        for name in [
            "__add__",
            "__sub__",
            "__mul__",
            "__truediv__",
            "__floordiv__",
            "__mod__",
            "__pow__",
            "__matmul__",
            "__radd__",
            "__rsub__",
            "__rmul__",
            "__rtruediv__",
            "__rfloordiv__",
            "__rmod__",
            "__rpow__",
            "__rmatmul__",
        ] {
            assert_eq!(
                dunder_kind(name),
                Some(DunderKind::BinaryNumeric),
                "{}",
                name
            );
        }
    }

    #[test]
    fn recognizes_every_binary_bitwise() {
        for name in [
            "__and__",
            "__or__",
            "__xor__",
            "__lshift__",
            "__rshift__",
            "__rand__",
            "__ror__",
            "__rxor__",
            "__rlshift__",
            "__rrshift__",
        ] {
            assert_eq!(
                dunder_kind(name),
                Some(DunderKind::BinaryBitwise),
                "{}",
                name
            );
        }
    }

    #[test]
    fn recognizes_every_comparison() {
        for name in ["__eq__", "__ne__", "__lt__", "__le__", "__gt__", "__ge__"] {
            assert_eq!(dunder_kind(name), Some(DunderKind::Comparison), "{}", name);
        }
    }

    #[test]
    fn recognizes_every_unary() {
        for name in ["__neg__", "__pos__", "__abs__", "__invert__"] {
            assert_eq!(dunder_kind(name), Some(DunderKind::Unary), "{}", name);
        }
    }

    #[test]
    fn recognizes_every_conversion() {
        for name in [
            "__bool__",
            "__int__",
            "__float__",
            "__str__",
            "__repr__",
            "__hash__",
            "__index__",
            "__len__",
            "__format__",
        ] {
            assert_eq!(dunder_kind(name), Some(DunderKind::Conversion), "{}", name);
        }
    }

    #[test]
    fn recognizes_every_container() {
        for name in [
            "__getitem__",
            "__setitem__",
            "__delitem__",
            "__contains__",
            "__iter__",
            "__next__",
            "__call__",
        ] {
            assert_eq!(dunder_kind(name), Some(DunderKind::Container), "{}", name);
        }
    }

    #[test]
    fn recognizes_every_lifecycle() {
        for name in ["__init__", "__new__", "__del__", "__copy__", "__deepcopy__"] {
            assert_eq!(dunder_kind(name), Some(DunderKind::Lifecycle), "{}", name);
        }
    }

    #[test]
    fn rejects_non_dunder_names() {
        assert_eq!(dunder_kind("foo"), None);
        assert_eq!(dunder_kind("__private"), None);
        assert_eq!(dunder_kind("_single_underscore"), None);
        assert_eq!(dunder_kind("__mycustom__"), None);
        assert!(!is_dunder("regular_method"));
    }

    #[test]
    fn canonical_name_is_total_over_the_kind_table() {
        // Every name dunder_kind recognizes must have a canonical spelling,
        // and vice versa — the two tables may never drift apart.
        for name in [
            "__add__",
            "__rmatmul__",
            "__rrshift__",
            "__ge__",
            "__invert__",
            "__format__",
            "__call__",
            "__deepcopy__",
        ] {
            assert_eq!(canonical_dunder_name(name), Some(name));
            assert!(dunder_kind(name).is_some());
        }
        assert_eq!(canonical_dunder_name("__mycustom__"), None);
    }

    #[test]
    fn reflected_name_forms_a_bijection_on_binary_dunders() {
        // Forward → reflected roundtrip defined for all binary dunders.
        let forwards = [
            "__add__",
            "__sub__",
            "__mul__",
            "__truediv__",
            "__floordiv__",
            "__mod__",
            "__pow__",
            "__matmul__",
            "__and__",
            "__or__",
            "__xor__",
            "__lshift__",
            "__rshift__",
        ];
        for f in forwards {
            let r = reflected_name(f).expect(f);
            // Reflected name is the forward with `r` prefix in the first underscore slot.
            assert!(r.starts_with("__r"));
            // Reflected names are NOT valid inputs to reflected_name (no double reflection).
            assert_eq!(reflected_name(r), None);
            // Both forward and reflected classify to the same kind.
            assert_eq!(dunder_kind(f), dunder_kind(r));
        }
    }

    #[test]
    fn reflected_name_comparison_pairs() {
        assert_eq!(reflected_name("__lt__"), Some("__gt__"));
        assert_eq!(reflected_name("__gt__"), Some("__lt__"));
        assert_eq!(reflected_name("__le__"), Some("__ge__"));
        assert_eq!(reflected_name("__ge__"), Some("__le__"));
        // __eq__ / __ne__ are self-reflected.
        assert_eq!(reflected_name("__eq__"), Some("__eq__"));
        assert_eq!(reflected_name("__ne__"), Some("__ne__"));
    }

    #[test]
    fn reflected_name_returns_none_for_unary_and_unknown() {
        assert_eq!(reflected_name("__neg__"), None);
        assert_eq!(reflected_name("__str__"), None);
        assert_eq!(reflected_name("random"), None);
    }
}
