//! # typeck — one constraint-based type inference
//!
//! Type inference is ONE algorithm in three phases — never a fixpoint of
//! mutually recursive monotone passes (PITFALLS A3 / Principle 5):
//!
//! 1. **collect** — a walk over the HIR builds the per-local assignment table
//!    and records which locals are *authoritative* (carry a frontend annotation,
//!    so their type drives `Repr` and inference must not touch them).
//! 2. **solve** — a single monotone worklist iterates the expr / local types to a
//!    lattice fixpoint. Local↔expr dependencies are cyclic across loop back-edges
//!    (`acc = acc + 1.5`); the scalar lattice has finite height, so the monotone
//!    iteration converges. This is ONE worklist, not a re-run of passes.
//! 3. **materialize** — write the solved [`SemTy`] back onto each HIR expr **and**
//!    each inferred [`pyaot_hir::HirLocal`], so `repr_of` can pick `Raw(F64)` for
//!    float locals / `Raw(I8)` for bool locals. Authoritative (annotated) locals
//!    keep their declared type.
//!
//! Inference finishes BEFORE lowering and does not leak into it. Representation is
//! decided by `repr_of` at the lowering boundary. Because the tagged baseline is
//! always correct, inference precision is a performance lever, not a correctness
//! requirement (Principle 2): a node left `Dyn` (→ `Tagged` → `rt_*` dispatch)
//! still compiles correctly, just to slower code.
//!
//! ## Soundness of local-repr narrowing (the one trap here)
//!
//! A local has exactly one flow-insensitive `Repr` slot, so it gets exactly one
//! inferred `SemTy`. The numeric tower makes `join(Int, Float) = Float`, but a
//! single slot inferred `Float` (→ `Raw(F64)`) cannot soundly also hold a tagged
//! `int`: unboxing a tagged int as an f64 is a silent miscompile (PITFALLS A2).
//! So when the joined type would take a `Raw` representation, we additionally
//! require every assigned value to *already* have that representation; otherwise
//! the local falls back to `Dyn` (→ `Tagged`). This is the "stay Tagged when in
//! doubt" rule — it never fabricates a collapsed `Float` that a later pass would
//! treat as an unbox hint (PITFALLS B6).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use la_arena::Idx;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    BinOp, BuiltinFunctionKind, ContainerMethod, ContainerOp, HirExpr, HirExprKind, HirFunction,
    HirModule, HirStmt, HirTerminator, ResolveResult, Symbol, SymbolRef, UnaryOp,
};
use pyaot_types::{repr_of, RawKind, Repr, SemTy, TypeLattice};

/// Run inference over every function, mutating each node's [`SemTy`] in place.
///
/// Per-function inference: a callee's return type is read from its (annotated or
/// `Dyn`) signature — return-type inference across functions is not in scope.
pub fn infer(module: &mut HirModule, resolve: &ResolveResult) -> Result<()> {
    // Snapshot each function's declared return type so `Call` results can be typed
    // without holding a second borrow of `module` while we mutate a function.
    let ret_tys: Vec<SemTy> = module.functions.iter().map(|f| f.ret_ty.clone()).collect();
    for func in module.functions.iter_mut() {
        let solution = {
            let solver = Solver::collect(&*func, resolve, &ret_tys);
            solver.solve()
        };
        materialize(func, &solution);
    }
    // Types are now materialized on every node; validate the unboxed-slot
    // boundaries before lowering can emit an unsound coercion.
    check_repr_boundaries(module, resolve)?;
    Ok(())
}

/// How a slot's representation *reinterprets a tagged value by its assumed type*
/// when a value is coerced into it — the family of coercions a contract violation
/// can turn into a crash. Every such coercion must be guarded here (the discipline
/// PITFALLS A2 / Phase 3 established for `Raw`, extended to `Heap` in Phase 4).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReinterpretKind {
    /// `UnboxFloat`/`UntagBool` (`Raw(F64)`/`Raw(I8)`): reads the assumed-typed bits
    /// *immediately* — a fixnum read as an f64 SIGSEGVs at the unbox itself. So even
    /// a gradual `Dyn` value is unsafe: rejected unless a proven subtype.
    Strict,
    /// `TaggedToHeap` (`Heap(_)`): re-types a tagged value as a heap pointer of the
    /// assumed shape. Bit-identical, so a wrong value does not misread immediately —
    /// it crashes *later* at a container op (CPython would `TypeError` there). A
    /// concrete non-matching type (`int` into a `list[int]` slot) is still rejected
    /// loudly; a gradual `Dyn` value is admitted (a future runtime guard, exactly as
    /// uniform-tagged iteration elements legitimately produce `Dyn → Heap` bindings).
    Gradual,
}

