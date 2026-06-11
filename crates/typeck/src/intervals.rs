//! # Raw-int loop specialization as a `typeck` proof (Phase 3c, Tier B)
//!
//! A **standalone, terminal, forward integer-interval abstract interpretation**
//! that runs *after* the SemTy worklist has converged and `materialize` has
//! written final types onto every local/expr, and *after* `check_repr_boundaries`
//! (see [`super::infer`]). It reads finalized types + the HIR CFG and writes only
//! two boolean eligibility flags — [`pyaot_hir::HirLocal::raw_int_ok`] and
//! [`pyaot_hir::HirExpr::raw_int_ok`]. It never feeds back into SemTy inference.
//!
//! This is **not** a PITFALLS-A3 violation: A3 forbids splitting *type
//! derivation* into mutually-recursive sub-passes. This pass answers a strictly
//! different question ("does this provably-`int` value stay within a magnitude
//! bound with no i64 overflow?") that the SemTy lattice never computes — the same
//! category as the terminal `check_repr_boundaries`. A wrong result can only flip
//! a slot already typed `Int` between `Raw(I64)` and `Tagged`; it cannot change
//! any `SemTy` or any ABI.
//!
//! ## Domain
//!
//! A saturating signed interval over `i128`. Endpoints may carry the [`NEG_INF`]
//! / [`POS_INF`] sentinels that *widening* produces; an interval is **eligible**
//! (→ `Raw(I64)`-able) iff it is a `Range` fully inside `[-BOUND, BOUND]` where
//! `BOUND = RAW_I64_NARROW_BOUND` (`= 2^48`, the single source of truth in
//! `types`). Arithmetic transfers are computed in `i128` and clamped to `⊤` the
//! instant any endpoint leaves `[-BOUND, BOUND]` — over-approximation is always
//! toward `⊤` (PITFALLS A2), never a finite range narrower than reality.
//!
//! ## Soundness obligations honoured
//!
//! * A local is flagged only if **every** writer's interval is eligible (the
//!   `raw_uniform` "stay Tagged when in doubt" discipline) AND it is not a
//!   parameter (a param int's entry value comes from an unbounded caller).
//! * A `BinOp` is flagged only if its result is eligible AND each operand is
//!   itself flagged-raw, a fixnum `IntLit` within `±BOUND`, or a `raw_int_ok`
//!   local — the bottom-up closure invariant lowering relies on (PITFALLS B16).
//! * Bitwise / shift / `Pow` / true `Div` and possibly-bignum operands → `⊤`.
//! * The `2^48 < 2^60` fixnum ceiling guarantees a re-tag (`Raw(I64) → Tagged`)
//!   of any flagged value is an immediate fixnum (the runtime demotes any
//!   in-range integer result to a fixnum), so the round-trip is the identity.

use std::collections::HashMap;

use la_arena::Idx;

use pyaot_hir::{
    BinOp, CmpOp, HirBlock, HirExpr, HirExprKind, HirFunction, HirModule, HirStmt, HirTerminator,
    ResolveResult, Symbol, SymbolRef, UnaryOp,
};
use pyaot_types::SemTy;
use pyaot_utils::LocalId;

use crate::WIDEN_LIMIT;

/// The conservative magnitude bound, in `i128`. A `Range` inside `[-BOUND, BOUND]`
/// cannot promote to a heap `BigInt` and leaves headroom so a raw `Add`/`Sub`/
/// `Mul` of two such values never overflows i64 and its result is still a valid
/// tagged fixnum.
const BOUND: i128 = pyaot_types::RAW_I64_NARROW_BOUND as i128;

/// `-∞` / `+∞` sentinels for endpoints produced by widening. They are far outside
/// `[-BOUND, BOUND]`, so any interval carrying one is ineligible until narrowing
/// recovers a finite endpoint; and `as_bounded` rejects them, so arithmetic on a
/// widened interval collapses to `⊤` rather than computing on a sentinel.
const NEG_INF: i128 = i128::MIN;
const POS_INF: i128 = i128::MAX;

/// A saturating signed integer interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interval {
    /// The empty set (a dead edge, or "not yet evaluated").
    Bottom,
    /// `[lo, hi]` with `lo <= hi`; either endpoint may be a `±∞` sentinel.
    Range { lo: i128, hi: i128 },
    /// `[-∞, +∞]` — unknown.
    Top,
}

impl Interval {
    /// Canonicalizing constructor: empties to `Bottom`, the fully-unbounded range
    /// to `Top`, everything else a `Range` (sentinel endpoints allowed).
    fn range(lo: i128, hi: i128) -> Interval {
        if lo > hi {
            Interval::Bottom
        } else if lo == NEG_INF && hi == POS_INF {
            Interval::Top
        } else {
            Interval::Range { lo, hi }
        }
    }

    /// An arithmetic result: `⊤` the instant an endpoint leaves `[-BOUND, BOUND]`
    /// (the overflow / bignum-promotion guard), else the canonical `Range`.
    fn range_clamped(lo: i128, hi: i128) -> Interval {
        if lo < -BOUND || hi > BOUND {
            Interval::Top
        } else {
            Interval::range(lo, hi)
        }
    }

