//! Property-based tests for the `Type` lattice operations (`join`, `meet`).
//!
//! These tests encode the algebraic laws the lattice must satisfy.  They were
//! introduced as `#[ignore]`'d stubs in Phase 0.4 and are un-ignored here as
//! part of §S3.1 (TypeLattice trait implementation).
//!
//! Laws validated:
//!   * `forall t: join(t, t) == t`                            (idempotence)
//!   * `forall a b: join(a, b) == join(b, a)`                 (commutativity)
//!   * `forall a b c: join(a, join(b, c)) == join(join(a, b), c)` (associativity)
//!   * `forall t: join(top, t) == top`                        (top absorbs)
//!   * `forall t: join(bottom, t) == t`                       (bottom identity)
//!   * Same four laws for `meet`, with top/bottom roles swapped.
//!   * `is_subtype(a, b) && is_subtype(b, a) <=> a == b`      (antisymmetry)

use crate::{Type, TypeLattice};

fn top() -> Type {
    Type::top()
}

fn bottom() -> Type {
    Type::bottom()
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
        Type::list_of(Type::Int),
        Type::list_of(Type::Float),
        Type::tuple_of(vec![Type::Int, Type::Str]),
        Type::tuple_var_of(Type::Int),
        Type::Union(vec![Type::Int, Type::Str]),
        Type::optional(Type::Int),
    ]
}

#[test]
fn join_is_idempotent() {
    for t in sample_types() {
        assert_eq!(t.join(&t), t, "join({t:?}, {t:?}) should equal {t:?}");
    }
}

#[test]
fn join_is_commutative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            assert_eq!(
                a.join(b),
                b.join(a),
                "join({a:?}, {b:?}) ≠ join({b:?}, {a:?})"
            );
        }
    }
}

#[test]
fn join_is_associative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            for c in &ts {
                let lhs = a.join(&b.join(c));
                let rhs = a.join(b).join(c);
                assert_eq!(lhs, rhs, "associativity broken on ({a:?}, {b:?}, {c:?})");
            }
        }
    }
}

#[test]
fn join_top_absorbs() {
    let top_ty = top();
    for t in sample_types() {
        assert_eq!(top_ty.join(&t), top_ty, "join(top, {t:?}) should be top");
    }
}

#[test]
fn join_bottom_is_identity() {
    for t in sample_types() {
        assert_eq!(bottom().join(&t), t, "join(bottom, {t:?}) should be {t:?}");
    }
}

#[test]
fn meet_is_idempotent() {
    for t in sample_types() {
        assert_eq!(t.meet(&t), t, "meet({t:?}, {t:?}) should equal {t:?}");
    }
}

#[test]
fn meet_is_commutative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            assert_eq!(
                a.meet(b),
                b.meet(a),
                "meet({a:?}, {b:?}) ≠ meet({b:?}, {a:?})"
            );
        }
    }
}

#[test]
fn meet_is_associative() {
    let ts = sample_types();
    for a in &ts {
        for b in &ts {
            for c in &ts {
                let lhs = a.meet(&b.meet(c));
                let rhs = a.meet(b).meet(c);
                assert_eq!(lhs, rhs, "associativity broken on ({a:?}, {b:?}, {c:?})");
            }
        }
    }
}

#[test]
fn meet_top_is_identity() {
    let top_ty = top();
    for t in sample_types() {
        assert_eq!(top_ty.meet(&t), t, "meet(top, {t:?}) should be {t:?}");
    }
}

#[test]
fn meet_bottom_absorbs() {
    let bot = bottom();
    for t in sample_types() {
        assert_eq!(bot.meet(&t), bot, "meet(bottom, {t:?}) should be bottom");
    }
}

#[test]
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
