//! Type system for the Python AOT compiler

#![forbid(unsafe_code)]

pub mod builtin_classes;
pub mod dunders;
pub mod exceptions;
pub mod tag_kinds;

pub use builtin_classes::{
    BUILTIN_DICT_CLASS_ID, BUILTIN_LIST_CLASS_ID, BUILTIN_SET_CLASS_ID, BUILTIN_TUPLE_CLASS_ID,
    BUILTIN_TUPLE_VAR_CLASS_ID,
};
pub use exceptions::{
    exception_name_to_tag, exception_tag_to_name, is_builtin_exception_name, BuiltinException,
    BuiltinExceptionKind, BUILTIN_EXCEPTIONS, BUILTIN_EXCEPTION_COUNT, FIRST_USER_CLASS_ID,
    RESERVED_BUILTIN_TYPE_SLOTS, RESERVED_STDLIB_EXCEPTION_SLOTS,
};

pub use tag_kinds::{is_type_tag_name, type_tag_to_name, TypeTagKind, TYPE_TAG_COUNT};

use pyaot_utils::{ClassId, InternedString};
use std::collections::{HashMap, HashSet};

/// Type identifier
pub type TypeId = u32;

/// TypeVar definition (for generic functions).
/// Populated by `T = TypeVar('T', ...)` declarations in Python source.
#[derive(Debug, Clone)]
pub struct TypeVarDef {
    /// Constraint types: `TypeVar('T', int, str)` → `[int, str]`
    pub constraints: Vec<Type>,
    /// Upper bound: `TypeVar('T', bound=SomeType)` → `Some(SomeType)`
    pub bound: Option<Type>,
}

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

    /// defaultdict (dict subtype with factory for missing keys)
    DefaultDict(Box<Type>, Box<Type>),

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

    /// Unified generic type (§S3.2). Represents both built-in containers
    /// (`list[T]`, `dict[K,V]`, `set[T]`, `tuple[...]`) and user-defined
    /// generic classes (`Stack[T]`) with a single, uniform representation.
    ///
    /// For built-in containers the `base` is one of the constants in
    /// `builtin_classes`: `BUILTIN_LIST_CLASS_ID`, `BUILTIN_DICT_CLASS_ID`,
    /// `BUILTIN_SET_CLASS_ID`, `BUILTIN_TUPLE_CLASS_ID`,
    /// `BUILTIN_TUPLE_VAR_CLASS_ID`.
    ///
    /// The legacy `List`/`Dict`/`Set`/`Tuple`/`TupleVar` variants coexist
    /// with `Generic` during S3.2b (accessor-migration phase) and are deleted
    /// in S3.2c once every call site uses the accessor API.
    Generic {
        base: ClassId,
        args: Vec<Type>,
    },

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

    // -------------------------------------------------------------------------
    // TypeVar / generics API (S3.3)
    // -------------------------------------------------------------------------

    /// Returns `true` if this type or any nested type is `Type::Var(_)`.
    pub fn contains_var(&self) -> bool {
        match self {
            Type::Var(_) => true,
            Type::Union(ts) => ts.iter().any(|t| t.contains_var()),
            Type::Generic { args, .. } => args.iter().any(|t| t.contains_var()),
            Type::DefaultDict(k, v) => k.contains_var() || v.contains_var(),
            Type::Iterator(t) => t.contains_var(),
            Type::Function { params, ret } => {
                params.iter().any(|t| t.contains_var()) || ret.contains_var()
            }
            _ => false,
        }
    }

    /// Collects all distinct `Type::Var` names reachable from this type.
    pub fn collect_var_names(&self, out: &mut HashSet<InternedString>) {
        match self {
            Type::Var(name) => {
                out.insert(*name);
            }
            Type::Union(ts) => ts.iter().for_each(|t| t.collect_var_names(out)),
            Type::Generic { args, .. } => args.iter().for_each(|t| t.collect_var_names(out)),
            Type::DefaultDict(k, v) => {
                k.collect_var_names(out);
                v.collect_var_names(out);
            }
            Type::Iterator(t) => t.collect_var_names(out),
            Type::Function { params, ret } => {
                params.iter().for_each(|t| t.collect_var_names(out));
                ret.collect_var_names(out);
            }
            _ => {}
        }
    }

    /// Substitute all `Type::Var(name)` leaves using the given map.
    /// Unmapped names are left as `Type::Var(name)`.
    pub fn substitute(&self, subst: &HashMap<InternedString, Type>) -> Type {
        match self {
            Type::Var(name) => subst.get(name).cloned().unwrap_or_else(|| self.clone()),
            Type::Union(ts) => Type::Union(ts.iter().map(|t| t.substitute(subst)).collect()),
            Type::Generic { base, args } => Type::Generic {
                base: *base,
                args: args.iter().map(|t| t.substitute(subst)).collect(),
            },
            Type::DefaultDict(k, v) => {
                Type::DefaultDict(Box::new(k.substitute(subst)), Box::new(v.substitute(subst)))
            }
            Type::Iterator(t) => Type::Iterator(Box::new(t.substitute(subst))),
            Type::Function { params, ret } => Type::Function {
                params: params.iter().map(|t| t.substitute(subst)).collect(),
                ret: Box::new(ret.substitute(subst)),
            },
            _ => self.clone(),
        }
    }

    // -------------------------------------------------------------------------
    // Container accessor API (S3.2) — works on both legacy variants and
    // `Type::Generic{base, args}` so call sites migrate once and survive S3.2c.
    // -------------------------------------------------------------------------

    /// Returns `true` for `List(T)` and `Generic{LIST_ID, [T]}`.
    pub fn is_list_like(&self) -> bool {
        self.list_elem().is_some()
    }

    /// Returns `true` for `Dict(K,V)`, `DefaultDict(K,V)`, and
    /// `Generic{DICT_ID, [K, V]}`.
    pub fn is_dict_like(&self) -> bool {
        self.dict_kv().is_some()
    }

    /// Returns `true` for `Set(T)` and `Generic{SET_ID, [T]}`.
    pub fn is_set_like(&self) -> bool {
        self.set_elem().is_some()
    }

    /// Returns `true` for `Tuple(...)` or `TupleVar(...)` and their `Generic` equivalents.
    pub fn is_tuple_like(&self) -> bool {
        self.tuple_elems().is_some() || self.tuple_var_elem().is_some()
    }

    /// Element type of a list, or `None` if this is not a list type.
    pub fn list_elem(&self) -> Option<&Type> {
        match self {
            Type::Generic { base, args }
                if *base == builtin_classes::BUILTIN_LIST_CLASS_ID && !args.is_empty() =>
            {
                Some(&args[0])
            }
            _ => None,
        }
    }

    /// Key/value types of a dict, or `None` if this is not a dict type.
    /// Matches `DefaultDict` and `Generic{DICT_ID, [K, V]}`.
    pub fn dict_kv(&self) -> Option<(&Type, &Type)> {
        match self {
            Type::DefaultDict(k, v) => Some((k, v)),
            Type::Generic { base, args }
                if *base == builtin_classes::BUILTIN_DICT_CLASS_ID && args.len() == 2 =>
            {
                Some((&args[0], &args[1]))
            }
            _ => None,
        }
    }

    /// Element type of a set, or `None` if this is not a set type.
    pub fn set_elem(&self) -> Option<&Type> {
        match self {
            Type::Generic { base, args }
                if *base == builtin_classes::BUILTIN_SET_CLASS_ID && !args.is_empty() =>
            {
                Some(&args[0])
            }
            _ => None,
        }
    }

    /// Element types of a fixed-arity tuple, or `None` if this is not one.
    /// Does NOT match `TupleVar` (variable-length homogeneous tuples).
    pub fn tuple_elems(&self) -> Option<&[Type]> {
        match self {
            Type::Generic { base, args } if *base == builtin_classes::BUILTIN_TUPLE_CLASS_ID => {
                Some(args)
            }
            _ => None,
        }
    }

    /// Element type of a variable-length homogeneous tuple (`tuple[T, ...]`),
    /// or `None` if this is not a `TupleVar`-like type.
    pub fn tuple_var_elem(&self) -> Option<&Type> {
        match self {
            Type::Generic { base, args }
                if *base == builtin_classes::BUILTIN_TUPLE_VAR_CLASS_ID && !args.is_empty() =>
            {
                Some(&args[0])
            }
            _ => None,
        }
    }

    /// Element type of an `Iterator[T]`, or `None` if not an iterator.
    pub fn iter_elem(&self) -> Option<&Type> {
        match self {
            Type::Iterator(elem) => Some(elem),
            _ => None,
        }
    }

    // -------------------------------------------------------------------------
    // Container constructors — emit `Type::Generic{base, args}` (S3.2c).
    // -------------------------------------------------------------------------

    /// Construct a `list[elem]` type.
    pub fn list_of(elem: Type) -> Type {
        Type::Generic {
            base: builtin_classes::BUILTIN_LIST_CLASS_ID,
            args: vec![elem],
        }
    }

    /// Construct a `dict[k, v]` type.
    pub fn dict_of(k: Type, v: Type) -> Type {
        Type::Generic {
            base: builtin_classes::BUILTIN_DICT_CLASS_ID,
            args: vec![k, v],
        }
    }

    /// Construct a `set[elem]` type.
    pub fn set_of(elem: Type) -> Type {
        Type::Generic {
            base: builtin_classes::BUILTIN_SET_CLASS_ID,
            args: vec![elem],
        }
    }

    /// Construct a fixed-arity `tuple[T1, T2, ...]` type.
    pub fn tuple_of(elems: Vec<Type>) -> Type {
        Type::Generic {
            base: builtin_classes::BUILTIN_TUPLE_CLASS_ID,
            args: elems,
        }
    }

    /// Construct a variable-length `tuple[T, ...]` type (PEP 484).
    pub fn tuple_var_of(elem: Type) -> Type {
        Type::Generic {
            base: builtin_classes::BUILTIN_TUPLE_VAR_CLASS_ID,
            args: vec![elem],
        }
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
        Type::DefaultDict(_, _) => 9,
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
        Type::Generic { base, .. } => 23 + base.0,
        // Union should never appear as a member of another union after collection.
        Type::Union(_) => u32::MAX,
    }
}

