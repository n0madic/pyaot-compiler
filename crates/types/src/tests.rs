use super::*;

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
fn test_narrow_to_int() {
    // Union[int, str] narrowed to int -> int
    let union = Type::Union(vec![Type::Int, Type::Str]);
    let narrowed = union.narrow_to(&Type::Int);
    assert_eq!(narrowed, Type::Int);
}

#[test]
fn test_narrow_to_str() {
    // Union[int, str] narrowed to str -> str
    let union = Type::Union(vec![Type::Int, Type::Str]);
    let narrowed = union.narrow_to(&Type::Str);
    assert_eq!(narrowed, Type::Str);
}

#[test]
fn test_narrow_excluding_int() {
    // Union[int, str] excluding int -> str
    let union = Type::Union(vec![Type::Int, Type::Str]);
    let narrowed = union.narrow_excluding(&Type::Int);
    assert_eq!(narrowed, Type::Str);
}

#[test]
fn test_narrow_excluding_str() {
    // Union[int, str] excluding str -> int
    let union = Type::Union(vec![Type::Int, Type::Str]);
    let narrowed = union.narrow_excluding(&Type::Str);
    assert_eq!(narrowed, Type::Int);
}

#[test]
fn test_narrow_three_types() {
    // Union[int, str, None] narrowed to int -> int
    let union = Type::Union(vec![Type::Int, Type::Str, Type::None]);
    let narrowed = union.narrow_to(&Type::Int);
    assert_eq!(narrowed, Type::Int);

    // Union[int, str, None] excluding int -> Union[str, None]
    let narrowed = union.narrow_excluding(&Type::Int);
    match narrowed {
        Type::Union(types) => {
            assert_eq!(types.len(), 2);
            assert!(types.contains(&Type::Str));
            assert!(types.contains(&Type::None));
        }
        _ => panic!("Expected union"),
    }
}

#[test]
fn test_narrow_non_union() {
    // Narrowing a non-union type to itself returns itself
    let int_ty = Type::Int;
    let narrowed = int_ty.narrow_to(&Type::Int);
    assert_eq!(narrowed, Type::Int);

    // Excluding from non-union returns itself
    let narrowed = int_ty.narrow_excluding(&Type::Str);
    assert_eq!(narrowed, Type::Int);
}

#[test]
fn test_narrow_list_types() {
    // Union[list[int], str] narrowed to list -> list[int]
    let list_int = Type::List(Box::new(Type::Int));
    let union = Type::Union(vec![list_int.clone(), Type::Str]);
    let narrowed = union.narrow_to(&Type::List(Box::new(Type::Any)));
    assert_eq!(narrowed, list_int);
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
fn test_narrow_excluding_all_returns_never() {
    // Excluding the only type from a union returns Never
    let narrowed = Type::Int.narrow_excluding(&Type::Int);
    // Non-union types return themselves when narrowing
    assert_eq!(narrowed, Type::Int);

    // For union, excluding all types returns Never
    let union = Type::Union(vec![Type::Int]);
    let narrowed = union.narrow_excluding(&Type::Int);
    assert_eq!(narrowed, Type::Never);
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
fn test_unify_numeric_falls_back_to_union() {
    // Numeric — promotes.
    assert_eq!(Type::unify_numeric(&Type::Int, &Type::Float), Type::Float);
    assert_eq!(Type::unify_numeric(&Type::Bool, &Type::Int), Type::Int);
    // Non-numeric pair — falls back to Union.
    let u = Type::unify_numeric(&Type::Int, &Type::Str);
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
fn test_unify_field_type_identity() {
    // unify_field_type(T, T) == T for every T.
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
        assert_eq!(
            Type::unify_field_type(&t, &t),
            t,
            "identity law broken for {t:?}"
        );
    }
}

#[test]
fn test_unify_field_type_defers_to_tuple_shapes() {
    // Different-length tuples → TupleVar per Area D.
    let a = Type::Tuple(vec![Type::Int]);
    let b = Type::Tuple(vec![Type::Int, Type::Int]);
    let merged = Type::unify_field_type(&a, &b);
    assert!(matches!(merged, Type::TupleVar(_)));
}

#[test]
fn test_unify_field_type_numeric_promotion() {
    // Non-tuple numerics → numeric tower.
    assert_eq!(
        Type::unify_field_type(&Type::Int, &Type::Float),
        Type::Float
    );
    assert_eq!(Type::unify_field_type(&Type::Bool, &Type::Int), Type::Int);
    assert_eq!(
        Type::unify_field_type(&Type::Bool, &Type::Float),
        Type::Float
    );
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