/// The reinterpret family of a slot's representation, or `None` if storing into it
/// is always sound (`Tagged`, the proof-gated `Raw(I64)`, function pointers).
fn reinterpret_kind(ty: &SemTy) -> Option<ReinterpretKind> {
    match repr_of(ty) {
        Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I8) => Some(ReinterpretKind::Strict),
        Repr::Heap(_) => Some(ReinterpretKind::Gradual),
        _ => None,
    }
}

/// Reject a value whose static type cannot be soundly stored in a typed slot whose
/// representation reinterprets by assumed type (an annotated `float`/`bool` →
/// `Raw`, or a `list`/`dict`/`set`/`str`/… → typed `Heap`).
///
/// In CPython a type annotation is not enforced — `poly(3)` for `def poly(a:
/// float)` just runs with `a == 3`. This compiler, however, lowers annotated slots
/// to a representation that *reinterprets the bits by the annotated type*, so a
/// mismatched value would be misread (PITFALLS A2) — a SIGSEGV for the `Raw`
/// unbox, a deferred container-op crash for the `Heap` re-type. Rather than
/// accept-then-crash, we treat the annotation as a contract and reject the
/// violation loudly. (A future whole-program pass could instead demote such a slot
/// to `Tagged` when a call site proves it polymorphic — PITFALLS B10, deferred.)
fn check_repr_boundaries(module: &HirModule, resolve: &ResolveResult) -> Result<()> {
    for func in &module.functions {
        for (_b, block) in func.blocks.iter() {
            // Assignments into an annotated unboxed / typed-heap local slot.
            for stmt in &block.stmts {
                if let HirStmt::Assign { target, value } = stmt {
                    let target_ty = &func.locals[target.index()].ty;
                    if let Some(kind) = reinterpret_kind(target_ty) {
                        check_reinterpret(&func.exprs[*value], target_ty, kind, "assigned to")?;
                    }
                }
            }
            // Return value into an annotated return slot.
            if let HirTerminator::Return(Some(v)) = &block.term {
                if let Some(kind) = reinterpret_kind(&func.ret_ty) {
                    check_reinterpret(&func.exprs[*v], &func.ret_ty, kind, "returned from")?;
                }
            }
        }
        // Call arguments into annotated parameters.
        for (_idx, expr) in func.exprs.iter() {
            let HirExprKind::Call { callee, args } = &expr.kind else { continue };
            let HirExprKind::Name(SymbolRef::Resolved(id)) = func.exprs[*callee].kind else {
                continue;
            };
            let Symbol::Function(fid) = resolve.symbol(id) else { continue };
            let callee_fn = &module.functions[fid.index()];
            for (arg, param) in args.iter().zip(&callee_fn.params) {
                if let Some(kind) = reinterpret_kind(&param.ty) {
                    check_reinterpret(&func.exprs[*arg], &param.ty, kind, "passed to")?;
                }
            }
        }
    }
    Ok(())
}