    /// `Some((lo, hi))` iff this is a `Range` fully inside `[-BOUND, BOUND]`.
    fn as_bounded(self) -> Option<(i128, i128)> {
        match self {
            Interval::Range { lo, hi } if lo >= -BOUND && hi <= BOUND => Some((lo, hi)),
            _ => None,
        }
    }

    /// Eligible to back a `Raw(I64)` slot/expr: a provably-in-bound finite range.
    fn eligible(self) -> bool {
        self.as_bounded().is_some()
    }

    /// Convex hull (`⊥` identity, `⊤` absorbing).
    fn join(self, other: Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, x) | (x, Interval::Bottom) => x,
            (Interval::Top, _) | (_, Interval::Top) => Interval::Top,
            (Interval::Range { lo: al, hi: ah }, Interval::Range { lo: bl, hi: bh }) => {
                Interval::range(al.min(bl), ah.max(bh))
            }
        }
    }

    /// Intersection (`⊥` absorbing, `⊤` identity); an empty meet is `⊥` (a dead
    /// edge).
    fn meet(self, other: Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            (Interval::Top, x) | (x, Interval::Top) => x,
            (Interval::Range { lo: al, hi: ah }, Interval::Range { lo: bl, hi: bh }) => {
                Interval::range(al.max(bl), ah.min(bh))
            }
        }
    }

    /// Widen `self` toward `next` (assumed `⊒ self`): any endpoint that moved
    /// outward jumps to the corresponding `±∞` sentinel, pinning the ascending
    /// chain so the fixpoint terminates.
    fn widen(self, next: Interval) -> Interval {
        match (self, next) {
            (Interval::Bottom, x) => x,
            (x, Interval::Bottom) => x,
            (Interval::Top, _) | (_, Interval::Top) => Interval::Top,
            (Interval::Range { lo: ol, hi: oh }, Interval::Range { lo: nl, hi: nh }) => {
                let lo = if nl < ol { NEG_INF } else { ol };
                let hi = if nh > oh { POS_INF } else { oh };
                Interval::range(lo, hi)
            }
        }
    }

    /// Narrow `self` using `next` (the classic widen-then-narrow recovery): only
    /// an infinite endpoint may be replaced by `next`'s finite one, so the
    /// descending chain is bounded and the result stays a sound post-fixpoint.
    fn narrow(self, next: Interval) -> Interval {
        match (self, next) {
            (Interval::Range { lo: ol, hi: oh }, Interval::Range { lo: nl, hi: nh }) => {
                let lo = if ol == NEG_INF { nl } else { ol };
                let hi = if oh == POS_INF { nh } else { oh };
                Interval::range(lo, hi)
            }
            // Recover a widened `⊤` (e.g. `[-∞,+∞]`) toward a finite candidate.
            (Interval::Top, x @ Interval::Range { .. }) => x,
            (this, _) => this,
        }
    }

    fn negate(self) -> Interval {
        match self.as_bounded() {
            Some((lo, hi)) => Interval::range_clamped(-hi, -lo),
            None => Interval::Top,
        }
    }

    fn add(self, other: Interval) -> Interval {
        match (self.as_bounded(), other.as_bounded()) {
            (Some((al, ah)), Some((bl, bh))) => Interval::range_clamped(al + bl, ah + bh),
            _ => Interval::Top,
        }
    }

    fn sub(self, other: Interval) -> Interval {
        match (self.as_bounded(), other.as_bounded()) {
            (Some((al, ah)), Some((bl, bh))) => Interval::range_clamped(al - bh, ah - bl),
            _ => Interval::Top,
        }
    }

    fn mul(self, other: Interval) -> Interval {
        match (self.as_bounded(), other.as_bounded()) {
            (Some((al, ah)), Some((bl, bh))) => {
                // Products of |endpoints| <= 2^48 stay <= 2^96, well inside i128.
                let p = [al * bl, al * bh, ah * bl, ah * bh];
                let lo = *p.iter().min().unwrap();
                let hi = *p.iter().max().unwrap();
                Interval::range_clamped(lo, hi)
            }
            _ => Interval::Top,
        }
    }

    /// Python `%` (sign of divisor). Only a provably-positive divisor narrows: the
    /// result of `x % d` with `d ∈ [rl, rh]`, `rl ≥ 1`, is `[0, rh-1]` regardless
    /// of the dividend. A possibly-zero / possibly-negative divisor → `⊤` (so the
    /// op stays tagged and the runtime raises `ZeroDivisionError` correctly).
    fn modulo(self, divisor: Interval) -> Interval {
        match divisor {
            Interval::Range { lo: rl, hi: rh } if rl >= 1 && rh <= BOUND => {
                Interval::range_clamped(0, rh - 1)
            }
            _ => Interval::Top,
        }
    }

    /// Python `//` (floor). A bounded dividend by a provably-positive divisor:
    /// floor-div is coordinate-wise monotone, so the extremes are at the four
    /// endpoint corners. Otherwise `⊤`.
    fn floordiv(self, divisor: Interval) -> Interval {
        let (nl, nh) = match self.as_bounded() {
            Some(b) => b,
            None => return Interval::Top,
        };
        match divisor {
            Interval::Range { lo: rl, hi: rh } if rl >= 1 && rh <= BOUND => {
                // `div_euclid` == floor division for a positive divisor.
                let c = [
                    nl.div_euclid(rl),
                    nl.div_euclid(rh),
                    nh.div_euclid(rl),
                    nh.div_euclid(rh),
                ];
                let lo = *c.iter().min().unwrap();
                let hi = *c.iter().max().unwrap();
                Interval::range_clamped(lo, hi)
            }
            _ => Interval::Top,
        }
    }
}

