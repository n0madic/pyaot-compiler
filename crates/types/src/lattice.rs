//! The type lattice for [`SemTy`]: `join` (⊔), `meet` (⊓), `minus` (∖), and the
//! subtype order.
//!
//! Standard bounded-lattice logic over [`SemTy`]: the PEP 3141 numeric tower
//! (`Bool ⊂ Int ⊂ Float`), union normalization (flatten / dedup / drop `Never` /
//! absorb `Dyn` / covariant same-base merge / nominal class merge / subsumed-member
//! removal / canonical sort), tuple-shape merging, and the
//! runtime-distinguishable-union handling in `meet`/`minus` (PITFALLS B6).
//! `defaultdict` arms are TODO pending its representation decision (model as
//! `Generic{DEFAULTDICT_ID,..}` or a runtime object). The canonical-sort secondary
//! key uses `Debug` formatting for a deterministic ordering (a `Display` impl
//! would do equally well).
//!
//! Nominal subtyping is MRO-aware: every lattice operation takes a
//! [`ClassHierarchy`] env (the C3 linearization computed in `semantics`, stored
//! in `hir`'s `ClassTable`). `Class(a) <: Class(b)` iff `b ∈ mro(a)`, and
//! `join(Class(a), Class(b))` is their nearest common MRO ancestor — never an
//! equality-only `Union` collapse, which is the order-sensitivity that seeded
//! the old compiler's class-field-widening cascade.

use crate::builtin_classes;
use crate::sem::{SemTy, Sig};
use pyaot_utils::{ClassId, InternedString};

/// Oracle for the nominal class hierarchy: the C3 MRO computed in `semantics`
/// and stored in `hir`'s `ClassTable`. The lattice *consults* it per call; the
/// data is never duplicated into `types` (`hir` depends on `types`, not vice
/// versa).
pub trait ClassHierarchy {
    /// C3 linearization of `c`, self-first. Empty if `c` is unknown.
    fn mro(&self, c: ClassId) -> &[ClassId];
    /// `c`'s interned class name — for constructing a joined `SemTy::Class`.
    fn class_name(&self, c: ClassId) -> Option<InternedString>;
}

/// The empty hierarchy: every class unrelated. With it the lattice behaves
/// exactly like the pre-MRO version (class-free contexts and unit tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoClasses;

impl ClassHierarchy for NoClasses {
    fn mro(&self, _: ClassId) -> &[ClassId] {
        &[]
    }
    fn class_name(&self, _: ClassId) -> Option<InternedString> {
        None
    }
}

/// Nearest common MRO ancestor: the first element of `mro(a)` appearing in
/// `mro(b)` (C3 monotonicity ⇒ most-derived common ancestor). Under multiple
/// inheritance the two directional scans can disagree (`C(A, B)` vs `D(B, A)`
/// have incomparable minimal ancestors `A` and `B`); returning either would
/// break `join` commutativity, so agreement is required and the caller falls
/// back to the canonical `Union` otherwise.
fn nearest_common_ancestor(a: ClassId, b: ClassId, env: &dyn ClassHierarchy) -> Option<ClassId> {
    let (ma, mb) = (env.mro(a), env.mro(b));
    let ab = ma.iter().find(|c| mb.contains(c))?;
    let ba = mb.iter().find(|c| ma.contains(c))?;
    (ab == ba).then_some(*ab)
}

/// Bounded lattice interface. Every operation consults the nominal class
/// hierarchy through `env` (pass [`NoClasses`] in class-free contexts).
pub trait TypeLattice: Sized + Clone + Eq {
    /// Universal supertype (`Dyn`). `join(top, t) == top`.
    fn top() -> Self;
    /// Universal subtype (`Never`). `join(bot, t) == t`.
    fn bottom() -> Self;
    /// Least upper bound: most specific supertype of both.
    fn join(&self, other: &Self, env: &dyn ClassHierarchy) -> Self;
    /// Greatest lower bound: most specific subtype of both.
    fn meet(&self, other: &Self, env: &dyn ClassHierarchy) -> Self;
    /// Subtype relation: `self ≤ other`.
    fn is_subtype_of(&self, other: &Self, env: &dyn ClassHierarchy) -> bool;
    /// Set difference `self ∖ other` (for `isinstance` else-branch narrowing).
    fn minus(&self, other: &Self, env: &dyn ClassHierarchy) -> Self;
}

