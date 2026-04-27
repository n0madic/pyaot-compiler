//! Type system for the Python AOT compiler

#![forbid(unsafe_code)]

pub mod dunders;
pub mod exceptions;
pub mod tag_kinds;

pub use exceptions::{
    exception_name_to_tag, exception_tag_to_name, is_builtin_exception_name, BuiltinException,
    BuiltinExceptionKind, BUILTIN_EXCEPTIONS, BUILTIN_EXCEPTION_COUNT, FIRST_USER_CLASS_ID,
    RESERVED_STDLIB_EXCEPTION_SLOTS,
};

pub use tag_kinds::{is_type_tag_name, type_tag_to_name, TypeTagKind, TYPE_TAG_COUNT};

use pyaot_utils::{ClassId, InternedString};

/// Type identifier
pub type TypeId = u32;

/// The type representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// Primitive types
    Int,
    Float,
    Bool,
    Str,
    Bytes,
    None,

    /// Generic list
    List(Box<Type>),

    /// Generic dict
    Dict(Box<Type>, Box<Type>),

    /// defaultdict (dict subtype with factory for missing keys)
    DefaultDict(Box<Type>, Box<Type>),

    /// Generic set
    Set(Box<Type>),

    /// Tuple with specific element types (fixed-length heterogeneous).
    /// `tuple[int, str, float]` — element count known at compile-time;
    /// `t[k]` with literal `k` narrows to the exact element type.
    Tuple(Vec<Type>),

    /// Variable-length homogeneous tuple (PEP 484 / PEP 585 `tuple[T, ...]`).
    /// Element count is runtime-only; `t[k]` always returns `T` and needs a
    /// runtime bounds-check. Shares the same runtime layout as `Tuple` —
    /// `rt_tuple_*` functions work uniformly on both.
    TupleVar(Box<Type>),

    /// Union type (includes Optional via Union[T, None])
    /// Stored as a Vec with unique elements (normalized)
    Union(Vec<Type>),

    /// Function type
    Function {
        params: Vec<Type>,
        ret: Box<Type>,
    },

    /// Type variable (for generics)
    Var(InternedString),

    /// Unknown/Any type (for gradual typing) — may be raw i64 or heap pointer
    Any,

    /// Heap-allocated Any — guaranteed to be a valid *mut Obj pointer.
    /// Used for runtime-dispatched subscript results (rt_any_getitem),
    /// ObjectMethodCall returns, and similar cases where the value is
    /// always a boxed heap object. Print and compare use runtime dispatch.
    HeapAny,

    /// User-defined class type
    Class {
        class_id: ClassId,
        name: InternedString,
    },

    /// Iterator type (holds element type)
    Iterator(Box<Type>),

    /// Built-in exception types (for try/except type checking)
    /// Uses BuiltinExceptionKind enum from exceptions module
    BuiltinException(BuiltinExceptionKind),

    /// File type (for open() builtin). The boolean discriminates text mode
    /// (`false`) from binary mode (`true`, e.g. `open(p, "rb")`), because
    /// `.read()` / `.readline()` return `str` in text mode but `bytes` in
    /// binary mode and both need distinct static return-type inference.
    File(bool),

    /// Runtime object type from stdlib (StructTime, CompletedProcess, etc.)
    /// Uses TypeTagKind as single source of truth from core-defs.
    /// Field/method definitions are in stdlib-defs/object_types.rs
    RuntimeObject(TypeTagKind),

    /// Bottom type (empty union, uninhabited type)
    /// Represents the type with no values - used for empty unions
    /// and unreachable code. Never is a subtype of all types.
    Never,

    /// Singleton type for the `NotImplemented` sentinel. Operator dunders
    /// that don't know how to handle an operand return `NotImplemented`,
    /// which signals the interpreter to try the reflected dunder on the
    /// right operand. Treated like `Never` when unioned with other types
    /// for the purpose of return-type inference — it is a control-flow
    /// signal, not a real result value.
    NotImplementedT,
}

impl Type {
    /// Create an Optional type (sugar for Union[T, None])
    pub fn optional(t: Type) -> Type {
        // Ensure unique elements
        if t == Type::None {
            Type::None
        } else {
            Type::Union(vec![t, Type::None])
        }
    }

    /// Check if this is None type
    pub fn is_none(&self) -> bool {
        matches!(self, Type::None)
    }

    /// Check if this is a built-in exception type
    pub fn is_builtin_exception(&self) -> bool {
        matches!(self, Type::BuiltinException(_))
    }