/// A per-program-point abstract state: every local's interval (`⊤` = unknown).
type Env = Vec<Interval>;

/// An interned-expr → interval scratch/record map.
type ExprIv = HashMap<Idx<HirExpr>, Interval>;

/// **Public entry.** Per function: run the interval analysis to a fixpoint, then
/// set the eligibility flags under the `ty == Int` + eligible + operand-closure
/// gates. Infallible (any imprecision rides the always-sound tagged baseline).
pub(crate) fn narrow_raw_ints(module: &mut HirModule, resolve: &ResolveResult) {
    for func in &mut module.functions {
        if let Some((expr_iv, writers)) = analyze_func(func, resolve) {
            apply_flags(func, resolve, &expr_iv, &writers);
        }
    }
}

/// Negate a comparison (the `else` edge of a branch).
fn negate_cmp(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::GtE,
        CmpOp::LtE => CmpOp::Gt,
        CmpOp::Gt => CmpOp::LtE,
        CmpOp::GtE => CmpOp::Lt,
        CmpOp::Eq => CmpOp::NotEq,
        CmpOp::NotEq => CmpOp::Eq,
    }
}

/// Mirror a comparison when its operands are swapped (`a < b` ⇔ `b > a`).
fn mirror_cmp(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::Gt,
        CmpOp::LtE => CmpOp::GtE,
        CmpOp::Gt => CmpOp::Lt,
        CmpOp::GtE => CmpOp::LtE,
        CmpOp::Eq => CmpOp::Eq,
        CmpOp::NotEq => CmpOp::NotEq,
    }
}

/// The local an expression reads, if it is a direct local / resolved-name read of
/// an `int` slot (the only shape branch refinement and the operand-closure rule
/// can narrow).
fn int_local(func: &HirFunction, resolve: &ResolveResult, idx: Idx<HirExpr>) -> Option<LocalId> {
    let lid = match &func.exprs[idx].kind {
        HirExprKind::Local(lid) => *lid,
        HirExprKind::Name(SymbolRef::Resolved(sid)) => match resolve.symbol(*sid) {
            Symbol::Local(lid) => lid,
            _ => return None,
        },
        _ => return None,
    };
    (func.locals[lid.index()].ty == SemTy::Int).then_some(lid)
}

/// Combine an integer binary operator's operand intervals.
fn binop_interval(op: BinOp, lv: Interval, rv: Interval) -> Interval {
    match op {
        BinOp::Add => lv.add(rv),
        BinOp::Sub => lv.sub(rv),
        BinOp::Mul => lv.mul(rv),
        BinOp::Mod => lv.modulo(rv),
        BinOp::FloorDiv => lv.floordiv(rv),
        // True `/` is float; `**` and bitwise/shift are bignum-possible.
        BinOp::Div
        | BinOp::Pow
        | BinOp::BitAnd
        | BinOp::BitOr
        | BinOp::BitXor
        | BinOp::Shl
        | BinOp::Shr => Interval::Top,
    }
}

/// A leaf / unary / binary integer interval in `env` (the read-only evaluator the
/// dataflow uses). Recurses only into arithmetic operands — sufficient because a
/// local's value is an arithmetic expression or a leaf; a `BinOp` buried in a
/// call argument never influences any local's interval, so the analysis need not
/// descend into one (the `record_all` apply walk does, to flag it). Non-integer /
/// unanalyzable shapes are `⊤`.
fn eval(func: &HirFunction, resolve: &ResolveResult, env: &Env, idx: Idx<HirExpr>) -> Interval {
    match &func.exprs[idx].kind {
        HirExprKind::IntLit(v) => Interval::range(*v as i128, *v as i128),
        // A bignum literal does not fit i64 by construction; a bool is `0`/`1`.
        HirExprKind::BigIntLit(_) => Interval::Top,
        HirExprKind::BoolLit(b) => {
            let v = *b as i128;
            Interval::range(v, v)
        }
        HirExprKind::Local(lid) => {
            if func.locals[lid.index()].ty == SemTy::Int {
                env[lid.index()]
            } else {
                Interval::Top
            }
        }
        HirExprKind::Name(SymbolRef::Resolved(sid)) => match resolve.symbol(*sid) {
            Symbol::Local(lid) if func.locals[lid.index()].ty == SemTy::Int => env[lid.index()],
            _ => Interval::Top,
        },
        HirExprKind::Unary { op, operand } => {
            let ov = eval(func, resolve, env, *operand);
            match op {
                UnaryOp::Neg => ov.negate(),
                UnaryOp::Pos => ov,
                // `~x` / `not x` route through the tagged baseline; conservative.
                UnaryOp::Invert | UnaryOp::Not => Interval::Top,
            }
        }
        HirExprKind::BinOp { op, l, r } => {
            let lv = eval(func, resolve, env, *l);
            let rv = eval(func, resolve, env, *r);
            binop_interval(*op, lv, rv)
        }
        // Parameters, calls, globals, cells, container/heap reads, … → ⊤.
        _ => Interval::Top,
    }
}