/// Build a canonical `Type::Union` from a set of member types:
/// PEP 3141 numeric tower: `bool ⊂ int ⊂ float`.
/// Returns the wider of two numeric types, or `None` if either is non-numeric.
fn numeric_promote(a: &Type, b: &Type) -> Option<Type> {
    match (a, b) {
        (Type::Float, Type::Float | Type::Int | Type::Bool)
        | (Type::Int | Type::Bool, Type::Float) => Some(Type::Float),
        (Type::Int, Type::Int | Type::Bool) | (Type::Bool, Type::Int) => Some(Type::Int),
        (Type::Bool, Type::Bool) => Some(Type::Bool),
        _ => None,
    }
}

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

    // Merge Generic members with the same base class via covariant element-wise
    // join. E.g. Union[list[int], list[float]] → list[float].
    // Repeat until stable (a merge may expose another mergeable pair).
    // Direct element-wise merge avoids re-entering make_canonical_union.
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < deduped.len() {
            if let Type::Generic { base: b1, args: a1 } = &deduped[i] {
                let base = *b1;
                let arity = a1.len();
                let j = deduped.iter().enumerate().position(|(k, t)| {
                    k != i
                        && matches!(t, Type::Generic { base: b2, args: a2 }
                                    if *b2 == base && a2.len() == arity)
                });
                if let Some(j) = j {
                    let hi = j.max(i);
                    let lo = j.min(i);
                    let a = deduped.remove(hi);
                    let b = deduped.remove(lo);
                    let (args_a, args_b) = match (&a, &b) {
                        (Type::Generic { args: aa, .. }, Type::Generic { args: ab, .. }) => {
                            (aa.clone(), ab.clone())
                        }
                        _ => unreachable!(),
                    };
                    let merged_args: Vec<Type> = args_a
                        .iter()
                        .zip(args_b.iter())
                        .map(|(t1, t2)| {
                            if let Some(n) = numeric_promote(t1, t2) {
                                n
                            } else if t1 == t2 {
                                t1.clone()
                            } else {
                                // Defer to join for non-trivial cases; this
                                // cannot re-enter make_canonical_union for same-
                                // base Generics because these args are not
                                // Generic with the same base as the outer pair.
                                t1.join(t2)
                            }
                        })
                        .collect();
                    deduped.push(Type::Generic {
                        base,
                        args: merged_args,
                    });
                    merged = true;
                    break;
                }
            }
            i += 1;
        }
        if !merged {
            break;
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
                j != i && (m.is_subtype_of(o) || numeric_promote(m, o).as_ref() == Some(o))
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
        if let Some(t) = numeric_promote(self, other) {
            return t;
        }
        // Covariant element-wise join for Generic containers with the same base.
        if let (Type::Generic { base: b1, args: a1 }, Type::Generic { base: b2, args: a2 }) =
            (self, other)
        {
            if b1 == b2 && a1.len() == a2.len() {
                return Type::Generic {
                    base: *b1,
                    args: a1
                        .iter()
                        .zip(a2.iter())
                        .map(|(t1, t2)| t1.join(t2))
                        .collect(),
                };
            }
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
            _ if this.list_elem().is_some() && other.list_elem().is_some() => {
                let (a, b) = (this.list_elem().unwrap(), other.list_elem().unwrap());
                *a == Type::Any || *b == Type::Any || Self::is_subtype_of_inner(a, b)
            }
            _ if this.set_elem().is_some() && other.set_elem().is_some() => {
                let (a, b) = (this.set_elem().unwrap(), other.set_elem().unwrap());
                *a == Type::Any || *b == Type::Any || Self::is_subtype_of_inner(a, b)
            }
            // DefaultDict explicit arms (DefaultDict→DefaultDict, DefaultDict→Dict)
            (Type::DefaultDict(k1, v1), Type::DefaultDict(k2, v2)) => {
                (**k1 == Type::Any || **k2 == Type::Any || Self::is_subtype_of_inner(k1, k2))
                    && (**v1 == Type::Any || **v2 == Type::Any || Self::is_subtype_of_inner(v1, v2))
            }
            // Dict→Dict and Generic-dict subtyping (excludes DefaultDict to preserve Dict→DefaultDict = false)
            _ if this.dict_kv().is_some()
                && other.dict_kv().is_some()
                && !matches!(this, Type::DefaultDict(..))
                && !matches!(other, Type::DefaultDict(..)) =>
            {
                let ((k1, v1), (k2, v2)) = (this.dict_kv().unwrap(), other.dict_kv().unwrap());
                (*k1 == Type::Any || *k2 == Type::Any || Self::is_subtype_of_inner(k1, k2))
                    && (*v1 == Type::Any || *v2 == Type::Any || Self::is_subtype_of_inner(v1, v2))
            }
            _ if this.tuple_elems().is_some() && other.tuple_elems().is_some() => {
                let (ts1, ts2) = (this.tuple_elems().unwrap(), other.tuple_elems().unwrap());
                ts1.len() == ts2.len()
                    && ts1
                        .iter()
                        .zip(ts2.iter())
                        .all(|(t1, t2)| *t1 == Type::Any || Self::is_subtype_of_inner(t1, t2))
            }
            _ if this.tuple_elems().is_some() && other.tuple_var_elem().is_some() => {
                let (ts, elem) = (this.tuple_elems().unwrap(), other.tuple_var_elem().unwrap());
                ts.iter()
                    .all(|t| *t == Type::Any || Self::is_subtype_of_inner(t, elem))
            }
            _ if this.tuple_var_elem().is_some() && other.tuple_var_elem().is_some() => {
                let (a, b) = (
                    this.tuple_var_elem().unwrap(),
                    other.tuple_var_elem().unwrap(),
                );
                *a == Type::Any || Self::is_subtype_of_inner(a, b)
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
            // Covariant generic subtyping: same base class, pairwise arg subtyping.
            // Any-wildcard: a slot typed `Any` on either side is vacuously compatible.
            (Type::Generic { base: b1, args: a1 }, Type::Generic { base: b2, args: a2 }) => {
                b1 == b2
                    && a1.len() == a2.len()
                    && a1.iter().zip(a2.iter()).all(|(t1, t2)| {
                        *t1 == Type::Any || *t2 == Type::Any || Self::is_subtype_of_inner(t1, t2)
                    })
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
            Type::DefaultDict(k, v) => write!(f, "defaultdict[{}, {}]", k, v),
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
            Type::Generic { base, args } => {
                use crate::builtin_classes::*;
                if *base == BUILTIN_LIST_CLASS_ID {
                    let elem = args.first().map(|t| format!("{}", t)).unwrap_or_default();
                    write!(f, "list[{}]", elem)
                } else if *base == BUILTIN_DICT_CLASS_ID && args.len() == 2 {
                    write!(f, "dict[{}, {}]", args[0], args[1])
                } else if *base == BUILTIN_SET_CLASS_ID {
                    let elem = args.first().map(|t| format!("{}", t)).unwrap_or_default();
                    write!(f, "set[{}]", elem)
                } else if *base == BUILTIN_TUPLE_CLASS_ID {
                    write!(f, "tuple[")?;
                    for (i, t) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", t)?;
                    }
                    write!(f, "]")
                } else if *base == BUILTIN_TUPLE_VAR_CLASS_ID {
                    let elem = args.first().map(|t| format!("{}", t)).unwrap_or_default();
                    write!(f, "tuple[{}, ...]", elem)
                } else {
                    write!(f, "Generic<{}", base.0)?;
                    for t in args {
                        write!(f, ", {}", t)?;
                    }
                    write!(f, ">")
                }
            }
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
        TypeSpec::List(elem) => Type::list_of(typespec_to_type(elem)),
        TypeSpec::Dict(k, v) => Type::dict_of(typespec_to_type(k), typespec_to_type(v)),
        TypeSpec::Tuple(elem) => Type::tuple_of(vec![typespec_to_type(elem)]),
        TypeSpec::Set(elem) => Type::set_of(typespec_to_type(elem)),
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