/// Error unless `value`'s type may be soundly stored in a `target`-typed
/// reinterpret slot. `Never` (unreachable) is always accepted; a [`ReinterpretKind::Gradual`]
/// slot additionally accepts `Dyn` (gradual typing, deferred to a runtime guard).
fn check_reinterpret(
    value: &HirExpr,
    target: &SemTy,
    kind: ReinterpretKind,
    verb: &str,
) -> Result<()> {
    let ok = value.ty == SemTy::Never
        || value.ty.is_subtype_of(target)
        || (kind == ReinterpretKind::Gradual && value.ty == SemTy::Dyn);
    if ok {
        return Ok(());
    }
    let detail = match kind {
        ReinterpretKind::Strict =>
            "this compiler unboxes annotated `float`/`bool` slots, so a mismatched \
             value would be misread. Pass a matching type, e.g. `3.0` instead of `3`.",
        ReinterpretKind::Gradual =>
            "this compiler stores annotated container/`str`/`bytes` slots as typed \
             heap pointers, so a mismatched value would be reinterpreted as one and \
             crash at the first operation on it. Pass a matching type.",
    };
    Err(CompilerError::type_error(
        format!(
            "a value of type `{}` cannot be {verb} a `{}` slot: a type annotation is \
             a contract here, not a coercion ({detail})",
            type_name(&value.ty),
            type_name(target),
        ),
        value.span,
    ))
}

/// A short Python-facing name for a `SemTy` (best-effort, for diagnostics).
fn type_name(ty: &SemTy) -> &'static str {
    match ty {
        SemTy::Int => "int",
        SemTy::Float => "float",
        SemTy::Bool => "bool",
        SemTy::Str => "str",
        SemTy::Bytes => "bytes",
        SemTy::NoneTy => "None",
        SemTy::Dyn => "Any",
        SemTy::Iterator(_) => "iterator",
        _ if ty.list_elem().is_some() => "list",
        _ if ty.dict_kv().is_some() => "dict",
        _ if ty.set_elem().is_some() => "set",
        _ if ty.tuple_elems().is_some() || ty.tuple_var_elem().is_some() => "tuple",
        _ => "<other>",
    }
}

/// The solved types: one per HIR expr node and one per local slot.
struct Solution {
    expr_ty: HashMap<Idx<HirExpr>, SemTy>,
    local_ty: Vec<SemTy>,
}

/// Per-function worklist solver over the [`TypeLattice`].
struct Solver<'a> {
    func: &'a HirFunction,
    resolve: &'a ResolveResult,
    ret_tys: &'a [SemTy],
    /// Current per-expr type (absent = `Never`, the lattice bottom).
    expr_ty: HashMap<Idx<HirExpr>, SemTy>,
    /// Current per-local type.
    local_ty: Vec<SemTy>,
    /// `true` for locals whose frontend type is authoritative (a parameter or an
    /// explicit annotation): their type is fixed and never inferred.
    authoritative: Vec<bool>,
    /// Value expressions assigned to each local, indexed by `LocalId`.
    assignments: Vec<Vec<Idx<HirExpr>>>,
}

impl<'a> Solver<'a> {
    /// **collect** — seed the assignment table and the authoritative-local set.
    fn collect(func: &'a HirFunction, resolve: &'a ResolveResult, ret_tys: &'a [SemTy]) -> Self {
        let n = func.locals.len();
        // A frontend type other than `Dyn` is authoritative: it comes from a
        // parameter annotation, a `name: T` annotation, or a synthetic local the
        // frontend deliberately typed (e.g. `__name__: str`, chained-compare
        // results). Plain `x = ...` locals are `Dyn` and get inferred.
        let authoritative: Vec<bool> =
            func.locals.iter().map(|l| l.ty != SemTy::Dyn).collect();
        let local_ty: Vec<SemTy> = func
            .locals
            .iter()
            .enumerate()
            .map(|(i, l)| if authoritative[i] { l.ty.clone() } else { SemTy::Never })
            .collect();

        let mut assignments: Vec<Vec<Idx<HirExpr>>> = vec![Vec::new(); n];
        for (_bidx, block) in func.blocks.iter() {
            for stmt in &block.stmts {
                if let HirStmt::Assign { target, value } = stmt {
                    assignments[target.index()].push(*value);
                }
            }
        }

        Solver { func, resolve, ret_tys, expr_ty: HashMap::new(), local_ty, authoritative, assignments }
    }