/// The comprehensive recording walk (apply phase only): compute `idx`'s interval
/// **and** descend into *every* sub-expression so that a flaggable `BinOp` buried
/// in a call argument / subscript / container literal (`xs.append(i*3 % k)`) gets
/// its interval recorded. Arithmetic nodes carry a real interval; every other
/// node is `⊤` but still recursed for its children.
fn record_all(
    func: &HirFunction,
    resolve: &ResolveResult,
    env: &Env,
    idx: Idx<HirExpr>,
    rec: &mut ExprIv,
) -> Interval {
    let child = |c: Idx<HirExpr>, rec: &mut ExprIv| {
        record_all(func, resolve, env, c, rec);
    };
    let iv = match &func.exprs[idx].kind {
        HirExprKind::IntLit(v) => Interval::range(*v as i128, *v as i128),
        HirExprKind::BigIntLit(_) => Interval::Top,
        HirExprKind::BoolLit(b) => {
            let v = *b as i128;
            Interval::range(v, v)
        }
        HirExprKind::Local(lid) => {
            if func.locals[lid.index()].ty == SemTy::Int {
                env[lid.index()]
            } else {
                Interval::Top
            }
        }
        HirExprKind::Name(SymbolRef::Resolved(sid)) => match resolve.symbol(*sid) {
            Symbol::Local(lid) if func.locals[lid.index()].ty == SemTy::Int => env[lid.index()],
            _ => Interval::Top,
        },
        HirExprKind::Unary { op, operand } => {
            let ov = record_all(func, resolve, env, *operand, rec);
            match op {
                UnaryOp::Neg => ov.negate(),
                UnaryOp::Pos => ov,
                UnaryOp::Invert | UnaryOp::Not => Interval::Top,
            }
        }
        HirExprKind::BinOp { op, l, r } => {
            let lv = record_all(func, resolve, env, *l, rec);
            let rv = record_all(func, resolve, env, *r, rec);
            binop_interval(*op, lv, rv)
        }
        // ── compound non-integer nodes: recurse all children, value is ⊤ ──
        HirExprKind::Compare { l, r, .. } => {
            child(*l, rec);
            child(*r, rec);
            Interval::Top
        }
        HirExprKind::Call { callee, args } => {
            child(*callee, rec);
            for a in args {
                child(*a, rec);
            }
            Interval::Top
        }
        HirExprKind::MethodCall { recv, args, .. } => {
            child(*recv, rec);
            for a in args {
                child(*a, rec);
            }
            Interval::Top
        }
        HirExprKind::ContainerExpr { args, .. } => {
            for a in args {
                child(*a, rec);
            }
            Interval::Top
        }
        HirExprKind::Subscript { base, index } => {
            child(*base, rec);
            child(*index, rec);
            Interval::Top
        }
        HirExprKind::Slice { base, start, end, step } => {
            child(*base, rec);
            for o in [start, end, step].into_iter().flatten() {
                child(*o, rec);
            }
            Interval::Top
        }
        HirExprKind::ListLit { elems }
        | HirExprKind::TupleLit { elems }
        | HirExprKind::SetLit { elems } => {
            for e in elems {
                child(*e, rec);
            }
            Interval::Top
        }
        HirExprKind::DictLit { pairs } => {
            for (k, v) in pairs {
                child(*k, rec);
                child(*v, rec);
            }
            Interval::Top
        }
        HirExprKind::FormatValue { value, .. }
        | HirExprKind::Attribute { value, .. }
        | HirExprKind::IsInstance { value, .. }
        | HirExprKind::IsInstanceBuiltin { value, .. }
        | HirExprKind::IsNone { value }
        | HirExprKind::ExcInstanceStr { value } => {
            child(*value, rec);
            Interval::Top
        }
        HirExprKind::Sum { iterable, start } => {
            child(*iterable, rec);
            if let Some(s) = start {
                child(*s, rec);
            }
            Interval::Top
        }
        HirExprKind::CallRuntime { args, .. } => {
            for a in args.iter().flatten() {
                child(*a, rec);
            }
            Interval::Top
        }
        HirExprKind::GenericConstruct { args, .. } => {
            for a in args {
                child(*a, rec);
            }
            Interval::Top
        }
        HirExprKind::MakeClosure { captures, .. } => {
            for c in captures {
                child(*c, rec);
            }
            Interval::Top
        }
        HirExprKind::MakeCell { init } => {
            if let Some(i) = init {
                child(*i, rec);
            }
            Interval::Top
        }
        HirExprKind::GenQuery { gen, value, .. } => {
            child(*gen, rec);
            if let Some(v) = value {
                child(*v, rec);
            }
            Interval::Top
        }
        // ── leaves with no expression children ──
        _ => Interval::Top,
    };
    rec.insert(idx, iv);
    iv
}

