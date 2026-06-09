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
    BinOp, BuiltinFunctionKind, HirExpr, HirExprKind, HirFunction, HirModule, HirStmt,
    HirTerminator, ResolveResult, Symbol, SymbolRef, UnaryOp,
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

/// True for the strongly-typed unboxed representations whose tagged→raw coercion
/// *reinterprets the bits by assumed type* (`UnboxFloat` reads a heap-float
/// pointer; `UntagBool` reads the bool payload). Feeding such a slot a value of
/// the wrong type silently misreads it — a SIGSEGV for `Raw(F64)` on a fixnum, a
/// wrong value for `Raw(I8)`. `Raw(I64)` is excluded: it is only ever produced by
/// the proof-gated 3c narrowing, never at an annotation boundary.
fn reinterprets_by_type(ty: &SemTy) -> bool {
    matches!(repr_of(ty), Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I8))
}

/// Reject a value whose static type cannot be soundly stored in a typed unboxed
/// slot (an annotated `float`/`bool` parameter, local, or return).
///
/// In CPython a type annotation is not enforced — `poly(3)` for `def poly(a:
/// float)` just runs with `a == 3`. This compiler, however, *unboxes* annotated
/// `float`/`bool` slots into `Raw(F64)`/`Raw(I8)` (the headline Phase-3 ABI), so a
/// mismatched value would be misread (PITFALLS A2). Rather than accept-then-crash,
/// we treat the annotation as a contract and reject the violation loudly and
/// soundly. (A future whole-program pass could instead demote such a parameter to
/// `Tagged` when a call site proves it polymorphic — PITFALLS B10, deferred.)
fn check_repr_boundaries(module: &HirModule, resolve: &ResolveResult) -> Result<()> {
    for func in &module.functions {
        // Assignments into an annotated `float`/`bool` local slot.
        for (_b, block) in func.blocks.iter() {
            for stmt in &block.stmts {
                if let HirStmt::Assign { target, value } = stmt {
                    let target_ty = &func.locals[target.index()].ty;
                    if reinterprets_by_type(target_ty) {
                        check_assignable(&func.exprs[*value], target_ty, "assigned to")?;
                    }
                }
            }
            // Return value into an annotated `float`/`bool` return slot.
            if let HirTerminator::Return(Some(v)) = &block.term {
                if reinterprets_by_type(&func.ret_ty) {
                    check_assignable(&func.exprs[*v], &func.ret_ty, "returned from")?;
                }
            }
        }
        // Call arguments into annotated `float`/`bool` parameters.
        for (_idx, expr) in func.exprs.iter() {
            let HirExprKind::Call { callee, args } = &expr.kind else { continue };
            let HirExprKind::Name(SymbolRef::Resolved(id)) = func.exprs[*callee].kind else {
                continue;
            };
            let Symbol::Function(fid) = resolve.symbol(id) else { continue };
            let callee_fn = &module.functions[fid.index()];
            for (arg, param) in args.iter().zip(&callee_fn.params) {
                if reinterprets_by_type(&param.ty) {
                    check_assignable(&func.exprs[*arg], &param.ty, "passed to")?;
                }
            }
        }
    }
    Ok(())
}

/// Error unless `value`'s type may be soundly stored in a `target`-typed unboxed
/// slot. `Never` (unreachable) values are accepted.
fn check_assignable(value: &HirExpr, target: &SemTy, verb: &str) -> Result<()> {
    if value.ty == SemTy::Never || value.ty.is_subtype_of(target) {
        return Ok(());
    }
    Err(CompilerError::type_error(
        format!(
            "a value of type `{}` cannot be {verb} a `{}` slot: a type annotation \
             is a contract here, not a coercion (this compiler unboxes annotated \
             `float`/`bool` slots, so a mismatched value would be misread). Pass a \
             matching type, e.g. `3.0` instead of `3`.",
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
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::FloorDiv | BinOp::Mod | BinOp::Pow => {
                l.join(&r)
            }
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
}

#[cfg(test)]
mod tests;