    /// **solve** — iterate the monotone worklist to a fixpoint, then write back.
    fn solve(mut self) -> Solution {
        // Gauss-Seidel sweeps: recompute every expr type, then every inferred
        // local type, until a full sweep changes nothing. Every recomputation is
        // monotone-increasing in the lattice and `Dyn` is an absorbing top, so the
        // iteration terminates.
        loop {
            let mut changed = false;
            let expr_indices: Vec<Idx<HirExpr>> = self.func.exprs.iter().map(|(i, _)| i).collect();
            for idx in &expr_indices {
                let new = self.eval_expr(*idx);
                if self.expr_ty.get(idx) != Some(&new) {
                    self.expr_ty.insert(*idx, new);
                    changed = true;
                }
            }
            for i in 0..self.local_ty.len() {
                if self.authoritative[i] {
                    continue;
                }
                let new = self.recompute_local(i);
                if self.local_ty[i] != new {
                    self.local_ty[i] = new;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Solution { expr_ty: self.expr_ty, local_ty: self.local_ty }
    }

    /// The current type of an expr (bottom = `Never` if not yet computed).
    fn ety(&self, idx: Idx<HirExpr>) -> SemTy {
        self.expr_ty.get(&idx).cloned().unwrap_or(SemTy::Never)
    }

    /// Recompute one inferred local's type from its assigned values, applying the
    /// `Raw`-repr soundness guard (see the module docs).
    fn recompute_local(&self, i: usize) -> SemTy {
        let mut joined = SemTy::Never;
        for &v in &self.assignments[i] {
            joined = joined.join(&self.ety(v));
        }
        // `Never` is the in-progress bottom: a local still being computed (or one
        // only fed by not-yet-evaluated values across a loop back-edge) must stay
        // `Never`, never jump to a spurious `Dyn`. `join` treats `Never` as the
        // identity, so a bottom contributor is correctly ignored by dependents;
        // an injected `Dyn` would instead absorb and poison them irreversibly.
        // Genuinely-unconstrained locals are mapped to `Dyn` once, in materialize.
        if joined == SemTy::Never {
            return SemTy::Never;
        }
        // A `Raw` slot is only sound if every assigned value already has that
        // representation — otherwise a numerically-promoted contributor (a tagged
        // int feeding a `Float` slot) would be silently unboxed (PITFALLS A2/B6).
        if matches!(repr_of(&joined), Repr::Raw(_)) {
            let target = repr_of(&joined);
            // A still-`Never` contributor (not yet evaluated this sweep) is skipped
            // — it adds nothing to the join, so it must not spuriously block the
            // narrowing and force a sticky `Dyn`.
            let uniform = self.assignments[i].iter().all(|&v| {
                let t = self.ety(v);
                t == SemTy::Never || repr_of(&t) == target
            });
            if !uniform {
                return SemTy::Dyn;
            }
        }
        joined
    }

    /// The type of an expr node from its kind and its operands' current types.
    fn eval_expr(&self, idx: Idx<HirExpr>) -> SemTy {
        match &self.func.exprs[idx].kind {
            HirExprKind::StrLit(_) => SemTy::Str,
            HirExprKind::IntLit(_) | HirExprKind::BigIntLit(_) => SemTy::Int,
            HirExprKind::FloatLit(_) => SemTy::Float,
            HirExprKind::BoolLit(_) => SemTy::Bool,
            HirExprKind::NoneLit => SemTy::NoneTy,
            HirExprKind::Compare { .. } => SemTy::Bool,
            HirExprKind::Local(lid) => self.local_ty[lid.index()].clone(),
            HirExprKind::Name(symref) => self.name_ty(*symref),
            HirExprKind::Unary { op, operand } => self.unary_ty(*op, self.ety(*operand)),
            HirExprKind::BinOp { op, l, r } => self.binop_ty(*op, self.ety(*l), self.ety(*r)),
            HirExprKind::Call { callee, args } => self.call_ty(*callee, args),
            // ── containers (Phase 4) ──
            HirExprKind::ListLit { elems } => SemTy::list_of(self.join_all(elems)),
            HirExprKind::SetLit { elems } => SemTy::set_of(self.join_all(elems)),
            HirExprKind::TupleLit { elems } => {
                SemTy::tuple_of(elems.iter().map(|e| self.ety(*e)).collect())
            }
            HirExprKind::DictLit { pairs } => {
                let k = pairs.iter().fold(SemTy::Never, |acc, (k, _)| acc.join(&self.ety(*k)));
                let v = pairs.iter().fold(SemTy::Never, |acc, (_, v)| acc.join(&self.ety(*v)));
                SemTy::dict_of(k, v)
            }
            HirExprKind::BytesLit(_) => SemTy::Bytes,
            HirExprKind::Subscript { base, index } => self.subscript_ty(*base, *index),
            HirExprKind::ContainerExpr { op, args } => self.container_op_ty(*op, args),
            HirExprKind::MethodCall { recv, method, .. } => {
                method_ty(&self.ety(*recv), *method)
            }
        }
    }

    /// Join the types of every expr in `elems` (the lattice bottom for empty).
    fn join_all(&self, elems: &[Idx<HirExpr>]) -> SemTy {
        elems.iter().fold(SemTy::Never, |acc, e| acc.join(&self.ety(*e)))
    }

    /// Result type of a subscript read `base[index]`, from the base's container
    /// shape. A fixed-tuple indexed by an integer literal yields that slot's type.
    fn subscript_ty(&self, base: Idx<HirExpr>, index: Idx<HirExpr>) -> SemTy {
        let bt = self.ety(base);
        if let Some(elem) = bt.list_elem() {
            return elem.clone();
        }
        if let Some((_, v)) = bt.dict_kv() {
            return v.clone();
        }
        if let Some(elems) = bt.tuple_elems() {
            // A literal index selects the exact slot; otherwise join all slots.
            if let HirExprKind::IntLit(i) = self.func.exprs[index].kind {
                let n = elems.len() as i64;
                let idx = if i < 0 { n + i } else { i };
                if idx >= 0 && (idx as usize) < elems.len() {
                    return elems[idx as usize].clone();
                }
            }
            return elems.iter().fold(SemTy::Never, |acc, t| acc.join(t));
        }
        if let Some(e) = bt.tuple_var_elem() {
            return e.clone();
        }
        match bt {
            SemTy::Str => SemTy::Str,
            SemTy::Bytes => SemTy::Int,
            SemTy::Never => SemTy::Never,
            _ => SemTy::Dyn,
        }
    }

    /// Result type of a container / iterator op (the `ContainerExpr` and
    /// `Symbol::Container` paths share this).
    fn container_op_ty(&self, op: ContainerOp, args: &[Idx<HirExpr>]) -> SemTy {
        use ContainerOp as C;
        let arg0 = || args.first().map(|a| self.ety(*a)).unwrap_or(SemTy::Dyn);
        match op {
            C::Len => SemTy::Int,
            C::Contains | C::ListCmp(_) | C::TupleCmp(_) | C::IterExhausted => SemTy::Bool,
            C::Iter => SemTy::Iterator(Box::new(iter_elem_ty(&arg0()))),
            C::IterNext => match arg0() {
                SemTy::Iterator(elem) => *elem,
                // `Never` is the in-progress bottom (the iterator's type is not yet
                // solved this sweep) — stay `Never`, never jump to a spurious `Dyn`
                // that would absorb and poison the consuming accumulator (PITFALLS
                // A2/B6, the same early-Dyn trap the worklist guards against).
                SemTy::Never => SemTy::Never,
                _ => SemTy::Dyn,
            },
            // ── iteration builtins (the arg is the *iterable*; lowering wraps it) ──
            C::Enumerate => SemTy::Iterator(Box::new(SemTy::tuple_of(vec![
                SemTy::Int,
                iter_elem_ty(&arg0()),
            ]))),
            C::Zip => {
                let a = iter_elem_ty(&arg0());
                let b = args.get(1).map(|x| iter_elem_ty(&self.ety(*x))).unwrap_or(SemTy::Dyn);
                SemTy::Iterator(Box::new(SemTy::tuple_of(vec![a, b])))
            }
            C::ListFromIter => SemTy::list_of(iter_elem_ty(&arg0())),
            C::TupleFromIter => SemTy::tuple_var_of(iter_elem_ty(&arg0())),
            C::DictFromPairs => match iter_elem_ty(&arg0()).tuple_elems() {
                // `dict([(k, v), …])` — the element is a 2-tuple of (key, value).
                Some(kv) if kv.len() == 2 => SemTy::dict_of(kv[0].clone(), kv[1].clone()),
                _ => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
            },
            C::BytesFromList => SemTy::Bytes,
            C::Sorted => SemTy::list_of(iter_elem_ty(&arg0())),
            C::Reversed => SemTy::Iterator(Box::new(iter_elem_ty(&arg0()))),
            // Remaining ops are emitted only by lowering (literals / subscript /
            // operators), never typed through this path.
            _ => SemTy::Dyn,
        }
    }

    /// The type of a resolved name used as a value (only locals carry one here).
    fn name_ty(&self, symref: SymbolRef) -> SemTy {
        if let SymbolRef::Resolved(id) = symref {
            if let Symbol::Local(lid) = self.resolve.symbol(id) {
                return self.local_ty[lid.index()].clone();
            }
        }
        SemTy::Dyn
    }

    /// Result type of a unary operator.
    fn unary_ty(&self, op: UnaryOp, operand: SemTy) -> SemTy {
        match op {
            UnaryOp::Not => SemTy::Bool,
            // `~x` is integer-valued for int-like operands; `bool`/`int` → `int`.
            UnaryOp::Invert => {
                if is_int_like(&operand) {
                    SemTy::Int
                } else {
                    SemTy::Dyn
                }
            }
            // `-x` / `+x` keep the numeric kind, with `bool` widening to `int`
            // (`-True == -1`). Non-numeric operands fall back to tagged.
            UnaryOp::Neg | UnaryOp::Pos => match operand {
                SemTy::Float => SemTy::Float,
                SemTy::Int | SemTy::Bool => SemTy::Int,
                SemTy::Never => SemTy::Never,
                _ => SemTy::Dyn,
            },
        }
    }

    /// Result type of a binary operator, applying CPython's numeric semantics.
    fn binop_ty(&self, op: BinOp, l: SemTy, r: SemTy) -> SemTy {
        match op {
            // Arithmetic follows the numeric tower via `join` (Bool ⊂ Int ⊂ Float;
            // same-type stays; mixed non-numerics → a tagged union). `**` is also
            // joined: `int ** int` is usually `int` (and its tagged repr prints a
            // bignum or a promoted float correctly either way — Principle 2).
            // `*` repeats a sequence by an int (`[0] * 3`, `(1,) * n`, `b"x" * 4`),
            // preserving the sequence type; otherwise it is numeric (joined).
            BinOp::Mul => {
                if is_sequence(&l) && is_int_like(&r) {
                    l
                } else if is_int_like(&l) && is_sequence(&r) {
                    r
                } else {
                    l.join(&r)
                }
            }
            // `+` over two same-base containers already joins to that container
            // (covariant lattice join), so list/tuple/bytes concatenation types
            // correctly without a special case.
            BinOp::Add | BinOp::Sub | BinOp::FloorDiv | BinOp::Mod | BinOp::Pow => l.join(&r),
            // Python 3 true division always yields `float` for numeric operands
            // (`7 / 2 == 3.5`).
            BinOp::Div => {
                if is_numeric(&l) && is_numeric(&r) {
                    SemTy::Float
                } else if l == SemTy::Never || r == SemTy::Never {
                    SemTy::Never
                } else {
                    SemTy::Dyn
                }
            }
            // Bitwise / shift are integer-valued when both operands are int-like.
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                if is_int_like(&l) && is_int_like(&r) {
                    SemTy::Int
                } else if l == SemTy::Never || r == SemTy::Never {
                    SemTy::Never
                } else {
                    l.join(&r)
                }
            }
        }
    }

    /// Result type of a call: a compiled function's declared return, or a
    /// per-builtin result type.
    fn call_ty(&self, callee: Idx<HirExpr>, args: &[Idx<HirExpr>]) -> SemTy {
        let symref = match &self.func.exprs[callee].kind {
            HirExprKind::Name(s) => *s,
            _ => return SemTy::Dyn,
        };
        let SymbolRef::Resolved(id) = symref else { return SemTy::Dyn };
        match self.resolve.symbol(id) {
            Symbol::Function(fid) => self.ret_tys[fid.index()].clone(),
            Symbol::Builtin(kind) => self.builtin_ty(kind, args),
            Symbol::Container(op) => self.container_op_ty(op, args),
            // `range(...)` used as a value is an iterable of ints.
            Symbol::BuiltinRange => SemTy::Iterator(Box::new(SemTy::Int)),
            _ => SemTy::Dyn,
        }
    }

    /// Per-builtin result type.
    fn builtin_ty(&self, kind: BuiltinFunctionKind, args: &[Idx<HirExpr>]) -> SemTy {
        use BuiltinFunctionKind as K;
        match kind {
            K::Len | K::Hash | K::Ord => SemTy::Int,
            K::Int => SemTy::Int,
            K::Float => SemTy::Float,
            K::Bool => SemTy::Bool,
            K::Str | K::Repr | K::Chr => SemTy::Str,
            // `abs` preserves the numeric kind of its argument.
            K::Abs => match args.first().map(|a| self.ety(*a)) {
                Some(SemTy::Float) => SemTy::Float,
                _ => SemTy::Int,
            },
            K::Type => SemTy::Dyn,
        }
    }
}

/// True for `int` / `bool` (the int-like operands of bitwise / shift ops).
fn is_int_like(t: &SemTy) -> bool {
    matches!(t, SemTy::Int | SemTy::Bool)
}

/// True for the numeric-tower types `bool` / `int` / `float`.
fn is_numeric(t: &SemTy) -> bool {
    matches!(t, SemTy::Bool | SemTy::Int | SemTy::Float)
}

/// True for the repeatable sequence types (`list` / `tuple` / `bytes` / `str`).
fn is_sequence(t: &SemTy) -> bool {
    matches!(t, SemTy::Bytes | SemTy::Str)
        || t.list_elem().is_some()
        || t.tuple_elems().is_some()
        || t.tuple_var_elem().is_some()
}

/// Result type of a container method call from the receiver type and method (the
/// concrete dispatch is by receiver in lowering; here we just produce the Python
/// result type). Unknown (receiver, method) pairs fall back to `Dyn` — the
/// always-correct tagged baseline.
fn method_ty(recv: &SemTy, method: ContainerMethod) -> SemTy {
    use ContainerMethod as M;
    let none = SemTy::NoneTy;
    // List receiver.
    if let Some(elem) = recv.list_elem() {
        return match method {
            M::Append | M::Insert | M::Extend | M::Clear | M::Reverse | M::Sort => none,
            M::Pop => elem.clone(),
            M::Index | M::Count => SemTy::Int,
            M::Copy => recv.clone(),
            _ => SemTy::Dyn,
        };
    }
    // Dict receiver.
    if let Some((k, v)) = recv.dict_kv() {
        return match method {
            M::Get | M::Pop | M::Setdefault => v.clone(),
            M::Keys => SemTy::list_of(k.clone()),
            M::Values => SemTy::list_of(v.clone()),
            M::Items => SemTy::list_of(SemTy::tuple_of(vec![k.clone(), v.clone()])),
            M::Update | M::Clear => none,
            M::Copy => recv.clone(),
            _ => SemTy::Dyn,
        };
    }
    // Set receiver.
    if recv.set_elem().is_some() {
        return match method {
            M::Add | M::Remove | M::Discard | M::Update | M::Clear => none,
            M::Union | M::Intersection | M::Difference | M::Copy => recv.clone(),
            _ => SemTy::Dyn,
        };
    }
    SemTy::Dyn
}

/// The element type produced by iterating `t` (for `iter()` / `for`-loops). A
/// `str` iterates to single-char `str`; `bytes` to `int`; an unknown iterable to
/// `Dyn` (the always-correct tagged baseline).
///
/// The iterable being the lattice bottom (`Never`, an in-progress type this sweep)
/// yields `Never` so it stays the join identity and never poisons a consumer. But
/// a *recognized container* whose element is `Never` (an unrefined empty literal
/// — `f = []` then `f.append(...)` keeps `list[Never]`) yields **`Dyn`**, since at
/// runtime it holds tagged values of unknown type, not a bottom that would wrongly
/// type a `min`/`max` result as `None` (and print "None" without reading it).
fn iter_elem_ty(t: &SemTy) -> SemTy {
    let elem = iter_elem_raw(t);
    if elem == SemTy::Never && *t != SemTy::Never {
        SemTy::Dyn
    } else {
        elem
    }
}

fn iter_elem_raw(t: &SemTy) -> SemTy {
    if let Some(e) = t.list_elem() {
        return e.clone();
    }
    if let Some(e) = t.set_elem() {
        return e.clone();
    }
    if let Some(e) = t.tuple_var_elem() {
        return e.clone();
    }
    if let Some(elems) = t.tuple_elems() {
        return elems.iter().fold(SemTy::Never, |acc, x| acc.join(x));
    }
    if let Some((k, _)) = t.dict_kv() {
        // Iterating a dict yields its keys.
        return k.clone();
    }
    match t {
        SemTy::Str => SemTy::Str,
        SemTy::Bytes => SemTy::Int,
        SemTy::Iterator(e) => (**e).clone(),
        SemTy::Never => SemTy::Never,
        _ => SemTy::Dyn,
    }
}

/// **materialize** — write solved types back. Expr nodes that solved to `Never`
/// (genuinely unconstrained / unreachable) keep their frontend type rather than
/// taking the bottom representation.
fn materialize(func: &mut HirFunction, solution: &Solution) {
    for (i, local) in func.locals.iter_mut().enumerate() {
        let ty = &solution.local_ty[i];
        if *ty != SemTy::Never {
            local.ty = ty.clone();
        }
        // Defensive soundness gate for the Phase-3c `Raw(I64)` override: only an
        // `int` slot may take it. If inference produced anything else, drop the
        // flag so lowering keeps the safe tagged representation (PITFALLS A6).
        if local.raw_int_ok && local.ty != SemTy::Int {
            local.raw_int_ok = false;
        }
    }
    for (idx, expr) in func.exprs.iter_mut() {
        if let Some(ty) = solution.expr_ty.get(&idx) {
            if *ty != SemTy::Never {
                expr.ty = ty.clone();
            }
        }
    }
    bootstrap_empty_containers(func);
}

/// **Empty-container element-type bootstrap (PITFALLS B4).** A terminal, forward
/// materialize-time rule (no feedback, so it does not reopen the A3 fixpoint):
/// for an `Assign { target, value }` whose `target` is an authoritative container
/// local (`x: list[int]`) and whose `value` is an empty literal solved to
/// `…_of(Never)`, overwrite the literal's type with the target's container type.
///
/// Without this, `x: list[int] = []` lowers the literal to `Heap(List(Never))`
/// while `x` is `Heap(List(Tagged))`, and the assignment coercion is illegal —
/// the literal must carry the element type *before any store*. A non-annotated
/// `x = []` keeps `…_of(Never)` → `Tagged` element slots (correct, just slower).
fn bootstrap_empty_containers(func: &mut HirFunction) {
    // Collect (value-expr, target-type) overwrites first to avoid borrowing
    // `func.exprs` mutably while reading `func.locals` / `func.blocks`.
    let mut overwrites: Vec<(Idx<HirExpr>, SemTy)> = Vec::new();
    for (_b, block) in func.blocks.iter() {
        for stmt in &block.stmts {
            let HirStmt::Assign { target, value } = stmt else { continue };
            let target_ty = func.locals[target.index()].ty.clone();
            if !is_growable_container(&target_ty) {
                continue;
            }
            if is_empty_container_literal(&func.exprs[*value]) {
                overwrites.push((*value, target_ty));
            }
        }
    }
    for (value, ty) in overwrites {
        func.exprs[value].ty = ty;
    }
}

/// True for the growable built-in containers seeded by an empty literal.
fn is_growable_container(t: &SemTy) -> bool {
    t.list_elem().is_some() || t.dict_kv().is_some() || t.set_elem().is_some()
}

/// True iff `expr` is an empty container literal (`[]` / `{}` / `set()`-shaped)
/// whose solved element type is the lattice bottom.
fn is_empty_container_literal(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::ListLit { elems } | HirExprKind::SetLit { elems } => elems.is_empty(),
        HirExprKind::DictLit { pairs } => pairs.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
