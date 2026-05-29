//! Solver environment — a deterministic `TypeKey → Type` map with the
//! monotone JOIN update rule the solver relies on for termination.
//!
//! Termination argument: every update is `env[k] := env[k].join(new)`.
//! `TypeLattice::join` is monotone, so each key only ever moves UP the
//! lattice order. For *primitives* the lattice has finite height
//! (`Never < Bool < Int < Float < Any` ≈ 5 levels) and convergence is
//! immediate. *Nested containers*, however, have UNBOUNDED height —
//! `Never ⊏ list[Never] ⊏ list[list[Never]] ⊏ …` — so a self-referential
//! construction cycle (`for x in stack: stack.append(wrap(x))`) would
//! grow the element type one level per worklist pass and never converge.
//! [`MAX_TYPE_DEPTH`] caps container nesting: any sub-type deeper than the
//! cap is collapsed to `Type::Any` (the lattice top, which absorbs under
//! `join`), restoring finite height and guaranteeing termination. The cap
//! is far deeper than any real program's container nesting, so it never
//! costs precision on well-formed code.

use indexmap::IndexMap;
use pyaot_types::{Type, TypeLattice};

use super::key::TypeKey;

/// Maximum container-nesting depth retained by [`Env::join_into`]. Beyond
/// this, sub-types collapse to `Type::Any` to bound lattice height. Chosen
/// far above realistic nesting (the deepest types observed in the example
/// corpus are ~4 levels, e.g. `list[list[list[T]]]`) so the cap only ever
/// fires on a non-terminating construction cycle, never on real code.
const MAX_TYPE_DEPTH: u32 = 12;

/// Rebuild `ty` with container nesting capped at `remaining` levels.
/// Compound types (`Generic`, `Iterator`, `DefaultDict`, `Function`) each
/// consume one level of `remaining`; when it hits zero the whole sub-type
/// becomes `Type::Any`. `Union` is breadth, not depth, so it recurses at
/// the same level — but folds the capped members back through `join` to
/// preserve the normalized (deduplicated) Union invariant.
fn cap_type_depth(ty: &Type, remaining: u32) -> Type {
    match ty {
        Type::Generic { base, args } => {
            if remaining == 0 {
                return Type::Any;
            }
            Type::Generic {
                base: *base,
                args: args
                    .iter()
                    .map(|a| cap_type_depth(a, remaining - 1))
                    .collect(),
            }
        }
        Type::Iterator(t) => {
            if remaining == 0 {
                return Type::Any;
            }
            Type::Iterator(Box::new(cap_type_depth(t, remaining - 1)))
        }
        Type::DefaultDict(k, v) => {
            if remaining == 0 {
                return Type::Any;
            }
            Type::DefaultDict(
                Box::new(cap_type_depth(k, remaining - 1)),
                Box::new(cap_type_depth(v, remaining - 1)),
            )
        }
        Type::Function { params, ret } => {
            if remaining == 0 {
                return Type::Any;
            }
            Type::Function {
                params: params
                    .iter()
                    .map(|p| cap_type_depth(p, remaining - 1))
                    .collect(),
                ret: Box::new(cap_type_depth(ret, remaining - 1)),
            }
        }
        Type::Union(members) => {
            // Rebuild the union directly rather than folding via `join`, which
            // applies the numeric tower (Int ⊔ Float = Float) and would
            // collapse a runtime-distinguishable union. A union's members sit
            // at the same depth as the union, so `remaining` is not decremented.
            let mut parts: Vec<Type> = Vec::new();
            for m in members {
                let capped = cap_type_depth(m, remaining);
                if !parts.contains(&capped) {
                    parts.push(capped);
                }
            }
            match parts.len() {
                0 => Type::Never,
                1 => parts.into_iter().next().unwrap(),
                _ => Type::Union(parts),
            }
        }
        // Leaves — no nesting to cap.
        _ => ty.clone(),
    }
}

/// Cheap pre-check: does `ty` nest deeper than `MAX_TYPE_DEPTH`? Lets
/// `join_into` skip the rebuild allocation on the overwhelmingly common
/// shallow types.
fn exceeds_max_depth(ty: &Type, remaining: u32) -> bool {
    match ty {
        Type::Generic { args, .. } => {
            remaining == 0 || args.iter().any(|a| exceeds_max_depth(a, remaining - 1))
        }
        Type::Iterator(t) => remaining == 0 || exceeds_max_depth(t, remaining - 1),
        Type::DefaultDict(k, v) => {
            remaining == 0
                || exceeds_max_depth(k, remaining - 1)
                || exceeds_max_depth(v, remaining - 1)
        }
        Type::Function { params, ret } => {
            remaining == 0
                || exceeds_max_depth(ret, remaining - 1)
                || params.iter().any(|p| exceeds_max_depth(p, remaining - 1))
        }
        Type::Union(members) => members.iter().any(|m| exceeds_max_depth(m, remaining)),
        _ => false,
    }
}

