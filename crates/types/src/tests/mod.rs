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
    let list_int = Type::list_of(Type::Int);
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
    assert!(Type::Never.is_subtype_of(&Type::list_of(Type::Int)));

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
        Type::list_of(Type::Int),
        Type::tuple_of(vec![Type::Int, Type::Str]),
        Type::tuple_var_of(Type::Int),
    ] {
        assert_eq!(t.join(&t), t, "idempotence broken for {t:?}");
    }
}

#[test]
fn test_join_different_tuple_lengths() {
    // Different-length tuples → canonical Union (lattice join; not TupleVar).
    let a = Type::tuple_of(vec![Type::Int]);
    let b = Type::tuple_of(vec![Type::Int, Type::Int]);
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

// ---------------------------------------------------------------------------
// S3.2 §S3.2a — Type::Generic accessor API + builtin ClassId constants
// ---------------------------------------------------------------------------

#[test]
fn test_generic_builtin_class_ids_are_unique() {
    use crate::{
        BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID,
        BUILTIN_TUPLE_VAR_CLASS_ID, FIRST_USER_CLASS_ID,
    };
    let ids = [
        BUILTIN_LIST_CLASS_ID.0,
        BUILTIN_DICT_CLASS_ID.0,
        BUILTIN_SET_CLASS_ID.0,
        BUILTIN_TUPLE_CLASS_ID.0,
        BUILTIN_TUPLE_VAR_CLASS_ID.0,
    ];
    // All five IDs must be distinct.
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        assert!(seen.insert(id), "duplicate builtin class id: {id}");
    }
    // All must be strictly below FIRST_USER_CLASS_ID.
    for id in ids {
        assert!(
            id < FIRST_USER_CLASS_ID as u32,
            "builtin class id {id} >= FIRST_USER_CLASS_ID ({})",
            FIRST_USER_CLASS_ID
        );
    }
}

#[test]
fn test_accessor_roundtrip_legacy_variants() {
    // Accessors work on the legacy variants (coexist until S3.2c).
    let list_int = Type::list_of(Type::Int);
    assert_eq!(list_int.list_elem(), Some(&Type::Int));
    assert!(list_int.is_list_like());

    let dict_sv = Type::dict_of(Type::Str, Type::Int);
    assert_eq!(dict_sv.dict_kv(), Some((&Type::Str, &Type::Int)));
    assert!(dict_sv.is_dict_like());

    let set_f = Type::set_of(Type::Float);
    assert_eq!(set_f.set_elem(), Some(&Type::Float));

    let tup = Type::tuple_of(vec![Type::Int, Type::Str]);
    assert_eq!(tup.tuple_elems(), Some([Type::Int, Type::Str].as_slice()));
    assert_eq!(tup.tuple_var_elem(), None);

    let tupvar = Type::tuple_var_of(Type::Int);
    assert_eq!(tupvar.tuple_var_elem(), Some(&Type::Int));
    assert_eq!(tupvar.tuple_elems(), None);

    let iter = Type::Iterator(Box::new(Type::Str));
    assert_eq!(iter.iter_elem(), Some(&Type::Str));

    // Non-list types return None from list_elem.
    assert_eq!(Type::Int.list_elem(), None);
    assert_eq!(Type::Str.set_elem(), None);
}

#[test]
fn test_accessor_roundtrip_generic_variant() {
    use crate::{
        BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID,
        BUILTIN_TUPLE_VAR_CLASS_ID,
    };

    let g_list = Type::Generic {
        base: BUILTIN_LIST_CLASS_ID,
        args: vec![Type::Int],
    };
    assert_eq!(g_list.list_elem(), Some(&Type::Int));
    assert!(g_list.is_list_like());

    let g_dict = Type::Generic {
        base: BUILTIN_DICT_CLASS_ID,
        args: vec![Type::Str, Type::Float],
    };
    assert_eq!(g_dict.dict_kv(), Some((&Type::Str, &Type::Float)));
    assert!(g_dict.is_dict_like());

    let g_set = Type::Generic {
        base: BUILTIN_SET_CLASS_ID,
        args: vec![Type::Bool],
    };
    assert_eq!(g_set.set_elem(), Some(&Type::Bool));

    let g_tup = Type::Generic {
        base: BUILTIN_TUPLE_CLASS_ID,
        args: vec![Type::Int, Type::Str],
    };
    assert_eq!(g_tup.tuple_elems(), Some([Type::Int, Type::Str].as_slice()));
    assert_eq!(g_tup.tuple_var_elem(), None);

    let g_tupvar = Type::Generic {
        base: BUILTIN_TUPLE_VAR_CLASS_ID,
        args: vec![Type::Float],
    };
    assert_eq!(g_tupvar.tuple_var_elem(), Some(&Type::Float));
    assert_eq!(g_tupvar.tuple_elems(), None);
}

