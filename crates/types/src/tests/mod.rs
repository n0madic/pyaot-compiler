use super::*;
use crate::TypeLattice;

mod lattice_props;

#[test]
fn test_optional_normalization() {
    let opt_int = Type::optional(Type::Int);
    assert!(matches!(opt_int, Type::Union(_)));
}

#[test]
fn test_subtyping() {
    assert!(Type::None.is_subtype_of(&Type::optional(Type::Int)));
    assert!(Type::Int.is_subtype_of(&Type::Int));
    assert!(!Type::Int.is_subtype_of(&Type::Str));
}

#[test]
fn test_union_normalization() {
    let union = Type::normalize_union(vec![Type::Int, Type::None, Type::Int]);
    match union {
        Type::Union(types) => assert_eq!(types.len(), 2),
        _ => panic!("Expected union"),
    }
}

#[test]
fn test_meet_int() {
    // Union[int, str] meet int -> int (isinstance then-branch)
    let union = Type::Int.join(&Type::Str);
    assert_eq!(union.meet(&Type::Int), Type::Int);
}

#[test]
fn test_meet_str() {
    // Union[int, str] meet str -> str
    let union = Type::Int.join(&Type::Str);
    assert_eq!(union.meet(&Type::Str), Type::Str);
}

#[test]
fn test_minus_int() {
    // Union[int, str] minus int -> str (isinstance else-branch)
    let union = Type::Int.join(&Type::Str);
    assert_eq!(union.minus(&Type::Int), Type::Str);
}

#[test]
fn test_minus_str() {
    // Union[int, str] minus str -> int
    let union = Type::Int.join(&Type::Str);
    assert_eq!(union.minus(&Type::Str), Type::Int);
}

#[test]
fn test_meet_and_minus_three_types() {
    // Union[int, str, None] meet int -> int
    let union = Type::Int.join(&Type::Str).join(&Type::None);
    assert_eq!(union.meet(&Type::Int), Type::Int);

    // Union[int, str, None] minus int -> Union[str, None]
    let remaining = union.minus(&Type::Int);
    match remaining {
        Type::Union(types) => {
            assert_eq!(types.len(), 2);
            assert!(types.contains(&Type::Str));
            assert!(types.contains(&Type::None));
        }
        _ => panic!("Expected union, got {remaining:?}"),
    }
}

#[test]
fn test_meet_and_minus_non_union() {
    // meet of a concrete type with itself returns itself
    assert_eq!(Type::Int.meet(&Type::Int), Type::Int);
    // minus of a concrete type from a different concrete type returns itself
    assert_eq!(Type::Int.minus(&Type::Str), Type::Int);
}

#[test]
fn test_meet_list_types() {
    // meet(Union[list[int], str], str) = str  — Str ≤ Union so meet = Str
    let list_int = Type::List(Box::new(Type::Int));
    let union = list_int.clone().join(&Type::Str);
    assert_eq!(union.meet(&Type::Str), Type::Str);
    // meet(Union[list[int], str], int) = Never  — Int not in the union
    assert_eq!(union.meet(&Type::Int), Type::Never);
}

#[test]
fn test_empty_union_returns_never() {
    // Empty union should normalize to Never
    let empty = Type::normalize_union(vec![]);
    assert_eq!(empty, Type::Never);
}

#[test]
fn test_never_subtyping() {
    // Never is subtype of everything
    assert!(Type::Never.is_subtype_of(&Type::Int));
    assert!(Type::Never.is_subtype_of(&Type::Str));
    assert!(Type::Never.is_subtype_of(&Type::Any));
    assert!(Type::Never.is_subtype_of(&Type::None));
    assert!(Type::Never.is_subtype_of(&Type::List(Box::new(Type::Int))));

    // Nothing is subtype of Never (except Never itself)
    assert!(!Type::Int.is_subtype_of(&Type::Never));
    assert!(!Type::Str.is_subtype_of(&Type::Never));
    assert!(!Type::Any.is_subtype_of(&Type::Never));
    assert!(!Type::None.is_subtype_of(&Type::Never));

    // Never is subtype of itself (reflexivity)
    assert!(Type::Never.is_subtype_of(&Type::Never));
}