/// Solver environment. Wraps an `IndexMap` for deterministic iteration
/// (important because reducers may iterate over `env` keys during
/// materialization).
#[derive(Debug, Default, Clone)]
pub struct Env {
    values: IndexMap<TypeKey, Type>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot reader — returns `Type::bottom()` (`Never`) for unseen keys.
    /// `Never` is the lattice identity for `join`, so unseen keys behave
    /// correctly under all monotone updates.
    pub fn get(&self, key: TypeKey) -> Type {
        self.values.get(&key).cloned().unwrap_or_else(Type::bottom)
    }

    /// Returns `Some(&Type)` only if the key has been explicitly bound.
    /// Distinct from [`Self::get`] which fills unseen keys with `Never`.
    /// Test-only: the solver always uses [`Self::get`]'s bottom-filling form.
    #[cfg(test)]
    pub fn lookup(&self, key: TypeKey) -> Option<&Type> {
        self.values.get(&key)
    }

    /// Monotone update: `env[key] := env[key].join(incoming)`.
    ///
    /// Returns `true` iff the stored value changed. The caller uses this
    /// return value to schedule dependents on the worklist.
    ///
    /// Properties enforced by the unit tests in this file:
    /// - Idempotent: calling twice with the same value never returns
    ///   `true` the second time.
    /// - Monotone: the post-call value is `≥` (in the lattice order) the
    ///   pre-call value.
    /// - Bottom-stable: joining `Type::Never` into any key returns `false`
    ///   (Never is the join identity).
    pub fn join_into(&mut self, key: TypeKey, incoming: Type) -> bool {
        let current = self.values.get(&key).cloned().unwrap_or_else(Type::bottom);
        let joined = current.join(&incoming);
        // Bound lattice height: collapse container nesting beyond
        // MAX_TYPE_DEPTH to `Any` so a self-referential construction cycle
        // converges instead of growing `list[list[…]]` forever. Only
        // rebuilds when the joined type is actually too deep (rare).
        let joined = if exceeds_max_depth(&joined, MAX_TYPE_DEPTH) {
            cap_type_depth(&joined, MAX_TYPE_DEPTH)
        } else {
            joined
        };
        if joined == current {
            return false;
        }
        self.values.insert(key, joined);
        true
    }

    /// Iterate (key, type) pairs in insertion order. Materialization uses
    /// this to walk the env when writing back to `LoweringSeedInfo`.
    pub fn iter(&self) -> impl Iterator<Item = (&TypeKey, &Type)> {
        self.values.iter()
    }
}

#[cfg(test)]
mod tests {
    //! Lattice-property tests for [`Env::join_into`].
    //!
    //! These tests are the foundation of the solver's correctness argument:
    //! if `join_into` is not monotone, the worklist algorithm can loop
    //! forever or produce non-deterministic results. The tests cover the
    //! three properties the solver relies on (monotonicity, idempotency,
    //! bottom-stability) on the primitive corner cases and a representative
    //! container case.

    use super::*;
    use pyaot_hir::ExprId;
    use pyaot_types::Type;

    fn k() -> TypeKey {
        // ExprId is `la_arena::Idx<Expr>`; build a synthetic one via the
        // raw u32 constructor. Solver tests don't rely on the underlying
        // expression existing — only on the key being addressable.
        TypeKey::Expr(ExprId::from_raw(0u32.into()))
    }

    #[test]
    fn unseen_key_reads_bottom() {
        let env = Env::new();
        assert_eq!(env.get(k()), Type::Never);
        assert!(env.lookup(k()).is_none());
    }

    #[test]
    fn join_into_never_is_noop() {
        let mut env = Env::new();
        // First write: bottom join bottom = bottom; no change recorded.
        let changed = env.join_into(k(), Type::Never);
        assert!(!changed);
        assert!(env.lookup(k()).is_none());

        // Second write: bottom join Int = Int; change recorded.
        let changed = env.join_into(k(), Type::Int);
        assert!(changed);

        // Third write: Int join Never = Int; no change.
        let changed = env.join_into(k(), Type::Never);
        assert!(!changed);
        assert_eq!(env.get(k()), Type::Int);
    }