#[test]
fn test_generic_subtyping_covariant() {
    use crate::TypeLattice;
    use crate::{BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID};

    // List[Int] ≤ List[Any]
    let g_list_int = Type::Generic {
        base: BUILTIN_LIST_CLASS_ID,
        args: vec![Type::Int],
    };
    let g_list_any = Type::Generic {
        base: BUILTIN_LIST_CLASS_ID,
        args: vec![Type::Any],
    };
    assert!(g_list_int.is_subtype_of(&g_list_any));
    // List[Any] ≤ List[Int] (Any wildcard is bidirectional in is_subtype_of)
    assert!(g_list_any.is_subtype_of(&g_list_int));

    // Different base: not subtypes.
    let g_dict = Type::Generic {
        base: BUILTIN_DICT_CLASS_ID,
        args: vec![Type::Str, Type::Int],
    };
    assert!(!g_list_int.is_subtype_of(&g_dict));
    assert!(!g_dict.is_subtype_of(&g_list_int));
}

#[test]
fn test_generic_join_same_base() {
    use crate::TypeLattice;
    use crate::BUILTIN_LIST_CLASS_ID;

    let g_int = Type::Generic {
        base: BUILTIN_LIST_CLASS_ID,
        args: vec![Type::Int],
    };
    let g_float = Type::Generic {
        base: BUILTIN_LIST_CLASS_ID,
        args: vec![Type::Float],
    };
    // join(List[Int], List[Float]) = List[Float] via numeric tower covariant join.
    let joined = g_int.join(&g_float);
    assert_eq!(
        joined,
        Type::Generic {
            base: BUILTIN_LIST_CLASS_ID,
            args: vec![Type::Float],
        }
    );
    // Idempotent.
    assert_eq!(g_int.join(&g_int), g_int);
}

#[test]
fn test_generic_display() {
    use crate::{
        BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID,
        BUILTIN_TUPLE_VAR_CLASS_ID,
    };
    assert_eq!(
        format!(
            "{}",
            Type::Generic {
                base: BUILTIN_LIST_CLASS_ID,
                args: vec![Type::Int]
            }
        ),
        "list[int]"
    );
    assert_eq!(
        format!(
            "{}",
            Type::Generic {
                base: BUILTIN_DICT_CLASS_ID,
                args: vec![Type::Str, Type::Float]
            }
        ),
        "dict[str, float]"
    );
    assert_eq!(
        format!(
            "{}",
            Type::Generic {
                base: BUILTIN_SET_CLASS_ID,
                args: vec![Type::Bool]
            }
        ),
        "set[bool]"
    );
    assert_eq!(
        format!(
            "{}",
            Type::Generic {
                base: BUILTIN_TUPLE_CLASS_ID,
                args: vec![Type::Int, Type::Str]
            }
        ),
        "tuple[int, str]"
    );
    assert_eq!(
        format!(
            "{}",
            Type::Generic {
                base: BUILTIN_TUPLE_VAR_CLASS_ID,
                args: vec![Type::Float]
            }
        ),
        "tuple[float, ...]"
    );
}

#[test]
fn test_generic_is_heap() {
    use crate::BUILTIN_LIST_CLASS_ID;
    assert!(Type::Generic {
        base: BUILTIN_LIST_CLASS_ID,
        args: vec![Type::Int]
    }
    .is_heap());
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