#[test]
fn test_minus_all_returns_never() {
    // Subtracting a type from itself returns Never.
    assert_eq!(Type::Int.minus(&Type::Int), Type::Never);

    // Subtracting all members of a union returns Never.
    let union = Type::Int.join(&Type::Str);
    let remaining = union.minus(&Type::Int).minus(&Type::Str);
    assert_eq!(remaining, Type::Never);
}

#[test]
fn test_never_display() {
    assert_eq!(format!("{}", Type::Never), "Never");
}

// ---------------------------------------------------------------------------
// Area E §E.1 — numeric tower helpers
// ---------------------------------------------------------------------------

#[test]
fn test_promote_numeric_tower() {
    // All 9 numeric combinations (Bool, Int, Float × Bool, Int, Float).
    let cases = [
        (Type::Bool, Type::Bool, Type::Bool),
        (Type::Bool, Type::Int, Type::Int),
        (Type::Bool, Type::Float, Type::Float),
        (Type::Int, Type::Bool, Type::Int),
        (Type::Int, Type::Int, Type::Int),
        (Type::Int, Type::Float, Type::Float),
        (Type::Float, Type::Bool, Type::Float),
        (Type::Float, Type::Int, Type::Float),
        (Type::Float, Type::Float, Type::Float),
    ];
    for (a, b, expected) in cases {
        assert_eq!(
            Type::promote_numeric(&a, &b),
            Some(expected.clone()),
            "promote_numeric({a:?}, {b:?}) should be {expected:?}"
        );
    }
}

#[test]
fn test_promote_numeric_non_numeric_returns_none() {
    assert_eq!(Type::promote_numeric(&Type::Int, &Type::Str), None);
    assert_eq!(Type::promote_numeric(&Type::Str, &Type::Int), None);
    assert_eq!(Type::promote_numeric(&Type::Any, &Type::Int), None);
    assert_eq!(Type::promote_numeric(&Type::None, &Type::Bool), None);
}

#[test]
fn test_join_numeric_and_non_numeric() {
    // Numeric pairs — promoted via tower.
    assert_eq!(Type::Int.join(&Type::Float), Type::Float);
    assert_eq!(Type::Bool.join(&Type::Int), Type::Int);
    // Non-numeric pair — produces canonical Union.
    let u = Type::Int.join(&Type::Str);
    match u {
        Type::Union(members) => {
            assert_eq!(members.len(), 2);
            assert!(members.contains(&Type::Int));
            assert!(members.contains(&Type::Str));
        }
        other => panic!("expected Union, got {other:?}"),
    }
}

#[test]
fn test_join_idempotent_for_misc_types() {
    // join(T, T) == T for every T (idempotence law).
    for t in [
        Type::Int,
        Type::Float,
        Type::Bool,
        Type::Str,
        Type::None,
        Type::List(Box::new(Type::Int)),
        Type::Tuple(vec![Type::Int, Type::Str]),
        Type::TupleVar(Box::new(Type::Int)),
    ] {
        assert_eq!(t.join(&t), t, "idempotence broken for {t:?}");
    }
}

#[test]
fn test_join_different_tuple_lengths() {
    // Different-length tuples → canonical Union (lattice join; not TupleVar).
    let a = Type::Tuple(vec![Type::Int]);
    let b = Type::Tuple(vec![Type::Int, Type::Int]);
    let merged = a.join(&b);
    assert!(
        matches!(merged, Type::Union(_)),
        "expected Union, got {merged:?}"
    );
}

#[test]
fn test_join_numeric_promotion() {
    // Numeric pairs → numeric tower widening via join.
    assert_eq!(Type::Int.join(&Type::Float), Type::Float);
    assert_eq!(Type::Bool.join(&Type::Int), Type::Int);
    assert_eq!(Type::Bool.join(&Type::Float), Type::Float);
}

#[test]
fn test_reflected_name_comparison_pairs() {
    use crate::dunders::reflected_name;
    assert_eq!(reflected_name("__lt__"), Some("__gt__"));
    assert_eq!(reflected_name("__gt__"), Some("__lt__"));
    assert_eq!(reflected_name("__le__"), Some("__ge__"));
    assert_eq!(reflected_name("__ge__"), Some("__le__"));
    assert_eq!(reflected_name("__eq__"), Some("__eq__"));
    assert_eq!(reflected_name("__ne__"), Some("__ne__"));
    // Non-dunder sanity check.
    assert_eq!(reflected_name("__str__"), None);
}