    /// Get the exception type tag for built-in exceptions (0-13).
    /// Returns None if not a built-in exception type.
    pub fn builtin_exception_type_tag(&self) -> Option<u8> {
        match self {
            Type::BuiltinException(kind) => Some(kind.tag()),
            _ => None,
        }
    }

    /// Get the BuiltinExceptionKind if this is a built-in exception type.
    pub fn as_builtin_exception(&self) -> Option<BuiltinExceptionKind> {
        match self {
            Type::BuiltinException(kind) => Some(*kind),
            _ => None,
        }
    }

    /// Returns true if values of this type are heap-allocated and need pointer storage/GC tracking.
    /// Raw primitives (Int, Bool, Float, None) are stored as immediate values.
    /// Compile-time-only types (Function, Var, Never) are not heap-allocated.
    pub fn is_heap(&self) -> bool {
        !matches!(
            self,
            Type::Int
                | Type::Bool
                | Type::Float
                | Type::None
                | Type::Function { .. }
                | Type::Var(_)
                | Type::Never
        )
    }

    /// Check if this is a Union type
    pub fn is_union(&self) -> bool {
        matches!(self, Type::Union(_))
    }

    /// Normalize a union type (flatten nested unions, remove duplicates)
    pub fn normalize_union(types: Vec<Type>) -> Type {
        let mut result = Vec::new();
        for ty in types {
            match ty {
                Type::Union(inner) => {
                    for t in inner {
                        if !result.contains(&t) {
                            result.push(t);
                        }
                    }
                }
                t => {
                    if !result.contains(&t) {
                        result.push(t);
                    }
                }
            }
        }

        if result.is_empty() {
            Type::Never
        } else if result.len() == 1 {
            result
                .into_iter()
                .next()
                .expect("union simplification must produce at least one type when len==1")
        } else {
            Type::Union(result)
        }
    }

    /// Narrow a Union type when isinstance(x, T) is true.
    /// Returns the narrowed type (the intersection of self with target).
    /// For Union[int, str] narrowed to int -> int
    /// For non-Union types, returns self if it matches target, otherwise Never.
    ///
    /// Returns Type::Never if no types match the target, indicating unreachable code.
    pub fn narrow_to(&self, target: &Type) -> Type {
        // Tuple-of-types `isinstance(x, (int, float))` is lowered to a
        // `TypeRef(Union[int, float])`. Narrowing against a Union target
        // means "narrow to ANY of these" — compute per-member and union.
        if let Type::Union(target_members) = target {
            let candidates: Vec<Type> = target_members
                .iter()
                .map(|m| self.narrow_to(m))
                .filter(|t| !matches!(t, Type::Never))
                .collect();
            return Type::normalize_union(candidates);
        }
        match self {
            Type::Union(types) => {
                // Find all types in the union that match the target
                let matching: Vec<Type> = types
                    .iter()
                    .filter(|t| Self::types_match_for_isinstance(t, target))
                    .cloned()
                    .collect();

                if matching.is_empty() {
                    // No matching types - return Never (unreachable code)
                    Type::Never
                } else if matching.len() == 1 {
                    matching
                        .into_iter()
                        .next()
                        .expect("type narrowing must produce at least one type when len==1")
                } else {
                    Type::Union(matching)
                }
            }
            // `Any` / `HeapAny` could be anything at runtime — `isinstance`
            // is the compiler's only tool to commit to a concrete shape, so
            // the then-branch gets narrowed straight to the target type.
            // Without this, `len(x)` after `isinstance(x, str)` (where `x`
            // is typed `Any`) falls through to the `Type::Never` branch of
            // `select_len_func` and silently returns 0.
            Type::Any | Type::HeapAny => target.clone(),
            // For non-Union types, return self if it matches target
            _ => {
                if Self::types_match_for_isinstance(self, target) {
                    self.clone()
                } else {
                    // Doesn't match - return Never (unreachable code)
                    Type::Never
                }
            }
        }
    }