/// Apply a block's statements to `env` (strong updates on `int` local writes),
/// recording expr intervals into `rec`.
fn transfer_block(func: &HirFunction, resolve: &ResolveResult, block: &HirBlock, env: &mut Env) {
    for stmt in &block.stmts {
        if let HirStmt::Assign { target, value } = stmt {
            let iv = eval(func, resolve, env, *value);
            // Only `int` slots carry an interval; others stay `⊤` (their reads
            // already evaluate to `⊤`).
            if func.locals[target.index()].ty == SemTy::Int {
                env[target.index()] = iv;
            }
        }
    }
}

/// Refine `local` against a comparison `local <op> bound`.
fn refine_local(cur: Interval, op: CmpOp, bound: Interval) -> Interval {
    // The finite endpoints of the bound (an infinite endpoint imposes no
    // constraint, so the refinement is skipped on that side).
    let (blo, bhi) = match bound {
        Interval::Range { lo, hi } => (lo, hi),
        Interval::Top => (NEG_INF, POS_INF),
        Interval::Bottom => return cur,
    };
    let upper = |cur: Interval, h: i128| {
        if h == POS_INF {
            cur
        } else {
            cur.meet(Interval::range(NEG_INF, h))
        }
    };
    let lower = |cur: Interval, l: i128| {
        if l == NEG_INF {
            cur
        } else {
            cur.meet(Interval::range(l, POS_INF))
        }
    };
    match op {
        CmpOp::Lt => upper(cur, bhi.saturating_sub(1)),
        CmpOp::LtE => upper(cur, bhi),
        CmpOp::Gt => lower(cur, blo.saturating_add(1)),
        CmpOp::GtE => lower(cur, blo),
        CmpOp::Eq => cur.meet(bound),
        CmpOp::NotEq => cur,
    }
}

/// Apply branch refinement for the `taken` edge of `Branch { cond }` to a copy of
/// the block's out-env. Returns `None` if the edge is infeasible (a `⊥` local).
fn refine_edge(
    func: &HirFunction,
    resolve: &ResolveResult,
    out_env: &Env,
    cond: Idx<HirExpr>,
    taken: bool,
) -> Option<Env> {
    let mut env = out_env.clone();
    if let HirExprKind::Compare { op, l, r } = &func.exprs[cond].kind {
        let op = if taken { *op } else { negate_cmp(*op) };
        if let Some(lid) = int_local(func, resolve, *l) {
            let rv = eval(func, resolve, out_env, *r);
            let cur = env[lid.index()];
            env[lid.index()] = refine_local(cur, op, rv);
        } else if let Some(rid) = int_local(func, resolve, *r) {
            let lv = eval(func, resolve, out_env, *l);
            let cur = env[rid.index()];
            env[rid.index()] = refine_local(cur, mirror_cmp(op), lv);
        }
    }
    // A `⊥` in any slot means this edge is statically dead — drop it.
    if env.contains(&Interval::Bottom) {
        None
    } else {
        Some(env)
    }
}

/// A CFG edge label: `Some((cond, taken))` for a branch arm (carrying the
/// condition expr and whether this is the taken side), or `None` for an
/// unconditional edge.
type Edge = Option<(Idx<HirExpr>, bool)>;

/// The successors of a block as `(dense_index, edge)`.
fn successors(
    term: &HirTerminator,
    index_of: &HashMap<Idx<HirBlock>, usize>,
) -> Vec<(usize, Edge)> {
    match term {
        HirTerminator::Return(_) | HirTerminator::Unreachable => Vec::new(),
        HirTerminator::Jump(b) => vec![(index_of[b], None)],
        HirTerminator::Branch { cond, then, else_ } => vec![
            (index_of[then], Some((*cond, true))),
            (index_of[else_], Some((*cond, false))),
        ],
        // Handler entry carries the pre-`try` state unrefined (conservative).
        HirTerminator::TryEnter { normal, handler } => {
            vec![(index_of[normal], None), (index_of[handler], None)]
        }
    }
}

/// The set of **loop-head** block indices — the targets of a back-edge (a DFS
/// edge to a node still on the recursion stack). Widening is applied ONLY at
/// these heads: widening every block would let a non-head's join discard a
/// loop-guard cap (e.g. a body cursor capped at `stop-1` would be widened to
/// `+∞`, the increment would then overflow to `⊤`, and the head would never
/// recover its bound). Widening every cycle's single head still cuts every cycle,
/// so the fixpoint terminates while non-head blocks keep their refined precision.
fn loop_heads(succ_of: &[Vec<usize>], entry: usize) -> std::collections::HashSet<usize> {
    let n = succ_of.len();
    // color: 0 = white (unseen), 1 = gray (on stack), 2 = black (done).
    let mut color = vec![0u8; n];
    let mut heads = std::collections::HashSet::new();
    let mut stack: Vec<(usize, usize)> = vec![(entry, 0)];
    color[entry] = 1;
    while let Some(&(node, ci)) = stack.last() {
        if ci < succ_of[node].len() {
            stack.last_mut().unwrap().1 += 1;
            let s = succ_of[node][ci];
            match color[s] {
                1 => {
                    heads.insert(s); // edge to a gray node → back-edge target
                }
                0 => {
                    color[s] = 1;
                    stack.push((s, 0));
                }
                _ => {}
            }
        } else {
            color[node] = 2;
            stack.pop();
        }
    }
    heads
}