// ============================================================================
// Canonical ordering helpers for union normalisation
// ============================================================================

/// Stable sort key so `Union` members have a canonical order regardless of
/// left/right operand position in `join`.
fn type_discriminant(t: &SemTy) -> u32 {
    match t {
        SemTy::Never => 0,
        SemTy::Bool => 1,
        SemTy::Int => 2,
        SemTy::Float => 3,
        SemTy::Str => 4,
        SemTy::Bytes => 5,
        SemTy::NoneTy => 6,
        SemTy::Iterator(_) => 13,
        SemTy::Callable(_) => 14,
        SemTy::Var(_) => 15,
        SemTy::Class { .. } => 16,
        SemTy::BuiltinException(_) => 17,
        SemTy::File { .. } => 18,
        SemTy::RuntimeObject(_) => 19,
        SemTy::NotImplementedT => 20,
        SemTy::Dyn => 21,
        SemTy::Generic { base, .. } => 23 + base.0,
        // A Union never appears as a member of another union after flattening.
        SemTy::Union(_) => u32::MAX,
    }
}

/// PEP 3141 numeric tower: `bool ⊂ int ⊂ float`. Returns the wider of two
/// numeric types, or `None` if either is non-numeric.
fn numeric_promote(a: &SemTy, b: &SemTy) -> Option<SemTy> {
    match (a, b) {
        (SemTy::Float, SemTy::Float | SemTy::Int | SemTy::Bool)
        | (SemTy::Int | SemTy::Bool, SemTy::Float) => Some(SemTy::Float),
        (SemTy::Int, SemTy::Int | SemTy::Bool) | (SemTy::Bool, SemTy::Int) => Some(SemTy::Int),
        (SemTy::Bool, SemTy::Bool) => Some(SemTy::Bool),
        _ => None,
    }
}