    /// Narrow a Union type when isinstance(x, T) is false.
    /// Returns the type excluding the target.
    /// For Union[int, str] excluding int -> str
    pub fn narrow_excluding(&self, excluded: &Type) -> Type {
        // Union-valued exclusion (`isinstance(x, (int, float))` in the
        // else-branch): exclude each member sequentially.
        if let Type::Union(excluded_members) = excluded {
            return excluded_members
                .iter()
                .fold(self.clone(), |acc, m| acc.narrow_excluding(m));
        }
        match self {
            Type::Union(types) => {
                // Keep all types that don't match the excluded type
                let remaining: Vec<Type> = types
                    .iter()
                    .filter(|t| !Self::types_match_for_isinstance(t, excluded))
                    .cloned()
                    .collect();

                if remaining.is_empty() {
                    // All types excluded - return Never (bottom type)
                    Type::Never
                } else if remaining.len() == 1 {
                    remaining
                        .into_iter()
                        .next()
                        .expect("type exclusion must produce at least one type when len==1")
                } else {
                    Type::Union(remaining)
                }
            }
            // For non-Union types, excluding the matched type makes the
            // branch unreachable; otherwise the type is unchanged.
            _ => {
                if Self::types_match_for_isinstance(self, excluded) {
                    Type::Never
                } else {
                    self.clone()
                }
            }
        }
    }