    #[test]
    fn join_into_is_idempotent() {
        let mut env = Env::new();
        assert!(env.join_into(k(), Type::Int));
        // Second identical write: no change.
        assert!(!env.join_into(k(), Type::Int));
        assert_eq!(env.get(k()), Type::Int);
    }

    #[test]
    fn join_into_is_monotone_under_numeric_tower() {
        let mut env = Env::new();
        // Int → Float widens (PEP 3141 numeric tower).
        assert!(env.join_into(k(), Type::Int));
        assert!(env.join_into(k(), Type::Float));
        assert_eq!(env.get(k()), Type::Float);
        // Float → Int does NOT narrow back to Int.
        let changed = env.join_into(k(), Type::Int);
        assert!(!changed);
        assert_eq!(env.get(k()), Type::Float);
    }

    #[test]
    fn join_into_widens_to_union() {
        let mut env = Env::new();
        assert!(env.join_into(k(), Type::Int));
        assert!(env.join_into(k(), Type::Str));
        // Int and Str are not comparable in the lattice; join collapses
        // to a canonical Union[Int, Str].
        match env.get(k()) {
            Type::Union(members) => {
                assert!(members.contains(&Type::Int));
                assert!(members.contains(&Type::Str));
                assert_eq!(members.len(), 2);
            }
            other => panic!("expected Union[Int, Str], got {other:?}"),
        }
    }

    #[test]
    fn join_into_any_absorbs() {
        let mut env = Env::new();
        env.join_into(k(), Type::Int);
        let changed = env.join_into(k(), Type::Any);
        assert!(changed);
        assert_eq!(env.get(k()), Type::Any);
        // Subsequent writes are no-ops — Any is the top.
        assert!(!env.join_into(k(), Type::Float));
        assert!(!env.join_into(k(), Type::Str));
        assert_eq!(env.get(k()), Type::Any);
    }

    #[test]
    fn join_into_container_covariance() {
        use pyaot_types::Type;
        let mut env = Env::new();
        // list[Int] join list[Float] = list[Float] (covariant element-wise).
        assert!(env.join_into(k(), Type::list_of(Type::Int)));
        assert!(env.join_into(k(), Type::list_of(Type::Float)));
        assert_eq!(env.get(k()), Type::list_of(Type::Float));
    }

    #[test]
    fn separate_keys_do_not_alias() {
        use pyaot_hir::ExprId;
        let mut env = Env::new();
        let k0 = TypeKey::Expr(ExprId::from_raw(0u32.into()));
        let k1 = TypeKey::Expr(ExprId::from_raw(1u32.into()));
        env.join_into(k0, Type::Int);
        env.join_into(k1, Type::Str);
        assert_eq!(env.get(k0), Type::Int);
        assert_eq!(env.get(k1), Type::Str);
    }

    #[test]
    fn join_into_caps_runaway_container_nesting() {
        // Models a self-referential construction cycle: each round wraps
        // the prior type in another `list[...]`. Without the depth cap the
        // joined value would grow forever (each round strictly greater in
        // the lattice → `join_into` always returns `true`), hanging the
        // worklist. The cap collapses nesting beyond MAX_TYPE_DEPTH to
        // `Any`, so after a bounded number of rounds the value stabilizes
        // (`Any` is the top → further joins are no-ops) and `join_into`
        // returns `false`.
        let mut env = Env::new();
        let mut ty = Type::Int;
        let mut converged_round = None;
        for round in 0..64 {
            let changed = env.join_into(k(), Type::list_of(ty.clone()));
            ty = env.get(k());
            if !changed {
                converged_round = Some(round);
                break;
            }
        }
        assert!(
            converged_round.is_some(),
            "runaway container nesting must converge under the depth cap"
        );
        // The stabilized value must be a list whose deep element collapsed
        // to `Any` — never an unbounded tower.
        assert!(
            matches!(env.get(k()), Type::Generic { .. }),
            "expected a (depth-capped) list type, got {:?}",
            env.get(k())
        );
    }

    #[test]
    fn meta_key_namespace_distinct_from_expr() {
        let mut env = Env::new();
        let k_expr = TypeKey::Expr(ExprId::from_raw(7u32.into()));
        let k_meta = TypeKey::Meta(7);
        env.join_into(k_expr, Type::Int);
        env.join_into(k_meta, Type::Str);
        assert_eq!(env.get(k_expr), Type::Int);
        assert_eq!(env.get(k_meta), Type::Str);
        assert!(k_meta.is_internal());
        assert!(!k_expr.is_internal());
    }
}