/// 1. Flatten nested unions. 2. Dedup. 3. Drop `Never`. 4. Absorb `Dyn`.
/// 5. Merge same-base `Generic` members covariantly.
/// 6. Merge `Class` members to their nearest common MRO ancestor.
/// 7. Remove subsumed members. 8. Sort canonically so `join(a,b) == join(b,a)`.
fn make_canonical_union(
    members: impl IntoIterator<Item = SemTy>,
    env: &dyn ClassHierarchy,
) -> SemTy {
    let mut flat: Vec<SemTy> = Vec::new();
    for m in members {
        match m {
            SemTy::Union(ts) => flat.extend(ts),
            other => flat.push(other),
        }
    }

    flat.retain(|t| *t != SemTy::Never);

    // Absorb Dyn (gradual top).
    if flat.iter().any(|t| matches!(t, SemTy::Dyn)) {
        return SemTy::Dyn;
    }

    // Full dedup.
    let mut deduped: Vec<SemTy> = Vec::with_capacity(flat.len());
    for t in flat {
        if !deduped.contains(&t) {
            deduped.push(t);
        }
    }

    // Merge Generic members with the same base via covariant element-wise join,
    // e.g. Union[list[int], list[float]] → list[float]. Repeat until stable.
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < deduped.len() {
            if let SemTy::Generic { base: b1, args: a1 } = &deduped[i] {
                let base = *b1;
                let arity = a1.len();
                let j = deduped.iter().enumerate().position(|(k, t)| {
                    k != i
                        && matches!(t, SemTy::Generic { base: b2, args: a2 }
                                    if *b2 == base && a2.len() == arity)
                });
                if let Some(j) = j {
                    let hi = j.max(i);
                    let lo = j.min(i);
                    let a = deduped.remove(hi);
                    let b = deduped.remove(lo);
                    let (args_a, args_b) = match (&a, &b) {
                        (SemTy::Generic { args: aa, .. }, SemTy::Generic { args: ab, .. }) => {
                            (aa.clone(), ab.clone())
                        }
                        _ => unreachable!(),
                    };
                    let merged_args: Vec<SemTy> = args_a
                        .iter()
                        .zip(args_b.iter())
                        .map(|(t1, t2)| {
                            if let Some(n) = numeric_promote(t1, t2) {
                                n
                            } else if t1 == t2 {
                                t1.clone()
                            } else {
                                t1.join(t2, env)
                            }
                        })
                        .collect();
                    deduped.push(SemTy::Generic { base, args: merged_args });
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

    // Merge `Class` members pairwise to their nearest common MRO ancestor
    // (unambiguous NCA only — see `nearest_common_ancestor`). Repeat until
    // stable, so a fold over `[Dog, Unrelated, Cat]` collapses Dog+Cat → Animal
    // regardless of fold order. Pairs without a common (or unambiguous)
    // ancestor stay distinct union members, as before.
    loop {
        let mut merged = false;
        'outer: for i in 0..deduped.len() {
            let SemTy::Class { class_id: id1, name: n1 } = deduped[i].clone() else { continue };
            for j in i + 1..deduped.len() {
                let SemTy::Class { class_id: id2, name: n2 } = deduped[j].clone() else {
                    continue;
                };
                let Some(anc) = nearest_common_ancestor(id1, id2, env) else { continue };
                // Reuse an operand's cached name when the ancestor is one of
                // them; otherwise ask the env. No name → skip (conservative).
                let name = if anc == id1 {
                    n1
                } else if anc == id2 {
                    n2
                } else {
                    match env.class_name(anc) {
                        Some(n) => n,
                        None => continue,
                    }
                };
                let joined = SemTy::Class { class_id: anc, name };
                deduped.remove(j);
                deduped.remove(i);
                if !deduped.contains(&joined) {
                    deduped.push(joined);
                }
                merged = true;
                break 'outer;
            }
        }
        if !merged {
            break;
        }
    }

    // Remove subsumed members: keep `m` only if no other member `o` covers it
    // (via subtyping or the numeric tower).
    let to_remove: Vec<usize> = deduped
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let subsumed = deduped.iter().enumerate().any(|(j, o)| {
                j != i && (m.is_subtype_of(o, env) || numeric_promote(m, o).as_ref() == Some(o))
            });
            subsumed.then_some(i)
        })
        .collect();
    for i in to_remove.into_iter().rev() {
        deduped.remove(i);
    }

    deduped.sort_by(|a, b| {
        type_discriminant(a)
            .cmp(&type_discriminant(b))
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
    });

    match deduped.len() {
        0 => SemTy::Never,
        1 => deduped.into_iter().next().expect("len==1"),
        _ => SemTy::Union(deduped),
    }
}

// ============================================================================
// TypeLattice for SemTy
// ============================================================================

impl TypeLattice for SemTy {
    fn top() -> Self {
        SemTy::Dyn
    }

    fn bottom() -> Self {
        SemTy::Never
    }