/// Run the forward interval analysis to a (widened, then narrowed) fixpoint.
/// Returns `(expr_iv, writers)` — the converged per-expr intervals and, per
/// local, the intervals of every value written into it — or `None` if the
/// widening loop fails to converge within its generous cap (bail → flag nothing).
fn analyze_func(
    func: &HirFunction,
    resolve: &ResolveResult,
) -> Option<(ExprIv, HashMap<usize, Vec<Interval>>)> {
    let n_locals = func.locals.len();
    let order: Vec<Idx<HirBlock>> = func.blocks.iter().map(|(i, _)| i).collect();
    let n = order.len();
    if n == 0 {
        return None;
    }
    let index_of: HashMap<Idx<HirBlock>, usize> =
        order.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let entry = index_of[&func.entry];
    let succ_of: Vec<Vec<usize>> = order
        .iter()
        .map(|&b| successors(&func.blocks[b].term, &index_of).into_iter().map(|(s, _)| s).collect())
        .collect();
    let heads = loop_heads(&succ_of, entry);

    let top_env = || vec![Interval::Top; n_locals];
    let mut in_env: Vec<Option<Env>> = vec![None; n];
    in_env[entry] = Some(top_env());
    let mut visit = vec![0usize; n];

    // Recompute every block's candidate in-env by pushing each reached block's
    // refined out-env to its successors (entry is pinned to all-⊤).
    let recompute = |in_env: &[Option<Env>]| -> Vec<Option<Env>> {
        let mut cand: Vec<Option<Env>> = vec![None; n];
        cand[entry] = Some(top_env());
        for bi in 0..n {
            let Some(env_b) = &in_env[bi] else { continue };
            let mut out = env_b.clone();
            transfer_block(func, resolve, &func.blocks[order[bi]], &mut out);
            for (succ, edge) in successors(&func.blocks[order[bi]].term, &index_of) {
                let refined = match edge {
                    None => Some(out.clone()),
                    Some((cond, taken)) => refine_edge(func, resolve, &out, cond, taken),
                };
                if let Some(r) = refined {
                    if succ == entry {
                        continue; // entry stays all-⊤
                    }
                    cand[succ] = Some(match cand[succ].take() {
                        None => r,
                        Some(acc) => join_env(&acc, &r),
                    });
                }
            }
        }
        cand
    };

    // ── Phase 1: widening to a post-fixpoint. ──
    // Each (block, local) endpoint climbs through at most `WIDEN_LIMIT` finite
    // values before widening pins it to a `±∞` sentinel, so the ascending chain
    // stabilizes; the cap bounds the pathological case and bails conservatively.
    let max_rounds = n.saturating_mul(n_locals.saturating_add(1)).saturating_mul(WIDEN_LIMIT + 2);
    let max_rounds = max_rounds.clamp(64, 200_000);
    let mut converged = false;
    for _ in 0..max_rounds {
        let cand = recompute(&in_env);
        let mut changed = false;
        for bi in 0..n {
            if bi == entry {
                continue;
            }
            let new_in = match (&in_env[bi], &cand[bi]) {
                (cur, None) => cur.clone(),
                (None, Some(c)) => Some(c.clone()),
                (Some(old), Some(c)) => {
                    let joined = join_env(old, c);
                    let merged = if heads.contains(&bi) && visit[bi] >= WIDEN_LIMIT {
                        widen_env(old, &joined)
                    } else {
                        joined
                    };
                    Some(merged)
                }
            };
            if new_in != in_env[bi] {
                in_env[bi] = new_in;
                visit[bi] += 1;
                changed = true;
            }
        }
        if !changed {
            converged = true;
            break;
        }
    }
    if !converged {
        return None;
    }

    // ── Phase 2: narrowing to recover loop-guard bounds. ──
    for _ in 0..(WIDEN_LIMIT + 2) {
        let cand = recompute(&in_env);
        let mut changed = false;
        for bi in 0..n {
            if bi == entry {
                continue;
            }
            if let (Some(old), Some(c)) = (&in_env[bi], &cand[bi]) {
                let narrowed = narrow_env(old, c);
                if &narrowed != old {
                    in_env[bi] = Some(narrowed);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // ── Final apply walk: record converged expr intervals + writer intervals. ──
    let mut expr_iv: ExprIv = HashMap::new();
    let mut writers: HashMap<usize, Vec<Interval>> = HashMap::new();
    for bi in 0..n {
        let Some(env0) = &in_env[bi] else { continue };
        let mut env = env0.clone();
        let block = &func.blocks[order[bi]];
        for stmt in &block.stmts {
            record_stmt_exprs(func, resolve, &env, stmt, &mut expr_iv);
            if let HirStmt::Assign { target, value } = stmt {
                let iv = *expr_iv.get(value).unwrap_or(&Interval::Top);
                if func.locals[target.index()].ty == SemTy::Int {
                    writers.entry(target.index()).or_default().push(iv);
                    env[target.index()] = iv;
                }
            }
        }
        record_term_exprs(func, resolve, &env, &block.term, &mut expr_iv);
    }
    Some((expr_iv, writers))
}

/// Record (into `rec`) the intervals of every expression rooted in a statement.
fn record_stmt_exprs(
    func: &HirFunction,
    resolve: &ResolveResult,
    env: &Env,
    stmt: &HirStmt,
    rec: &mut ExprIv,
) {
    let e = |idx: Idx<HirExpr>, rec: &mut ExprIv| {
        record_all(func, resolve, env, idx, rec);
    };
    match stmt {
        HirStmt::Expr(idx) => e(*idx, rec),
        HirStmt::Assign { value, .. } => e(*value, rec),
        HirStmt::Assert { cond } => e(*cond, rec),
        HirStmt::Print { args, .. } => {
            for a in args {
                e(*a, rec);
            }
        }
        HirStmt::SetItem { base, index, value } => {
            e(*base, rec);
            e(*index, rec);
            e(*value, rec);
        }
        HirStmt::SetAttr { base, value, .. } => {
            e(*base, rec);
            e(*value, rec);
        }
        HirStmt::ContainerPush { value, .. } => e(*value, rec),
        HirStmt::ContainerInsert { key, value, .. } => {
            e(*key, rec);
            e(*value, rec);
        }
        HirStmt::CellSet { value, .. } => e(*value, rec),
        HirStmt::GlobalSet { value, .. } => e(*value, rec),
        HirStmt::GenSetLocal { gen, value, .. } => {
            e(*gen, rec);
            e(*value, rec);
        }
        HirStmt::GenSetState { gen, .. } | HirStmt::GenSetExhausted { gen } => e(*gen, rec),
        // `Raise` / `ExcOp` operands never hold a hot raw-int BinOp; leaving them
        // unrecorded just keeps them tagged (sound).
        HirStmt::Raise(_) | HirStmt::ExcOp(_) => {}
    }
}

/// Record the intervals of a terminator's expressions (the branch condition / a
/// returned value).
fn record_term_exprs(
    func: &HirFunction,
    resolve: &ResolveResult,
    env: &Env,
    term: &HirTerminator,
    rec: &mut ExprIv,
) {
    match term {
        HirTerminator::Return(Some(v)) => {
            record_all(func, resolve, env, *v, rec);
        }
        HirTerminator::Branch { cond, .. } => {
            record_all(func, resolve, env, *cond, rec);
        }
        _ => {}
    }
}

/// Set the eligibility flags from the converged analysis.
fn apply_flags(
    func: &mut HirFunction,
    resolve: &ResolveResult,
    expr_iv: &ExprIv,
    writers: &HashMap<usize, Vec<Interval>>,
) {
    let n_params = func.params.len();
    // A local is `Raw(I64)`-eligible iff it is a non-parameter `int` slot with at
    // least one writer and every writer's interval is in-bound.
    let local_eligible: Vec<bool> = (0..func.locals.len())
        .map(|lid| {
            func.locals[lid].ty == SemTy::Int
                && lid >= n_params
                && writers
                    .get(&lid)
                    .is_some_and(|ws| !ws.is_empty() && ws.iter().all(|iv| iv.eligible()))
        })
        .collect();

    // Bottom-up: flag a `BinOp` iff its (int) result is eligible AND each operand
    // is itself flagged-raw, an in-bound fixnum literal, or a `raw_int_ok` local.
    let mut memo: HashMap<Idx<HirExpr>, bool> = HashMap::new();
    let mut flagged: Vec<Idx<HirExpr>> = Vec::new();
    let exprs: Vec<Idx<HirExpr>> = func.exprs.iter().map(|(i, _)| i).collect();
    for idx in exprs {
        rawable(func, resolve, expr_iv, &local_eligible, &mut memo, &mut flagged, idx);
    }

    for (lid, ok) in local_eligible.iter().enumerate() {
        if *ok {
            func.locals[lid].raw_int_ok = true;
        }
    }
    for idx in flagged {
        func.exprs[idx].raw_int_ok = true;
    }
}

/// Whether `idx` may be supplied to lowering as a `Raw(I64)` operand, recording
/// any flagged `BinOp` it visits. Memoized; the expr graph is a tree, so there
/// are no cycles.
fn rawable(
    func: &HirFunction,
    resolve: &ResolveResult,
    expr_iv: &ExprIv,
    local_eligible: &[bool],
    memo: &mut HashMap<Idx<HirExpr>, bool>,
    flagged: &mut Vec<Idx<HirExpr>>,
    idx: Idx<HirExpr>,
) -> bool {
    if let Some(&r) = memo.get(&idx) {
        return r;
    }
    let r = match &func.exprs[idx].kind {
        HirExprKind::IntLit(v) => (-BOUND..=BOUND).contains(&(*v as i128)),
        HirExprKind::Local(lid) => local_eligible.get(lid.index()).copied().unwrap_or(false),
        HirExprKind::Name(SymbolRef::Resolved(sid)) => match resolve.symbol(*sid) {
            Symbol::Local(lid) => local_eligible.get(lid.index()).copied().unwrap_or(false),
            _ => false,
        },
        HirExprKind::BinOp { op, l, r } => {
            // Recurse first (bottom-up): operands are decided before the parent.
            let lr = rawable(func, resolve, expr_iv, local_eligible, memo, flagged, *l);
            let rr = rawable(func, resolve, expr_iv, local_eligible, memo, flagged, *r);
            let res_ok = func.exprs[idx].ty == SemTy::Int
                && matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Mod | BinOp::FloorDiv
                )
                && expr_iv.get(&idx).copied().unwrap_or(Interval::Top).eligible()
                && lr
                && rr;
            if res_ok {
                flagged.push(idx);
            }
            res_ok
        }
        _ => false,
    };
    memo.insert(idx, r);
    r
}

fn join_env(a: &Env, b: &Env) -> Env {
    a.iter().zip(b).map(|(&x, &y)| x.join(y)).collect()
}

fn widen_env(old: &Env, next: &Env) -> Env {
    old.iter().zip(next).map(|(&x, &y)| x.widen(y)).collect()
}

fn narrow_env(old: &Env, next: &Env) -> Env {
    old.iter().zip(next).map(|(&x, &y)| x.narrow(y)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(lo: i128, hi: i128) -> Interval {
        Interval::range(lo, hi)
    }

    #[test]
    fn join_is_convex_hull() {
        assert_eq!(r(0, 5).join(r(3, 8)), r(0, 8));
        assert_eq!(Interval::Bottom.join(r(1, 2)), r(1, 2));
        assert_eq!(Interval::Top.join(r(1, 2)), Interval::Top);
    }

    #[test]
    fn meet_is_intersection_with_dead_edge() {
        assert_eq!(r(0, 5).meet(r(3, 8)), r(3, 5));
        // Disjoint → ⊥ (a dead edge).
        assert_eq!(r(0, 2).meet(r(5, 9)), Interval::Bottom);
        assert_eq!(Interval::Top.meet(r(1, 2)), r(1, 2));
    }

    #[test]
    fn mul_clamps_to_top_out_of_bound() {
        // `i*3` with i ∈ [0, 999999] stays a precise range.
        assert_eq!(r(0, 999_999).mul(r(3, 3)), r(0, 2_999_997));
        // `i*i` with i ∈ [0, 2^48] overflows the bound → ⊤.
        assert_eq!(r(0, BOUND).mul(r(0, BOUND)), Interval::Top);
    }

    #[test]
    fn modulo_depends_only_on_positive_divisor() {
        assert_eq!(r(0, 2_999_997).modulo(r(1009, 1009)), r(0, 1008));
        // Even an unbounded dividend yields `[0, d-1]`.
        assert_eq!(Interval::Top.modulo(r(7, 7)), r(0, 6));
        // A possibly-zero divisor stays unknown (→ tagged → runtime ZeroDiv).
        assert_eq!(r(0, 10).modulo(r(0, 5)), Interval::Top);
    }

    #[test]
    fn floordiv_floors_toward_negative_infinity() {
        // (-7) // 2 == -4 ; 7 // 2 == 3.
        assert_eq!(r(-7, -7).floordiv(r(2, 2)), r(-4, -4));
        assert_eq!(r(7, 7).floordiv(r(2, 2)), r(3, 3));
        // A negative divisor is not narrowed (stays tagged).
        assert_eq!(r(7, 7).floordiv(r(-2, -2)), Interval::Top);
    }

    #[test]
    fn widen_then_narrow_recovers_loop_bound() {
        // Model the `range(1000000)` header join: a stable lower bound 0 with an
        // upper bound that keeps climbing must widen to +∞ then narrow back.
        let widened = r(0, 16).widen(r(0, 17));
        assert_eq!(widened, Interval::Range { lo: 0, hi: POS_INF });
        // Narrowing against the loop-guard-capped candidate recovers the finite hi.
        let narrowed = widened.narrow(r(0, 1_000_000));
        assert_eq!(narrowed, r(0, 1_000_000));
        // The body cursor (meet with `< stop`) is then in-bound and eligible.
        let body = narrowed.meet(r(NEG_INF, 999_999));
        assert_eq!(body, r(0, 999_999));
        assert!(body.eligible());
    }

    #[test]
    fn widening_terminates_on_unbounded_climb() {
        // An accumulator with no guard climbs forever; widening pins it to ⊤.
        let mut iv = Interval::Bottom;
        for k in 0..(WIDEN_LIMIT as i128 + 4) {
            let next = iv.join(r(0, k));
            iv = if k as usize >= WIDEN_LIMIT { iv.widen(next) } else { next };
        }
        assert_eq!(iv, Interval::Range { lo: 0, hi: POS_INF });
        assert!(!iv.eligible());
    }

    #[test]
    fn collatz_style_value_stays_top() {
        // `n = 3*n + 1` from an unbounded start never narrows.
        let n = Interval::Top;
        let next = n.mul(r(3, 3)).add(r(1, 1));
        assert_eq!(next, Interval::Top);
    }

    #[test]
    fn negative_literal_interval() {
        assert_eq!(r(-7, -7).negate(), r(7, 7));
        assert!(r(-7, -7).eligible());
    }
}