    /// Check if two types match for isinstance purposes.
    /// This is used for type narrowing - we check if the runtime type
    /// would be considered an instance of the target type.
    fn types_match_for_isinstance(actual: &Type, target: &Type) -> bool {
        match (actual, target) {
            // Exact primitive matches
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Str, Type::Str) => true,
            (Type::Bytes, Type::Bytes) => true,
            (Type::None, Type::None) => true,

            // Bool is a subtype of Int (Python: isinstance(True, int) == True)
            (Type::Bool, Type::Int) => true,

            // Container types - match by container kind (ignore element types)
            (Type::List(_), Type::List(_)) => true,
            (Type::Dict(_, _), Type::Dict(_, _)) => true,
            (Type::DefaultDict(_, _), Type::DefaultDict(_, _)) => true,
            // defaultdict is a subtype of dict
            (Type::DefaultDict(_, _), Type::Dict(_, _)) => true,
            (Type::Set(_), Type::Set(_)) => true,
            // Both fixed and variable-length tuples match under isinstance(_, tuple).
            (Type::Tuple(_) | Type::TupleVar(_), Type::Tuple(_) | Type::TupleVar(_)) => true,

            // Class types - match by class_id
            // Note: Inheritance is handled at runtime via rt_isinstance_class_inherited.
            // This compile-time check only handles exact class matches.
            // For full inheritance support, the class hierarchy context would be needed.
            (
                Type::Class {
                    class_id: id1,
                    name: _,
                },
                Type::Class {
                    class_id: id2,
                    name: _,
                },
            ) => id1 == id2,

            // Iterator types
            (Type::Iterator(_), Type::Iterator(_)) => true,

            // File type — text and binary files are structurally the same
            // for dispatch purposes (both accept `.read()` etc.), so any
            // `File(_)` matches any other `File(_)`. The binary flag only
            // affects return-type inference, not structural equality.
            (Type::File(_), Type::File(_)) => true,

            // Runtime object types match by TypeTagKind
            (Type::RuntimeObject(k1), Type::RuntimeObject(k2)) => k1 == k2,

            // Built-in exception types match by kind
            (Type::BuiltinException(k1), Type::BuiltinException(k2)) => k1 == k2,

            // Everything else doesn't match
            _ => false,
        }
    }

    /// Unify two tuple shapes into a common supertype.
    ///
    /// - Same length → element-wise union, keep fixed shape.
    /// - Different lengths → `TupleVar(union of all elements)`.
    /// - Empty tuple is absorbed by any other tuple shape.
    /// - Fixed ∪ Variable → absorbs fixed into variable.
    /// - Non-tuple pair → falls back to `normalize_union`.
    ///
    /// Used by class-field unification (Area D §D.3.6) when the same
    /// `self.<field>` is assigned tuples of different shapes in different
    /// methods; see `ast_to_hir/classes.rs` for the call site.
    pub fn unify_tuple_shapes(a: &Type, b: &Type) -> Type {
        match (a, b) {
            // Empty tuple is absorbed by any other tuple shape.
            (Type::Tuple(es), other) if es.is_empty() => Self::absorb_empty_tuple_into(other),
            (other, Type::Tuple(es)) if es.is_empty() => Self::absorb_empty_tuple_into(other),

            // Same length → element-wise union, keep fixed shape.
            (Type::Tuple(ts1), Type::Tuple(ts2)) if ts1.len() == ts2.len() => Type::Tuple(
                ts1.iter()
                    .zip(ts2)
                    .map(|(a, b)| Type::normalize_union(vec![a.clone(), b.clone()]))
                    .collect(),
            ),

            // Different lengths → collapse to TupleVar(union-of-all-elements).
            (Type::Tuple(ts1), Type::Tuple(ts2)) => {
                let elems: Vec<Type> = ts1.iter().chain(ts2.iter()).cloned().collect();
                Type::TupleVar(Box::new(Type::normalize_union(elems)))
            }

            // Variable ∪ variable.
            (Type::TupleVar(e1), Type::TupleVar(e2)) => {
                Type::TupleVar(Box::new(Type::normalize_union(vec![
                    (**e1).clone(),
                    (**e2).clone(),
                ])))
            }

            // Fixed ∪ variable → absorb fixed into variable.
            (Type::Tuple(ts), Type::TupleVar(e)) | (Type::TupleVar(e), Type::Tuple(ts)) => {
                let mut elems = vec![(**e).clone()];
                elems.extend(ts.iter().cloned());
                Type::TupleVar(Box::new(Type::normalize_union(elems)))
            }

            _ => Type::normalize_union(vec![a.clone(), b.clone()]),
        }
    }

    fn absorb_empty_tuple_into(other: &Type) -> Type {
        match other {
            Type::Tuple(_) | Type::TupleVar(_) => other.clone(),
            _ => Type::normalize_union(vec![Type::Tuple(vec![]), other.clone()]),
        }
    }

    /// PEP 3141 numeric tower: `bool ⊂ int ⊂ float`. Returns the wider of two
    /// numeric types, or `None` if either argument is non-numeric.
    ///
    /// Used by [`unify_numeric`] and [`unify_field_type`] for cross-site
    /// type unification at class fields and local variables (Area E).
    pub fn promote_numeric(a: &Type, b: &Type) -> Option<Type> {
        match (a, b) {
            (Type::Float, Type::Float)
            | (Type::Float, Type::Int)
            | (Type::Float, Type::Bool)
            | (Type::Int, Type::Float)
            | (Type::Bool, Type::Float) => Some(Type::Float),
            (Type::Int, Type::Int) | (Type::Int, Type::Bool) | (Type::Bool, Type::Int) => {
                Some(Type::Int)
            }
            (Type::Bool, Type::Bool) => Some(Type::Bool),
            _ => None,
        }
    }

    /// Unify two types for a binding site using the numeric tower where
    /// applicable, else fall back to [`normalize_union`].
    ///
    /// - `{Bool, Int}`        → `Int`
    /// - `{Int, Float}`       → `Float`
    /// - `{Bool, Int, Float}` → `Float`
    /// - `{Int, Str}`         → `Union[Int, Str]` (non-numeric → union)
    pub fn unify_numeric(a: &Type, b: &Type) -> Type {
        if let Some(t) = Type::promote_numeric(a, b) {
            return t;
        }
        Type::normalize_union(vec![a.clone(), b.clone()])
    }

    /// Single entry-point for every cross-site unification call (class
    /// fields and local variables). Layers the Area D tuple-shape rule
    /// on top of the numeric tower so both generalisations apply in one
    /// place.
    ///
    /// When either argument is tuple-shaped, defers to
    /// [`unify_tuple_shapes`]; otherwise applies [`unify_numeric`].
    pub fn unify_field_type(a: &Type, b: &Type) -> Type {
        if matches!(a, Type::Tuple(_) | Type::TupleVar(_))
            || matches!(b, Type::Tuple(_) | Type::TupleVar(_))
        {
            return Type::unify_tuple_shapes(a, b);
        }
        Type::unify_numeric(a, b)
    }

    /// Check if type is a subtype of another.
    /// Delegates to `is_subtype_of_inner`; the inherent method is kept for
    /// legacy callers and will be removed in §S3.1 step 7.
    pub fn is_subtype_of(&self, other: &Type) -> bool {
        Type::is_subtype_of_inner(self, other)
    }
}

// ============================================================================
// TypeLattice trait — Phase 3 §S3.1
// ============================================================================

