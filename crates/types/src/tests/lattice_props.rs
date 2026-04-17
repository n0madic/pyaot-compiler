//! Property-based tests for the `Type` lattice operations (`join`, `meet`).
//!
//! These tests encode the algebraic laws the lattice must satisfy — the
//! "expected failures" for Phase 0 of the architecture refactor (see
//! `ARCHITECTURE_REFACTOR.md` § 0.4). They are all `#[ignore]`'d because
//! `join` and `meet` do not yet exist on `Type`; Phase 3 of the refactor
//! introduces them, replaces the local stubs below with real method calls,
//! and un-ignores every test in this file.
//!
//! Laws validated:
//!   * `forall t: join(t, t) == t`                            (idempotence)
//!   * `forall a b: join(a, b) == join(b, a)`                 (commutativity)
//!   * `forall a b c: join(a, join(b, c)) == join(join(a, b), c)` (associativity)
//!   * `forall t: join(top, t) == top`                        (top absorbs)
//!   * `forall t: join(bottom, t) == t`                       (bottom identity)
//!   * Same four laws for `meet`, with top/bottom roles swapped.
//!   * `is_subtype(a, b) && is_subtype(b, a) <=> a == b`      (antisymmetry)

use crate::Type;

/// Phase-3 stub: becomes `a.join(b)` once the lattice is wired up.
fn join(_a: &Type, _b: &Type) -> Type {
    todo!("lattice join — introduced in Phase 3 of ARCHITECTURE_REFACTOR.md")
}

/// Phase-3 stub: becomes `a.meet(b)` once the lattice is wired up.
fn meet(_a: &Type, _b: &Type) -> Type {
    todo!("lattice meet — introduced in Phase 3 of ARCHITECTURE_REFACTOR.md")
}

/// Top element of the lattice: every type is a subtype of `Any`.
fn top() -> Type {
    Type::Any
}

/// Bottom element of the lattice: `Never` is a subtype of every type.
fn bottom() -> Type {
    Type::Never
}

/// A diverse enough sample of `Type` values for universal-property checks.
fn sample_types() -> Vec<Type> {
    vec![
        Type::Int,
        Type::Float,
        Type::Bool,
        Type::Str,
        Type::None,
        Type::Any,
        Type::Never,
        Type::List(Box::new(Type::Int)),
        Type::List(Box::new(Type::Float)),
        Type::Tuple(vec![Type::Int, Type::Str]),
        Type::TupleVar(Box::new(Type::Int)),
        Type::Union(vec![Type::Int, Type::Str]),
        Type::optional(Type::Int),
    ]
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn join_is_idempotent() {
    for t in sample_types() {
        assert_eq!(join(&t, &t), t, "join({t:?}, {t:?}) should equal {t:?}");
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn join_is_commutative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            assert_eq!(
                join(a, b),
                join(b, a),
                "join({a:?}, {b:?}) ≠ join({b:?}, {a:?})"
            );
        }
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn join_is_associative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            for c in &ts {
                let lhs = join(a, &join(b, c));
                let rhs = join(&join(a, b), c);
                assert_eq!(lhs, rhs, "associativity broken on ({a:?}, {b:?}, {c:?})");
            }
        }
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn join_top_absorbs() {
    let top_ty = top();
    for t in sample_types() {
        assert_eq!(join(&top_ty, &t), top_ty, "join(top, {t:?}) should be top");
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn join_bottom_is_identity() {
    for t in sample_types() {
        assert_eq!(
            join(&bottom(), &t),
            t,
            "join(bottom, {t:?}) should be {t:?}"
        );
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn meet_is_idempotent() {
    for t in sample_types() {
        assert_eq!(meet(&t, &t), t, "meet({t:?}, {t:?}) should equal {t:?}");
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn meet_is_commutative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            assert_eq!(
                meet(a, b),
                meet(b, a),
                "meet({a:?}, {b:?}) ≠ meet({b:?}, {a:?})"
            );
        }
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn meet_is_associative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            for c in &ts {
                let lhs = meet(a, &meet(b, c));
                let rhs = meet(&meet(a, b), c);
                assert_eq!(lhs, rhs, "associativity broken on ({a:?}, {b:?}, {c:?})");
            }
        }
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn meet_top_is_identity() {
    let top_ty = top();
    for t in sample_types() {
        assert_eq!(meet(&top_ty, &t), t, "meet(top, {t:?}) should be {t:?}");
    }
}

#[test]
#[ignore = "Phase 3 — lattice join/meet not implemented yet"]
fn meet_bottom_absorbs() {
    let bot = bottom();
    for t in sample_types() {
        assert_eq!(meet(&bot, &t), bot, "meet(bottom, {t:?}) should be bottom");
    }
}

#[test]
#[ignore = "Phase 3 — subtype antisymmetry is part of the lattice contract"]
fn subtype_antisymmetry() {
    // Subtype is the partial order induced by the lattice:
    //     a ≤ b  &&  b ≤ a   ⇔   a == b
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            let a_le_b = a.is_subtype_of(b);
            let b_le_a = b.is_subtype_of(a);
            assert_eq!(
                a_le_b && b_le_a,
                a == b,
                "antisymmetry broken on ({a:?}, {b:?})"
            );
        }
    }
}
