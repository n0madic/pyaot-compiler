//! The type lattice for [`SemTy`]: `join` (⊔), `meet` (⊓), `minus` (∖), and the
//! subtype order.
//!
//! Standard bounded-lattice logic over [`SemTy`]: the PEP 3141 numeric tower
//! (`Bool ⊂ Int ⊂ Float`), union normalization (flatten / dedup / drop `Never` /
//! absorb `Dyn` / covariant same-base merge / subsumed-member removal / canonical
//! sort), tuple-shape merging, and the runtime-distinguishable-union handling in
//! `meet`/`minus` (PITFALLS B6). `defaultdict` arms are TODO pending its
//! representation decision (model as `Generic{DEFAULTDICT_ID,..}` or a runtime
//! object). The canonical-sort secondary key uses `Debug` formatting for a
//! deterministic ordering (a `Display` impl would do equally well).

use crate::builtin_classes;
use crate::sem::{SemTy, Sig};

/// Bounded lattice interface. Kept generic, exactly as in the original.
pub trait TypeLattice: Sized + Clone + Eq {
    /// Universal supertype (`Dyn`). `join(top, t) == top`.
    fn top() -> Self;
    /// Universal subtype (`Never`). `join(bot, t) == t`.
    fn bottom() -> Self;
    /// Least upper bound: most specific supertype of both.
    fn join(&self, other: &Self) -> Self;
    /// Greatest lower bound: most specific subtype of both.
    fn meet(&self, other: &Self) -> Self;
    /// Subtype relation: `self ≤ other`.
    fn is_subtype_of(&self, other: &Self) -> bool;
    /// Set difference `self ∖ other` (for `isinstance` else-branch narrowing).
    fn minus(&self, other: &Self) -> Self;
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
/// 5. Merge same-base `Generic` members covariantly. 6. Remove subsumed members.
/// 7. Sort canonically so `join(a,b) == join(b,a)`.
fn make_canonical_union(members: impl IntoIterator<Item = SemTy>) -> SemTy {
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
                                t1.join(t2)
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

    // Remove subsumed members: keep `m` only if no other member `o` covers it
    // (via subtyping or the numeric tower).
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

    fn join(&self, other: &Self) -> Self {
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
        // Covariant element-wise join for same-base Generic containers.
        if let (SemTy::Generic { base: b1, args: a1 }, SemTy::Generic { base: b2, args: a2 }) =
            (self, other)
        {
            if b1 == b2 && a1.len() == a2.len() {
                return SemTy::Generic {
                    base: *b1,
                    args: a1.iter().zip(a2.iter()).map(|(t1, t2)| t1.join(t2)).collect(),
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
                    .fold(SemTy::Never, |acc, t| acc.join(&t));
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
                    SemTy::Generic { base: *b1, args: vec![a1[0].join(&a2[0])] }
                };
            }
        }
        make_canonical_union([self.clone(), other.clone()])
    }

    fn meet(&self, other: &Self) -> Self {
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
        if self.is_subtype_of(other) {
            return self.clone();
        }
        if other.is_subtype_of(self) {
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
                    let m = t.meet(other);
                    if m != SemTy::Never && !parts.contains(&m) {
                        parts.push(m);
                    }
                }
                Self::union_from_parts(parts)
            }
            (_, SemTy::Union(ts)) => {
                let mut parts: Vec<SemTy> = Vec::new();
                for t in ts {
                    let m = self.meet(t);
                    if m != SemTy::Never && !parts.contains(&m) {
                        parts.push(m);
                    }
                }
                Self::union_from_parts(parts)
            }
            _ => SemTy::Never,
        }
    }

    fn is_subtype_of(&self, other: &Self) -> bool {
        SemTy::is_subtype_of_inner(self, other)
    }

    fn minus(&self, other: &Self) -> Self {
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
        if self == other || self.is_subtype_of(other) {
            return SemTy::Never;
        }
        // Union on right: subtract each member sequentially.
        if let SemTy::Union(excluded) = other {
            return excluded.iter().fold(self.clone(), |acc, m| acc.minus(m));
        }
        // Union on left: keep members not subsumed by `other`. Build directly,
        // NOT via `join`: `join`'s numeric tower would collapse a
        // runtime-distinguishable `Union[Int, Float]` to `Float`, which a
        // narrowing pass then misreads as an unbox hint (PITFALLS B6).
        if let SemTy::Union(ts) = self {
            let remaining: Vec<SemTy> = ts
                .iter()
                .map(|t| t.minus(other))
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

    fn is_subtype_of_inner(this: &SemTy, other: &SemTy) -> bool {
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
                left.iter().all(|t| Self::is_subtype_of_inner(t, right))
            }
            (left, SemTy::Union(right)) => {
                right.iter().any(|t| Self::is_subtype_of_inner(left, t))
            }

            // Covariant container subtyping (Dyn-wildcard slots are vacuously OK).
            _ if this.list_elem().is_some() && other.list_elem().is_some() => {
                let (a, b) = (this.list_elem().unwrap(), other.list_elem().unwrap());
                *a == SemTy::Dyn || *b == SemTy::Dyn || Self::is_subtype_of_inner(a, b)
            }
            _ if this.set_elem().is_some() && other.set_elem().is_some() => {
                let (a, b) = (this.set_elem().unwrap(), other.set_elem().unwrap());
                *a == SemTy::Dyn || *b == SemTy::Dyn || Self::is_subtype_of_inner(a, b)
            }
            _ if this.dict_kv().is_some() && other.dict_kv().is_some() => {
                let ((k1, v1), (k2, v2)) = (this.dict_kv().unwrap(), other.dict_kv().unwrap());
                (*k1 == SemTy::Dyn || *k2 == SemTy::Dyn || Self::is_subtype_of_inner(k1, k2))
                    && (*v1 == SemTy::Dyn || *v2 == SemTy::Dyn || Self::is_subtype_of_inner(v1, v2))
            }
            _ if this.tuple_elems().is_some() && other.tuple_elems().is_some() => {
                let (ts1, ts2) = (this.tuple_elems().unwrap(), other.tuple_elems().unwrap());
                ts1.len() == ts2.len()
                    && ts1
                        .iter()
                        .zip(ts2.iter())
                        .all(|(t1, t2)| *t1 == SemTy::Dyn || Self::is_subtype_of_inner(t1, t2))
            }
            _ if this.tuple_elems().is_some() && other.tuple_var_elem().is_some() => {
                let (ts, elem) = (this.tuple_elems().unwrap(), other.tuple_var_elem().unwrap());
                ts.iter()
                    .all(|t| *t == SemTy::Dyn || Self::is_subtype_of_inner(t, elem))
            }
            _ if this.tuple_var_elem().is_some() && other.tuple_var_elem().is_some() => {
                let (a, b) = (this.tuple_var_elem().unwrap(), other.tuple_var_elem().unwrap());
                *a == SemTy::Dyn || Self::is_subtype_of_inner(a, b)
            }
            (SemTy::Callable(s1), SemTy::Callable(s2)) => {
                let (Sig { params: p1, ret: r1 }, Sig { params: p2, ret: r2 }) =
                    (s1.as_ref(), s2.as_ref());
                p1.len() == p2.len()
                    && p2
                        .iter()
                        .zip(p1.iter())
                        .all(|(t2, t1)| Self::is_subtype_of_inner(t2, t1)) // contravariant params
                    && Self::is_subtype_of_inner(r1, r2) // covariant return
            }
            (
                SemTy::Class { class_id: id1, .. },
                SemTy::Class { class_id: id2, .. },
            ) => id1 == id2, // TODO: consult MRO once nominal subtyping lands
            (SemTy::Iterator(a), SemTy::Iterator(b)) => {
                **a == SemTy::Dyn || Self::is_subtype_of_inner(a, b)
            }
            // Covariant generic subtyping: same base, pairwise arg subtyping.
            (SemTy::Generic { base: b1, args: a1 }, SemTy::Generic { base: b2, args: a2 }) => {
                b1 == b2
                    && a1.len() == a2.len()
                    && a1.iter().zip(a2.iter()).all(|(t1, t2)| {
                        *t1 == SemTy::Dyn || *t2 == SemTy::Dyn || Self::is_subtype_of_inner(t1, t2)
                    })
            }
            _ => false,
        }
    }
}