/// A bounded lattice on types.
///
/// Laws (all enforced by the property tests in `tests::lattice_props`):
/// - `join(top, t) == top`, `meet(top, t) == t`
/// - `join(bot, t) == t`, `meet(bot, t) == bot`
/// - `join` and `meet` are commutative, associative, idempotent
/// - `is_subtype_of` is the partial order induced by the lattice:
///   `a ≤ b ⟺ join(a,b) == b ⟺ meet(a,b) == a`
/// - Antisymmetry: `a ≤ b ∧ b ≤ a ⟹ a == b`
pub trait TypeLattice: Sized + Clone + Eq {
    /// Universal supertype (`Any` in Python terms). `join(top, t) == top`.
    fn top() -> Self;
    /// Universal subtype (`Never` / `typing.Never`). `join(bot, t) == t`.
    fn bottom() -> Self;
    /// Least upper bound: the most specific type that is a supertype of both.
    fn join(&self, other: &Self) -> Self;
    /// Greatest lower bound: the most specific type that is a subtype of both.
    fn meet(&self, other: &Self) -> Self;
    /// Subtype relation: `self ≤ other`.
    fn is_subtype_of(&self, other: &Self) -> bool;
    /// Set difference `self \ other`: remove `other` from `self`.
    /// Used for `isinstance` else-branch type narrowing.
    fn minus(&self, other: &Self) -> Self;
}

// ============================================================================
// Canonical ordering helpers for union normalisation
// ============================================================================

/// Stable sort key for `Type` variants so that `Union` members are always in
/// a canonical order regardless of which operand was left/right in `join`.
fn type_discriminant(t: &Type) -> u32 {
    match t {
        Type::Never => 0,
        Type::Bool => 1,
        Type::Int => 2,
        Type::Float => 3,
        Type::Str => 4,
        Type::Bytes => 5,
        Type::None => 6,
        Type::List(_) => 7,
        Type::Dict(_, _) => 8,
        Type::DefaultDict(_, _) => 9,
        Type::Set(_) => 10,
        Type::Tuple(_) => 11,
        Type::TupleVar(_) => 12,
        Type::Iterator(_) => 13,
        Type::Function { .. } => 14,
        Type::Var(_) => 15,
        Type::Class { .. } => 16,
        Type::BuiltinException(_) => 17,
        Type::File(_) => 18,
        Type::RuntimeObject(_) => 19,
        Type::NotImplementedT => 20,
        Type::HeapAny => 21,
        Type::Any => 22,
        // Union should never appear as a member of another union after collection.
        Type::Union(_) => u32::MAX,
    }
}

/// Build a canonical `Type::Union` from a set of member types:
/// 1. Flatten any nested unions.
/// 2. Deduplicate.
/// 3. Remove `Never` (bottom identity).
/// 4. Absorb into `Any` if any member is `Any`/`HeapAny`.
/// 5. Remove members subsumed by another member (`Bool` removed when `Int` present).
/// 6. Sort by `(discriminant, display-string)` for commutativity.
fn make_canonical_union(members: impl IntoIterator<Item = Type>) -> Type {
    let mut flat: Vec<Type> = Vec::new();
    for m in members {
        match m {
            Type::Union(ts) => flat.extend(ts),
            other => flat.push(other),
        }
    }

    // Remove Never.
    flat.retain(|t| *t != Type::Never);

    // Absorb Any/HeapAny.
    if flat.iter().any(|t| matches!(t, Type::Any | Type::HeapAny)) {
        return Type::Any;
    }

    // Deduplicate.
    flat.dedup_by(|a, b| a == b); // only removes consecutive dups; full dedup below
    let mut deduped: Vec<Type> = Vec::with_capacity(flat.len());
    for t in flat {
        if !deduped.contains(&t) {
            deduped.push(t);
        }
    }

    // Remove subsumed members: keep `m` only if no other member `o ≠ m`
    // covers `m`. Covering means either `m ≤ o` (subtyping) or the numeric
    // tower promotes `m` to `o` (e.g., Int is subsumed by Float).
    let to_remove: Vec<usize> = deduped
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let subsumed = deduped.iter().enumerate().any(|(j, o)| {
                j != i && (m.is_subtype_of(o) || Type::promote_numeric(m, o).as_ref() == Some(o))
            });
            subsumed.then_some(i)
        })
        .collect();
    for i in to_remove.into_iter().rev() {
        deduped.remove(i);
    }

    // Canonical sort: primary key = discriminant, secondary key = Display string.
    deduped.sort_by(|a, b| {
        let da = type_discriminant(a);
        let db = type_discriminant(b);
        da.cmp(&db)
            .then_with(|| format!("{a}").cmp(&format!("{b}")))
    });

    match deduped.len() {
        0 => Type::Never,
        1 => deduped.into_iter().next().expect("len==1"),
        _ => Type::Union(deduped),
    }
}