    fn join(&self, other: &Self, env: &dyn ClassHierarchy) -> Self {
        // top absorbs
        if matches!(self, SemTy::Dyn) || matches!(other, SemTy::Dyn) {
            return SemTy::Dyn;
        }
        // bottom is identity
        if *self == SemTy::Never {
            return other.clone();
        }
        if *other == SemTy::Never {
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
        // Two different callable signatures never merge into one sig (a merged
        // sig would fabricate an indirect-call ABI nothing satisfies) and never
        // form a union (Phase 6): the join is the gradual top, so the slot goes
        // `Tagged` and a later call gets the loud Dyn-callee diagnostic.
        if matches!((self, other), (SemTy::Callable(_), SemTy::Callable(_))) {
            return SemTy::Dyn;
        }
        // Covariant element-wise join for same-base Generic containers.
        if let (SemTy::Generic { base: b1, args: a1 }, SemTy::Generic { base: b2, args: a2 }) =
            (self, other)
        {
            if b1 == b2 && a1.len() == a2.len() {
                return SemTy::Generic {
                    base: *b1,
                    args: a1.iter().zip(a2.iter()).map(|(t1, t2)| t1.join(t2, env)).collect(),
                };
            }
            // tuple[] ⊔ tuple[T..] → TupleVar[T..]: the empty tuple has no shape
            // of its own, so the only meaningful join is the variadic of the
            // non-empty side (lets a field initialised `()` refine to
            // `tuple[Node, ...]`). Different-arity non-empty tuples fall through
            // to canonical Union, preserving distinct shapes for matching.
            if *b1 == builtin_classes::BUILTIN_TUPLE_CLASS_ID
                && *b2 == builtin_classes::BUILTIN_TUPLE_CLASS_ID
                && (a1.is_empty() || a2.is_empty())
            {
                let elem_ty = a1
                    .iter()
                    .chain(a2.iter())
                    .cloned()
                    .fold(SemTy::Never, |acc, t| acc.join(&t, env));
                return SemTy::tuple_var_of(elem_ty);
            }
            // TupleVar[] acts as "untyped variadic".
            if *b1 == builtin_classes::BUILTIN_TUPLE_VAR_CLASS_ID
                && *b2 == builtin_classes::BUILTIN_TUPLE_VAR_CLASS_ID
            {
                return if a1.is_empty() {
                    other.clone()
                } else if a2.is_empty() {
                    self.clone()
                } else {
                    SemTy::Generic { base: *b1, args: vec![a1[0].join(&a2[0], env)] }
                };
            }
        }
        make_canonical_union([self.clone(), other.clone()], env)
    }

    fn meet(&self, other: &Self, env: &dyn ClassHierarchy) -> Self {
        // top is identity for meet
        if matches!(self, SemTy::Dyn) {
            return other.clone();
        }
        if matches!(other, SemTy::Dyn) {
            return self.clone();
        }
        // bottom absorbs
        if *self == SemTy::Never || *other == SemTy::Never {
            return SemTy::Never;
        }
        if self == other {
            return self.clone();
        }
        if self.is_subtype_of(other, env) {
            return self.clone();
        }
        if other.is_subtype_of(self, env) {
            return other.clone();
        }
        // Distribute over unions, building the result directly instead of folding
        // via `join`: `join` applies the numeric tower (Int ⊔ Float = Float),
        // which would collapse a runtime-distinguishable `Union[Int, Float]` to a
        // single primitive — a narrowing pass would then treat it as an unbox hint.
        match (self, other) {
            (SemTy::Union(ts), _) => {
                let mut parts: Vec<SemTy> = Vec::new();
                for t in ts {
                    let m = t.meet(other, env);
                    if m != SemTy::Never && !parts.contains(&m) {
                        parts.push(m);
                    }
                }
                Self::union_from_parts(parts)
            }
            (_, SemTy::Union(ts)) => {
                let mut parts: Vec<SemTy> = Vec::new();
                for t in ts {
                    let m = self.meet(t, env);
                    if m != SemTy::Never && !parts.contains(&m) {
                        parts.push(m);
                    }
                }
                Self::union_from_parts(parts)
            }
            _ => SemTy::Never,
        }
    }

    fn is_subtype_of(&self, other: &Self, env: &dyn ClassHierarchy) -> bool {
        SemTy::is_subtype_of_inner(self, other, env)
    }

    fn minus(&self, other: &Self, env: &dyn ClassHierarchy) -> Self {
        if matches!(self, SemTy::Dyn) {
            return self.clone();
        }
        if *self == SemTy::Never {
            return SemTy::Never;
        }
        if *other == SemTy::Never {
            return self.clone();
        }
        if matches!(other, SemTy::Dyn) {
            return SemTy::Never;
        }
        if self == other || self.is_subtype_of(other, env) {
            return SemTy::Never;
        }
        // Union on right: subtract each member sequentially.
        if let SemTy::Union(excluded) = other {
            return excluded.iter().fold(self.clone(), |acc, m| acc.minus(m, env));
        }
        // Union on left: keep members not subsumed by `other`. Build directly,
        // NOT via `join`: `join`'s numeric tower would collapse a
        // runtime-distinguishable `Union[Int, Float]` to `Float`, which a
        // narrowing pass then misreads as an unbox hint (PITFALLS B6).
        if let SemTy::Union(ts) = self {
            let remaining: Vec<SemTy> = ts
                .iter()
                .map(|t| t.minus(other, env))
                .filter(|t| *t != SemTy::Never)
                .collect();
            return Self::union_from_parts(remaining);
        }
        self.clone()
    }
}

impl SemTy {
    /// Build a `SemTy` from raw union members without the numeric tower: 0 →
    /// `Never`, 1 → the lone member, else `Union`. Used by `meet`/`minus` to
    /// preserve runtime-distinguishable members.
    fn union_from_parts(parts: Vec<SemTy>) -> SemTy {
        match parts.len() {
            0 => SemTy::Never,
            1 => parts.into_iter().next().unwrap(),
            _ => SemTy::Union(parts),
        }
    }