// ============================================================================
// TypeLattice implementation for Type
// ============================================================================

impl TypeLattice for Type {
    fn top() -> Self {
        Type::Any
    }

    fn bottom() -> Self {
        Type::Never
    }

    /// Least upper bound with the numeric tower, canonical union ordering, and
    /// subsumed-member removal.
    ///
    /// Design decisions:
    /// - `Any`/`HeapAny` is the top element: `join(Any, t) == Any`.
    /// - `Never` is the bottom element: `join(Never, t) == t`.
    /// - Numeric tower (`Bool ⊂ Int ⊂ Float`) collapses numeric pairs.
    /// - Unions are flattened, deduplicated, simplified (subsumed members
    ///   removed), and sorted canonically so that `join(a,b) == join(b,a)`.
    fn join(&self, other: &Self) -> Self {
        // top absorbs
        if matches!(self, Type::Any | Type::HeapAny) || matches!(other, Type::Any | Type::HeapAny) {
            return Type::Any;
        }
        // bottom is identity
        if *self == Type::Never {
            return other.clone();
        }
        if *other == Type::Never {
            return self.clone();
        }
        // reflexivity fast path
        if self == other {
            return self.clone();
        }
        // numeric tower: Bool ⊂ Int ⊂ Float
        if let Some(t) = Type::promote_numeric(self, other) {
            return t;
        }
        // General case: collect, flatten, simplify, sort.
        make_canonical_union([self.clone(), other.clone()])
    }

    /// Greatest lower bound using the subtype partial order.
    ///
    /// - `meet(top, t) == t`   (`Any` is the identity for meet)
    /// - `meet(bot, t) == bot` (`Never` absorbs)
    /// - `meet(a, b) == a` when `a ≤ b`
    /// - `meet(a, b) == b` when `b ≤ a`
    /// - Union on either side: distribute meet over union members and join
    ///   the non-Never results.
    fn meet(&self, other: &Self) -> Self {
        // top is identity for meet
        if matches!(self, Type::Any | Type::HeapAny) {
            return other.clone();
        }
        if matches!(other, Type::Any | Type::HeapAny) {
            return self.clone();
        }
        // bottom absorbs
        if *self == Type::Never || *other == Type::Never {
            return Type::Never;
        }
        // reflexivity
        if self == other {
            return self.clone();
        }
        // subtype shortcuts (symmetric)
        if self.is_subtype_of(other) {
            return self.clone();
        }
        if other.is_subtype_of(self) {
            return other.clone();
        }
        // distribute over unions
        match (self, other) {
            (Type::Union(ts), _) => ts
                .iter()
                .map(|t| t.meet(other))
                .filter(|t| *t != Type::Never)
                .fold(Type::Never, |acc, t| acc.join(&t)),
            (_, Type::Union(ts)) => ts
                .iter()
                .map(|t| self.meet(t))
                .filter(|t| *t != Type::Never)
                .fold(Type::Never, |acc, t| acc.join(&t)),
            _ => Type::Never,
        }
    }

    /// Subtype relation.  Delegates to the inherent `is_subtype_of` method
    /// (same body; the inherent version will be deleted in S3.1 step 7).
    fn is_subtype_of(&self, other: &Self) -> bool {
        // Delegate to the inherent method — same implementation, avoids
        // duplication while both coexist during migration.
        Type::is_subtype_of_inner(self, other)
    }

    /// Set difference `self \ other` for `isinstance` else-branch narrowing.
    ///
    /// - `minus(Any, t) == Any`   (can't represent "Any except T")
    /// - `minus(Never, t) == Never`
    /// - `minus(t, Never) == t`   (removing nothing)
    /// - `minus(t, Any) == Never` (removing everything)
    /// - `minus(t, t) == Never`
    /// - `minus(a, b) == Never` when `a ≤ b` (a is fully removed)
    /// - Unions: filter out subsumed members.
    fn minus(&self, other: &Self) -> Self {
        if matches!(self, Type::Any | Type::HeapAny) {
            return self.clone();
        }
        if *self == Type::Never {
            return Type::Never;
        }
        if *other == Type::Never {
            return self.clone();
        }
        if matches!(other, Type::Any | Type::HeapAny) {
            return Type::Never;
        }
        if self == other || self.is_subtype_of(other) {
            return Type::Never;
        }
        // Union on right: subtract each member sequentially.
        if let Type::Union(excluded) = other {
            return excluded.iter().fold(self.clone(), |acc, m| acc.minus(m));
        }
        // Union on left: keep members not subsumed by `other`.
        if let Type::Union(ts) = self {
            let remaining: Vec<Type> = ts
                .iter()
                .map(|t| t.minus(other))
                .filter(|t| *t != Type::Never)
                .collect();
            return remaining
                .into_iter()
                .fold(Type::Never, |acc, t| acc.join(&t));
        }
        // Concrete type not subsumed by other: keep self.
        self.clone()
    }
}

impl Type {
    /// Inherent implementation of the subtype relation, shared by both the
    /// inherent `is_subtype_of` (legacy callers) and the `TypeLattice` trait
    /// impl.  Will be inlined into the trait method and removed as an
    /// inherent function in §S3.1 step 7.
    fn is_subtype_of_inner(this: &Type, other: &Type) -> bool {
        match (this, other) {
            // Reflexivity
            (a, b) if a == b => true,

            // Never is subtype of everything (bottom type)
            (Type::Never, _) => true,
            // Nothing is subtype of Never (except Never itself, handled by reflexivity)
            (_, Type::Never) => false,

            // Any and HeapAny are supertypes of everything
            (_, Type::Any) | (_, Type::HeapAny) => true,
            (Type::Any, _) | (Type::HeapAny, _) => false,

            // Bool is subtype of Int (Python semantics: isinstance(True, int) == True)
            (Type::Bool, Type::Int) => true,

            // None is subtype of Optional[T]
            (Type::None, Type::Union(set)) if set.contains(&Type::None) => true,

            // Union subtyping: all members of left must be subtypes of right
            (Type::Union(left), right) => left.iter().all(|t| Self::is_subtype_of_inner(t, right)),

            // Right is union: left must be subtype of at least one member
            (left, Type::Union(right)) => right.iter().any(|t| Self::is_subtype_of_inner(left, t)),

            // Covariant container subtyping (see is_subtype_of for the rationale)
            (Type::List(a), Type::List(b)) => {
                **a == Type::Any || **b == Type::Any || Self::is_subtype_of_inner(a, b)
            }
            (Type::Set(a), Type::Set(b)) => {
                **a == Type::Any || **b == Type::Any || Self::is_subtype_of_inner(a, b)
            }
            (Type::Dict(k1, v1), Type::Dict(k2, v2))
            | (Type::DefaultDict(k1, v1), Type::DefaultDict(k2, v2))
            | (Type::DefaultDict(k1, v1), Type::Dict(k2, v2)) => {
                (**k1 == Type::Any || **k2 == Type::Any || Self::is_subtype_of_inner(k1, k2))
                    && (**v1 == Type::Any || **v2 == Type::Any || Self::is_subtype_of_inner(v1, v2))
            }
            (Type::Tuple(ts1), Type::Tuple(ts2)) => {
                ts1.len() == ts2.len()
                    && ts1
                        .iter()
                        .zip(ts2.iter())
                        .all(|(t1, t2)| *t1 == Type::Any || Self::is_subtype_of_inner(t1, t2))
            }
            (Type::Tuple(ts), Type::TupleVar(elem)) => ts
                .iter()
                .all(|t| *t == Type::Any || Self::is_subtype_of_inner(t, elem)),
            (Type::TupleVar(a), Type::TupleVar(b)) => {
                **a == Type::Any || Self::is_subtype_of_inner(a, b)
            }
            (
                Type::Function {
                    params: p1,
                    ret: r1,
                },
                Type::Function {
                    params: p2,
                    ret: r2,
                },
            ) => {
                p1.len() == p2.len()
                    && p2
                        .iter()
                        .zip(p1.iter())
                        .all(|(t2, t1)| Self::is_subtype_of_inner(t2, t1))
                    && Self::is_subtype_of_inner(r1, r2)
            }
            (
                Type::Class {
                    class_id: id1,
                    name: _,
                },
                Type::Class {
                    class_id: id2,
                    name: _,
                },
            ) => id1 == id2,
            (Type::Iterator(a), Type::Iterator(b)) => {
                **a == Type::Any || Self::is_subtype_of_inner(a, b)
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::Bool => write!(f, "bool"),
            Type::Str => write!(f, "str"),
            Type::Bytes => write!(f, "bytes"),
            Type::None => write!(f, "None"),
            Type::List(t) => write!(f, "list[{}]", t),
            Type::Dict(k, v) => write!(f, "dict[{}, {}]", k, v),
            Type::DefaultDict(k, v) => write!(f, "defaultdict[{}, {}]", k, v),
            Type::Set(t) => write!(f, "set[{}]", t),
            Type::Tuple(ts) => {
                write!(f, "tuple[")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", t)?;
                }
                write!(f, "]")
            }
            Type::TupleVar(t) => write!(f, "tuple[{}, ...]", t),
            Type::Union(ts) => {
                let types: Vec<_> = ts.iter().collect();
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", t)?;
                }
                Ok(())
            }
            Type::Function { params, ret } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            Type::Var(name) => write!(f, "'{:?}", name),
            Type::Any | Type::HeapAny => write!(f, "Any"),
            Type::Class { name, .. } => write!(f, "{:?}", name),
            Type::Iterator(t) => write!(f, "Iterator[{}]", t),
            Type::BuiltinException(kind) => write!(f, "{}", kind),
            Type::File(binary) => write!(f, "File({})", if *binary { "binary" } else { "text" }),
            Type::RuntimeObject(kind) => write!(f, "{}", kind),
            Type::Never => write!(f, "Never"),
            Type::NotImplementedT => write!(f, "NotImplementedType"),
        }
    }
}