    fn is_subtype_of_inner(this: &SemTy, other: &SemTy, env: &dyn ClassHierarchy) -> bool {
        match (this, other) {
            // Reflexivity
            (a, b) if a == b => true,

            // Never is subtype of everything; nothing is a subtype of Never.
            (SemTy::Never, _) => true,
            (_, SemTy::Never) => false,

            // Dyn is the gradual top.
            (_, SemTy::Dyn) => true,
            (SemTy::Dyn, _) => false,

            // Bool ⊂ Int (isinstance(True, int) == True).
            (SemTy::Bool, SemTy::Int) => true,

            // None ⊂ Optional[T].
            (SemTy::NoneTy, SemTy::Union(set)) if set.contains(&SemTy::NoneTy) => true,

            // Union subtyping.
            (SemTy::Union(left), right) => {
                left.iter().all(|t| Self::is_subtype_of_inner(t, right, env))
            }
            (left, SemTy::Union(right)) => {
                right.iter().any(|t| Self::is_subtype_of_inner(left, t, env))
            }

            // Covariant container subtyping (Dyn-wildcard slots are vacuously OK).
            _ if this.list_elem().is_some() && other.list_elem().is_some() => {
                let (a, b) = (this.list_elem().unwrap(), other.list_elem().unwrap());
                *a == SemTy::Dyn || *b == SemTy::Dyn || Self::is_subtype_of_inner(a, b, env)
            }
            _ if this.set_elem().is_some() && other.set_elem().is_some() => {
                let (a, b) = (this.set_elem().unwrap(), other.set_elem().unwrap());
                *a == SemTy::Dyn || *b == SemTy::Dyn || Self::is_subtype_of_inner(a, b, env)
            }
            _ if this.dict_kv().is_some() && other.dict_kv().is_some() => {
                let ((k1, v1), (k2, v2)) = (this.dict_kv().unwrap(), other.dict_kv().unwrap());
                (*k1 == SemTy::Dyn
                    || *k2 == SemTy::Dyn
                    || Self::is_subtype_of_inner(k1, k2, env))
                    && (*v1 == SemTy::Dyn
                        || *v2 == SemTy::Dyn
                        || Self::is_subtype_of_inner(v1, v2, env))
            }
            _ if this.tuple_elems().is_some() && other.tuple_elems().is_some() => {
                let (ts1, ts2) = (this.tuple_elems().unwrap(), other.tuple_elems().unwrap());
                ts1.len() == ts2.len()
                    && ts1
                        .iter()
                        .zip(ts2.iter())
                        .all(|(t1, t2)| *t1 == SemTy::Dyn || Self::is_subtype_of_inner(t1, t2, env))
            }
            _ if this.tuple_elems().is_some() && other.tuple_var_elem().is_some() => {
                let (ts, elem) = (this.tuple_elems().unwrap(), other.tuple_var_elem().unwrap());
                ts.iter()
                    .all(|t| *t == SemTy::Dyn || Self::is_subtype_of_inner(t, elem, env))
            }
            _ if this.tuple_var_elem().is_some() && other.tuple_var_elem().is_some() => {
                let (a, b) = (this.tuple_var_elem().unwrap(), other.tuple_var_elem().unwrap());
                *a == SemTy::Dyn || Self::is_subtype_of_inner(a, b, env)
            }
            (SemTy::Callable(s1), SemTy::Callable(s2)) => {
                let (
                    Sig { params: p1, ret: r1, varargs: v1, kwargs: k1 },
                    Sig { params: p2, ret: r2, varargs: v2, kwargs: k2 },
                ) = (s1.as_ref(), s2.as_ref());
                v1 == v2
                    && k1 == k2
                    && p1.len() == p2.len()
                    && p2
                        .iter()
                        .zip(p1.iter())
                        .all(|(t2, t1)| Self::is_subtype_of_inner(t2, t1, env)) // contravariant params
                    && Self::is_subtype_of_inner(r1, r2, env) // covariant return
            }
            // Nominal subtyping: `a <: b` iff `b` is in `a`'s C3 MRO.
            (
                SemTy::Class { class_id: id1, .. },
                SemTy::Class { class_id: id2, .. },
            ) => id1 == id2 || env.mro(*id1).contains(id2),
            (SemTy::Iterator(a), SemTy::Iterator(b)) => {
                **a == SemTy::Dyn || Self::is_subtype_of_inner(a, b, env)
            }
            // Covariant generic subtyping: same base, pairwise arg subtyping.
            (SemTy::Generic { base: b1, args: a1 }, SemTy::Generic { base: b2, args: a2 }) => {
                b1 == b2
                    && a1.len() == a2.len()
                    && a1.iter().zip(a2.iter()).all(|(t1, t2)| {
                        *t1 == SemTy::Dyn
                            || *t2 == SemTy::Dyn
                            || Self::is_subtype_of_inner(t1, t2, env)
                    })
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_utils::StringInterner;
    use std::collections::HashMap;

    /// Test hierarchy backed by explicit MRO vectors (what `semantics`' C3
    /// produces) plus interned names.
    struct TestClasses {
        mros: HashMap<ClassId, Vec<ClassId>>,
        names: HashMap<ClassId, InternedString>,
    }

    impl TestClasses {
        fn new(spec: &[(u32, &str, &[u32])], interner: &mut StringInterner) -> Self {
            let mut mros = HashMap::new();
            let mut names = HashMap::new();
            for (id, name, mro) in spec {
                let cid = ClassId::new(*id);
                mros.insert(cid, mro.iter().map(|i| ClassId::new(*i)).collect());
                names.insert(cid, interner.intern(name));
            }
            Self { mros, names }
        }
    }

    impl ClassHierarchy for TestClasses {
        fn mro(&self, c: ClassId) -> &[ClassId] {
            self.mros.get(&c).map(Vec::as_slice).unwrap_or(&[])
        }
        fn class_name(&self, c: ClassId) -> Option<InternedString> {
            self.names.get(&c).copied()
        }
    }

    fn cls(id: u32, env: &TestClasses) -> SemTy {
        let cid = ClassId::new(id);
        SemTy::Class { class_id: cid, name: env.names[&cid] }
    }

    /// Animal(0) ← Dog(1), Cat(2); Unrelated(3); diamond B(4), C(5) ← Animal,
    /// D(6) ← (B, C).
    fn zoo(interner: &mut StringInterner) -> TestClasses {
        TestClasses::new(
            &[
                (0, "Animal", &[0]),
                (1, "Dog", &[1, 0]),
                (2, "Cat", &[2, 0]),
                (3, "Unrelated", &[3]),
                (4, "B", &[4, 0]),
                (5, "C", &[5, 0]),
                (6, "D", &[6, 4, 5, 0]),
            ],
            interner,
        )
    }

    #[test]
    fn sibling_join_is_common_base() {
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (animal, dog, cat) = (cls(0, &env), cls(1, &env), cls(2, &env));
        assert_eq!(dog.join(&cat, &env), animal);
        assert_eq!(cat.join(&dog, &env), animal); // commutativity
    }

    #[test]
    fn derived_join_base_is_base() {
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (animal, dog) = (cls(0, &env), cls(1, &env));
        assert_eq!(dog.join(&animal, &env), animal);
        assert_eq!(animal.join(&dog, &env), animal);
    }

    #[test]
    fn unrelated_join_is_union() {
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (dog, unrelated) = (cls(1, &env), cls(3, &env));
        let joined = dog.join(&unrelated, &env);
        match &joined {
            SemTy::Union(ts) => assert_eq!(ts.len(), 2),
            other => panic!("expected Union, got {other:?}"),
        }
        assert_eq!(joined, unrelated.join(&dog, &env));
        // Under the empty hierarchy even siblings stay a Union (pre-MRO behavior).
        assert!(matches!(dog.join(&cls(2, &env), &NoClasses), SemTy::Union(_)));
    }

    #[test]
    fn diamond_join_and_subtyping() {
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (animal, b, c, d) = (cls(0, &env), cls(4, &env), cls(5, &env), cls(6, &env));
        assert_eq!(b.join(&c, &env), animal);
        // D's nearest ancestor with B is B itself (B ∈ mro(D)).
        assert_eq!(d.join(&b, &env), b);
        assert!(d.is_subtype_of(&animal, &env));
        assert!(d.is_subtype_of(&b, &env));
        assert!(d.is_subtype_of(&c, &env));
        assert!(b.is_subtype_of(&animal, &env));
        assert!(!b.is_subtype_of(&c, &env));
        assert!(!animal.is_subtype_of(&b, &env));
    }

    #[test]
    fn ambiguous_mi_join_falls_back_to_union() {
        // C(A, B) and D(B, A): the directional NCA scans disagree (A vs B), so
        // the join must stay a Union — and be commutative.
        let mut interner = StringInterner::default();
        let env = TestClasses::new(
            &[
                (0, "A", &[0]),
                (1, "B", &[1]),
                (2, "C", &[2, 0, 1]),
                (3, "D", &[3, 1, 0]),
            ],
            &mut interner,
        );
        let (c, d) = (cls(2, &env), cls(3, &env));
        let joined = c.join(&d, &env);
        assert!(matches!(joined, SemTy::Union(_)), "got {joined:?}");
        assert_eq!(joined, d.join(&c, &env));
    }

    #[test]
    fn fold_order_insensitive_class_merge() {
        // A fold over [Dog, Unrelated, Cat] must still collapse Dog+Cat →
        // Animal inside the union, regardless of the interleaving member.
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (animal, dog, cat, unrelated) =
            (cls(0, &env), cls(1, &env), cls(2, &env), cls(3, &env));
        let folded = [dog.clone(), unrelated.clone(), cat.clone()]
            .iter()
            .fold(SemTy::Never, |acc, t| acc.join(t, &env));
        let expected = animal.join(&unrelated, &env);
        assert_eq!(folded, expected);
        // And the reverse order agrees.
        let folded_rev = [cat, unrelated, dog]
            .iter()
            .fold(SemTy::Never, |acc, t| acc.join(t, &env));
        assert_eq!(folded_rev, expected);
    }

    #[test]
    fn union_of_derived_and_base_canonicalizes_to_base() {
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (animal, dog) = (cls(0, &env), cls(1, &env));
        assert_eq!(make_canonical_union([dog, animal.clone()], &env), animal);
    }

    #[test]
    fn covariant_list_join_through_classes() {
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (animal, dog, cat) = (cls(0, &env), cls(1, &env), cls(2, &env));
        assert_eq!(
            SemTy::list_of(dog).join(&SemTy::list_of(cat), &env),
            SemTy::list_of(animal)
        );
    }

    #[test]
    fn meet_and_minus_follow_nominal_subtyping() {
        let mut interner = StringInterner::default();
        let env = zoo(&mut interner);
        let (animal, dog) = (cls(0, &env), cls(1, &env));
        assert_eq!(dog.meet(&animal, &env), dog);
        assert_eq!(animal.meet(&dog, &env), dog);
        assert_eq!(dog.minus(&animal, &env), SemTy::Never);
        assert_eq!(animal.minus(&dog, &env), animal);
    }
}