/// Type environment for type checking
#[derive(Debug, Default)]
pub struct TypeEnv {
    types: Vec<Type>,
}

impl TypeEnv {
    pub fn new() -> Self {
        Self { types: Vec::new() }
    }

    pub fn insert(&mut self, ty: Type) -> TypeId {
        let id = self.types.len() as TypeId;
        self.types.push(ty);
        id
    }

    pub fn get(&self, id: TypeId) -> Option<&Type> {
        self.types.get(id as usize)
    }
}

// ============================================================================
// TypeSpec conversion (from stdlib-defs)
// ============================================================================

use pyaot_stdlib_defs::TypeSpec;

/// Convert TypeSpec (from stdlib-defs) to Type
/// Used for type inference from stdlib function definitions
pub fn typespec_to_type(spec: &TypeSpec) -> Type {
    match spec {
        TypeSpec::Int => Type::Int,
        TypeSpec::Float => Type::Float,
        TypeSpec::Bool => Type::Bool,
        TypeSpec::Str => Type::Str,
        TypeSpec::None => Type::None,
        TypeSpec::Bytes => Type::Bytes,
        TypeSpec::List(elem) => Type::List(Box::new(typespec_to_type(elem))),
        TypeSpec::Dict(k, v) => {
            Type::Dict(Box::new(typespec_to_type(k)), Box::new(typespec_to_type(v)))
        }
        TypeSpec::Tuple(elem) => Type::Tuple(vec![typespec_to_type(elem)]),
        TypeSpec::Set(elem) => Type::Set(Box::new(typespec_to_type(elem))),
        TypeSpec::Optional(inner) => Type::optional(typespec_to_type(inner)),
        TypeSpec::Any => Type::Any,
        TypeSpec::Iterator(elem) => Type::Iterator(Box::new(typespec_to_type(elem))),
        // TypeSpec::File carries no mode info — stdlib signatures that name
        // `File` as a param/return type default to text mode.
        TypeSpec::File => Type::File(false),
        // Runtime object types - use TypeTagKind as single source of truth
        TypeSpec::Match => Type::RuntimeObject(TypeTagKind::Match),
        TypeSpec::StructTime => Type::RuntimeObject(TypeTagKind::StructTime),
        TypeSpec::CompletedProcess => Type::RuntimeObject(TypeTagKind::CompletedProcess),
        TypeSpec::ParseResult => Type::RuntimeObject(TypeTagKind::ParseResult),
        TypeSpec::HttpResponse => Type::RuntimeObject(TypeTagKind::HttpResponse),
        TypeSpec::Request => Type::RuntimeObject(TypeTagKind::Request),
        TypeSpec::Hash => Type::RuntimeObject(TypeTagKind::Hash),
        TypeSpec::StringIO => Type::RuntimeObject(TypeTagKind::StringIO),
        TypeSpec::BytesIO => Type::RuntimeObject(TypeTagKind::BytesIO),
        TypeSpec::Deque => Type::RuntimeObject(TypeTagKind::Deque),
        TypeSpec::Counter => Type::RuntimeObject(TypeTagKind::Counter),
    }
}

#[cfg(test)]
mod tests;
