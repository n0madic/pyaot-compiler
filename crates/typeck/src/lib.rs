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
    BinOp, BuiltinFunctionKind, ClassTable, ContainerMethod, ContainerOp, HirExpr, HirExprKind,
    HirFunction, HirModule, HirStmt, HirTerminator, ResolveResult, Symbol, SymbolRef, UnaryOp,
};
use pyaot_types::{repr_of, sig_repr, RawKind, Repr, SemTy, Sig, TypeLattice};
use pyaot_utils::{ClassId, InternedString, StringInterner};

/// Run inference over every function, mutating each node's [`SemTy`] in place.
///
/// Per-function inference: a callee's return type is read from its (annotated or
/// `Dyn`) signature — return-type inference across functions is not in scope.
/// Class field/method/return types and nominal subtyping are consulted through
/// the [`ClassTable`] oracle (D4/D8), never baked into the sealed lattice.
pub fn infer(
    module: &mut HirModule,
    resolve: &ResolveResult,
    classes: &ClassTable,
    interner: &StringInterner,
) -> Result<()> {
    // Snapshot each function's declared return type so `Call` results can be typed
    // without holding a second borrow of `module` while we mutate a function.
    let ret_tys: Vec<SemTy> = module.functions.iter().map(|f| f.ret_ty.clone()).collect();
    // The visible Callable signature of each function when used as a closure
    // target (Phase 6A): declared params MINUS the env param 0 (every
    // `MakeClosure` target carries one), plus the 6C varargs/kwargs flags.
    let closure_sigs: Vec<Sig> = module
        .functions
        .iter()
        .map(|f| Sig {
            params: f.params.iter().skip(1).map(|p| p.ty.clone()).collect(),
            ret: f.ret_ty.clone(),
            varargs: f.varargs,
            kwargs: f.kwargs,
        })
        .collect();
    // ── promoted-global slot typing (Phase 6B) ──
    // The slot type is the join of `__main__`'s writes (the single storage's
    // module-level initializers / rebindings); a slot any OTHER function writes
    // (`global` declaration) is demoted to `Dyn` — that write's type is
    // invisible to main's solve, so a precise type would be unsound. `__main__`
    // is therefore solved FIRST; its solution seeds `global_tys` for the rest.
    let main_idx = module.main.index();
    let mut n_globals = 0usize;
    let mut demoted: Vec<bool> = Vec::new();
    for (i, f) in module.functions.iter().enumerate() {
        let mut grow = |vid: u32, demote: bool, demoted: &mut Vec<bool>| {
            let vid = vid as usize;
            if vid >= n_globals {
                n_globals = vid + 1;
                demoted.resize(n_globals, false);
            }
            if demote {
                demoted[vid] = true;
            }
        };
        for (_b, block) in f.blocks.iter() {
            for stmt in &block.stmts {
                if let HirStmt::GlobalSet { var_id, .. } = stmt {
                    grow(*var_id, i != main_idx, &mut demoted);
                }
            }
        }
        for (_e, expr) in f.exprs.iter() {
            if let HirExprKind::GlobalGet { var_id } = expr.kind {
                grow(var_id, false, &mut demoted);
            }
        }
    }
    let mut global_tys: Vec<SemTy> = vec![SemTy::Dyn; n_globals];

    let mut order: Vec<usize> = Vec::with_capacity(module.functions.len());
    order.push(main_idx);
    order.extend((0..module.functions.len()).filter(|i| *i != main_idx));
    for idx in order {
        let solution = {
            let solver = Solver::collect(
                &module.functions[idx],
                resolve,
                &ret_tys,
                &closure_sigs,
                classes,
                interner,
                &global_tys,
                &demoted,
                n_globals,
            );
            solver.solve()
        };
        if idx == main_idx {
            seed_global_tys(&module.functions[idx], &solution, &demoted, &mut global_tys);
        }
        materialize(&mut module.functions[idx], &solution);
    }
    // Types are now materialized on every node; validate the unboxed-slot
    // boundaries before lowering can emit an unsound coercion.
    check_repr_boundaries(module, resolve, classes, interner)?;
    Ok(())
}

/// Compute each global slot's type from `__main__`'s solved writes: the join,
/// with the same `Raw`-uniformity guard as locals (a mixed-repr join must stay
/// `Dyn`, never an unbox hint). Demoted or never-written slots stay `Dyn`.
fn seed_global_tys(
    main: &HirFunction,
    solution: &Solution,
    demoted: &[bool],
    global_tys: &mut [SemTy],
) {
    let mut writes: Vec<Vec<Idx<HirExpr>>> = vec![Vec::new(); global_tys.len()];
    for (_b, block) in main.blocks.iter() {
        for stmt in &block.stmts {
            if let HirStmt::GlobalSet { var_id, value } = stmt {
                writes[*var_id as usize].push(*value);
            }
        }
    }
    for (vid, ws) in writes.iter().enumerate() {
        if demoted[vid] || ws.is_empty() {
            continue;
        }
        let ety = |v: &Idx<HirExpr>| {
            erase_vars(&solution.expr_ty.get(v).cloned().unwrap_or(SemTy::Never))
        };
        let mut joined = SemTy::Never;
        for v in ws {
            joined = joined.join(&ety(v));
        }
        if joined == SemTy::Never {
            continue; // stays Dyn
        }
        if matches!(repr_of(&joined), Repr::Raw(_)) {
            let target = repr_of(&joined);
            if !ws.iter().all(|v| {
                let t = ety(v);
                t == SemTy::Never || repr_of(&t) == target
            }) {
                continue; // stays Dyn
            }
        }
        global_tys[vid] = joined;
    }
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
    /// `Repr::Closure(sig)` (Phase 6A): the slot's signature IS the indirect-call
    /// ABI, so a stored callable must match it at the *representation* level
    /// exactly (subtyping that changes a param/ret `Repr` would forge a different
    /// ABI). `Dyn` is admitted gradually like `Gradual`.
    Closure,
}

/// The reinterpret family of a slot's representation, or `None` if storing into it
/// is always sound (`Tagged`, the proof-gated `Raw(I64)`, function pointers).
fn reinterpret_kind(ty: &SemTy) -> Option<ReinterpretKind> {
    match repr_of(ty) {
        Repr::Raw(RawKind::F64) | Repr::Raw(RawKind::I8) => Some(ReinterpretKind::Strict),
        Repr::Heap(_) => Some(ReinterpretKind::Gradual),
        Repr::Closure(_) => Some(ReinterpretKind::Closure),
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
fn check_repr_boundaries(
    module: &HirModule,
    resolve: &ResolveResult,
    classes: &ClassTable,
    interner: &StringInterner,
) -> Result<()> {
    for func in &module.functions {
        for (_b, block) in func.blocks.iter() {
            for stmt in &block.stmts {
                match stmt {
                    // Assignments into an annotated unboxed / typed-heap local slot.
                    HirStmt::Assign { target, value } => {
                        let local = &func.locals[target.index()];
                        // A pin_tagged slot stays `Tagged` regardless of its
                        // (content) type — storing into it never reinterprets.
                        if local.pin_tagged {
                            continue;
                        }
                        let target_ty = &local.ty;
                        if let Some(kind) = reinterpret_kind(target_ty) {
                            check_reinterpret(
                                &func.exprs[*value], target_ty, kind, "assigned to", classes,
                            )?;
                        }
                    }
                    // Writes through a cell whose content type is authoritative
                    // (a captured annotation): reads will reinterpret by that
                    // type, so the written value must match it (Phase 6A).
                    HirStmt::CellSet { cell, value } => {
                        let local = &func.locals[cell.index()];
                        if local.cell_shared || local.ty == SemTy::Dyn {
                            continue;
                        }
                        if let Some(kind) = reinterpret_kind(&local.ty) {
                            check_reinterpret(
                                &func.exprs[*value], &local.ty, kind, "assigned to", classes,
                            )?;
                        }
                    }
                    // Writes into a typed instance-field slot (the A5 storage seam):
                    // a `float` field reads back via `UnboxFloat`, a class/`str`
                    // field via `TaggedToHeap`, so the *written* value must match.
                    HirStmt::SetAttr { base, name, value } => {
                        if let Some(cid) = class_of(&func.exprs[*base].ty, classes) {
                            if let Some(info) = classes.get(cid) {
                                if let Some(field_ty) = info.field_ty(*name) {
                                    if let Some(kind) = reinterpret_kind(field_ty) {
                                        check_reinterpret(
                                            &func.exprs[*value], field_ty, kind,
                                            "assigned to field", classes,
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Return value into an annotated return slot.
            if let HirTerminator::Return(Some(v)) = &block.term {
                if let Some(kind) = reinterpret_kind(&func.ret_ty) {
                    check_reinterpret(&func.exprs[*v], &func.ret_ty, kind, "returned from", classes)?;
                }
            }
        }
        // Arguments into annotated parameters. This is the SAME reinterpret seam
        // as the original `poly(3)` guard, extended (Phase 5) to every call form
        // whose lowering coerces args to the callee's param `Repr` — free
        // functions, constructors, **method calls** (instance / static / class /
        // `super()` / virtual), and **generic construction** — since a
        // `Tagged → Raw(F64)`/typed-`Heap` coercion the verifier accepts would
        // otherwise mis-read a mismatched value (PITFALLS A2).
        for (_idx, expr) in func.exprs.iter() {
            let (params, args) = match &expr.kind {
                HirExprKind::Call { callee, args } => {
                    let direct = match func.exprs[*callee].kind {
                        HirExprKind::Name(SymbolRef::Resolved(id)) => match resolve.symbol(id) {
                            Symbol::Function(fid) => {
                                Some((&module.functions[fid.index()].params[..], args))
                            }
                            // `Cls(args)` → `__init__(self, args…)`: skip `self`.
                            Symbol::Class(cid) => {
                                match init_params(classes, module, interner, cid) {
                                    Some(p) => Some((&p[1.min(p.len())..], args)),
                                    None => continue,
                                }
                            }
                            Symbol::Local(_) => None,
                            _ => continue,
                        },
                        _ => None,
                    };
                    match direct {
                        Some(pair) => pair,
                        // Indirect call through a callable VALUE (Phase 6A):
                        // the callee must be a known `Callable` — calling a
                        // gradual `Dyn` cannot build an indirect-call signature
                        // and is rejected loudly — and every argument is
                        // guarded against the signature's param reprs (the
                        // poly(3) seam, extended to closures).
                        None => {
                            check_indirect_call(func, *callee, args, classes)?;
                            continue;
                        }
                    }
                }
                // `recv.m(args)` — resolve the target method through the dispatch
                // the lowering uses, skipping `self` for instance/`super()` calls.
                // Trailing `*args`/`**kwargs` slots receive lowering-packed
                // containers, never call-site values — drop them from the zip.
                HirExprKind::MethodCall { recv, method_name, args } => {
                    match method_call_target(func, resolve, classes, *recv, *method_name) {
                        Some((fid, skip_self)) => {
                            let callee = &module.functions[fid.index()];
                            let p = &callee.params[..];
                            let p = if skip_self { &p[1.min(p.len())..] } else { p };
                            let cut = p.len()
                                - usize::from(callee.varargs)
                                - usize::from(callee.kwargs);
                            (&p[..cut.min(p.len())], args)
                        }
                        None => continue,
                    }
                }
                // `Cls[T](args)` → `__init__(self, args…)`: skip `self`.
                HirExprKind::GenericConstruct { class_id, args, .. } => {
                    match init_params(classes, module, interner, *class_id) {
                        Some(p) => (&p[1.min(p.len())..], args),
                        None => continue,
                    }
                }
                _ => continue,
            };
            for (arg, param) in args.iter().zip(params) {
                if let Some(kind) = reinterpret_kind(&param.ty) {
                    check_reinterpret(&func.exprs[*arg], &param.ty, kind, "passed to", classes)?;
                }
            }
        }
    }
    Ok(())
}

/// Validate an indirect call `value(args…)` (Phase 6A): the callee's static
/// type must be a concrete `Callable` of matching arity, and each argument is
/// guarded against the parameter types exactly like direct-call arguments.
fn check_indirect_call(
    func: &HirFunction,
    callee: Idx<HirExpr>,
    args: &[Idx<HirExpr>],
    classes: &ClassTable,
) -> Result<()> {
    let cexpr = &func.exprs[callee];
    let sig = match &cexpr.ty {
        SemTy::Callable(sig) => sig,
        // Unreachable code may call anything.
        SemTy::Never => return Ok(()),
        SemTy::Dyn => {
            return Err(CompilerError::type_error(
                "cannot call a value of unknown type: annotate it (e.g. \
                 `Callable[[int], int]`) so the compiler can build its native \
                 call signature"
                    .to_string(),
                cexpr.span,
            ))
        }
        other => {
            return Err(CompilerError::type_error(
                format!("`{}` object is not callable", type_name(other)),
                cexpr.span,
            ))
        }
    };
    let fixed = sig.fixed_arity();
    let arity_ok = if sig.varargs { args.len() >= fixed } else { args.len() == fixed };
    if !arity_ok {
        return Err(CompilerError::type_error(
            format!(
                "this callable takes {} positional argument(s){} but {} were given \
                 (indirect calls require the full declared arity — defaults are \
                 filled only at direct call sites)",
                fixed,
                if sig.varargs { " or more" } else { "" },
                args.len(),
            ),
            cexpr.span,
        ));
    }
    for (arg, pty) in args.iter().zip(sig.params.iter().take(fixed)) {
        if let Some(kind) = reinterpret_kind(pty) {
            check_reinterpret(&func.exprs[*arg], pty, kind, "passed to", classes)?;
        }
    }
    Ok(())
}

/// The `__init__` parameter list for class `cid`, or `None` if it defines none.
fn init_params<'a>(
    classes: &ClassTable,
    module: &'a HirModule,
    interner: &StringInterner,
    cid: ClassId,
) -> Option<&'a [pyaot_hir::HirParam]> {
    let info = classes.get(cid)?;
    let m = info.methods.iter().find(|m| interner.resolve(m.name) == "__init__")?;
    Some(&module.functions[m.func_id.index()].params)
}

/// The class id a `Name` expr resolves to (a `Symbol::Class`) — for
/// `ClassName.attr` / `ClassName.method()` class-level access.
fn name_class_ref_at(
    func: &HirFunction,
    resolve: &ResolveResult,
    idx: Idx<HirExpr>,
) -> Option<ClassId> {
    if let HirExprKind::Name(SymbolRef::Resolved(id)) = func.exprs[idx].kind {
        if let Symbol::Class(cid) = resolve.symbol(id) {
            return Some(cid);
        }
    }
    None
}

/// Resolve a `recv.method(args)` call to its target `FuncId` plus whether `self`
/// is dropped from the arg→param alignment — mirroring `lowering::lower_method_call`
/// dispatch exactly (instance / static / class / `super()`). Used by the
/// reinterpret-boundary check so method-call args are validated like free-call args.
fn method_call_target(
    func: &HirFunction,
    resolve: &ResolveResult,
    classes: &ClassTable,
    recv: Idx<HirExpr>,
    method_name: pyaot_utils::InternedString,
) -> Option<(pyaot_utils::FuncId, bool)> {
    // `super().m()` → the parent method, called with the current `self`.
    if let HirExprKind::Super(cid) = func.exprs[recv].kind {
        return classes.resolve_super_method(cid, method_name).map(|f| (f, true));
    }
    // `ClassName.m()` → a static/class method (never an instance method).
    if let Some(cid) = name_class_ref_at(func, resolve, recv) {
        return classes.get(cid).and_then(|i| {
            i.static_method(method_name)
                .or_else(|| i.class_method(method_name))
                .map(|m| (m.func_id, false))
        });
    }
    // `instance.m()` → static/class method (no `self`), else an instance method.
    let cid = class_of(&func.exprs[recv].ty, classes)?;
    let info = classes.get(cid)?;
    if let Some(m) = info.static_method(method_name).or_else(|| info.class_method(method_name)) {
        return Some((m.func_id, false));
    }
    info.method(method_name).map(|m| (m.func_id, true))
}

/// The class id a receiver type denotes (a nominal `Class`, or a user generic
/// instance whose base is a user class), else `None`.
fn class_of(ty: &SemTy, classes: &ClassTable) -> Option<ClassId> {
    match ty {
        SemTy::Class { class_id, .. } => Some(*class_id),
        SemTy::Generic { base, .. } if classes.get(*base).is_some() => Some(*base),
        _ => None,
    }
}

/// Error unless `value`'s type may be soundly stored in a `target`-typed
/// reinterpret slot. `Never` (unreachable) is always accepted; a [`ReinterpretKind::Gradual`]
/// slot additionally accepts `Dyn` (gradual typing, deferred to a runtime guard).
fn check_reinterpret(
    value: &HirExpr,
    target: &SemTy,
    kind: ReinterpretKind,
    verb: &str,
    classes: &ClassTable,
) -> Result<()> {
    let ok = match kind {
        // A Callable slot requires representation-level signature equality —
        // ordinary subtyping could change a param/ret `Repr` and forge a
        // different indirect-call ABI (Phase 6A). `Dyn` is admitted gradually.
        ReinterpretKind::Closure => {
            value.ty == SemTy::Never
                || value.ty == SemTy::Dyn
                || match (&value.ty, target) {
                    (SemTy::Callable(vs), SemTy::Callable(ts)) => sig_repr(vs) == sig_repr(ts),
                    _ => false,
                }
        }
        _ => {
            value.ty == SemTy::Never
                || value.ty.is_subtype_of(target)
                || nominal_subtype(&value.ty, target, classes)
                || (kind == ReinterpretKind::Gradual && value.ty == SemTy::Dyn)
        }
    };
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
        ReinterpretKind::Closure =>
            "this compiler compiles a `Callable[...]` slot to that exact native \
             call signature, so the stored function's parameter/return types must \
             match the annotation exactly.",
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

/// Nominal subtyping via the [`ClassTable`] oracle (D8): consulted here rather
/// than baked into the sealed `types` lattice (`Class = id1 == id2`). Beyond the
/// base case (`target` in `value`'s MRO) this also closes the cases the
/// nominal-unaware lattice misses: covariant container elements (`list[Dog] <:
/// list[Animal]`) and a union of subclasses (`Union[Dog, Cat] <: Animal`, which
/// is how a mixed `[Dog(), Cat()]` literal types).
fn nominal_subtype(value: &SemTy, target: &SemTy, classes: &ClassTable) -> bool {
    // A single element is assignable if the lattice already says so, or nominally.
    let elem_ok = |v: &SemTy, t: &SemTy| {
        *v == SemTy::Never
            || *t == SemTy::Dyn
            || v.is_subtype_of(t)
            || nominal_subtype(v, t, classes)
    };
    match (value, target) {
        (SemTy::Class { class_id: a, .. }, SemTy::Class { class_id: b, .. }) => {
            classes.is_subclass(*a, *b)
        }
        // A union is assignable iff every member is.
        (SemTy::Union(members), _) => members.iter().all(|m| elem_ok(m, target)),
        // Covariant same-base generic containers (element-wise).
        (
            SemTy::Generic { base: b1, args: a1 },
            SemTy::Generic { base: b2, args: a2 },
        ) if b1 == b2 && a1.len() == a2.len() => {
            a1.iter().zip(a2.iter()).all(|(x, y)| elem_ok(x, y))
        }
        _ => false,
    }
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
        SemTy::Callable(_) => "Callable",
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
    /// Each function's visible Callable signature (params minus env) — the
    /// type a `MakeClosure` over it produces (Phase 6A).
    closure_sigs: &'a [Sig],
    classes: &'a ClassTable,
    interner: &'a StringInterner,
    /// Current per-expr type (absent = `Never`, the lattice bottom).
    expr_ty: HashMap<Idx<HirExpr>, SemTy>,
    /// Current per-local type.
    local_ty: Vec<SemTy>,
    /// `true` for locals whose frontend type is authoritative (a parameter or an
    /// explicit annotation): their type is fixed and never inferred.
    authoritative: Vec<bool>,
    /// Value expressions assigned to each local, indexed by `LocalId`.
    assignments: Vec<Vec<Idx<HirExpr>>>,
    /// Value expressions written into the cell held by each local (`CellSet`
    /// plus the `MakeCell` init), indexed by the cell local's `LocalId` — the
    /// per-cell constraint of Phase 6A (the B10 field-join shape).
    cell_writes: Vec<Vec<Idx<HirExpr>>>,
    /// Precomputed global slot types (from `__main__`'s writes; Phase 6B).
    global_tys: &'a [SemTy],
    /// Slots written outside `__main__` — always `Dyn` (Phase 6B).
    global_demoted: &'a [bool],
    /// This function's own `GlobalSet` writes per slot — only `__main__` has
    /// any when the slot is not demoted, making its reads a worklist join.
    global_writes: Vec<Vec<Idx<HirExpr>>>,
}

impl<'a> Solver<'a> {
    /// **collect** — seed the assignment table and the authoritative-local set.
    #[allow(clippy::too_many_arguments)]
    fn collect(
        func: &'a HirFunction,
        resolve: &'a ResolveResult,
        ret_tys: &'a [SemTy],
        closure_sigs: &'a [Sig],
        classes: &'a ClassTable,
        interner: &'a StringInterner,
        global_tys: &'a [SemTy],
        global_demoted: &'a [bool],
        n_globals: usize,
    ) -> Self {
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
        let mut cell_writes: Vec<Vec<Idx<HirExpr>>> = vec![Vec::new(); n];
        let mut global_writes: Vec<Vec<Idx<HirExpr>>> = vec![Vec::new(); n_globals];
        for (_bidx, block) in func.blocks.iter() {
            for stmt in &block.stmts {
                match stmt {
                    HirStmt::Assign { target, value } => {
                        assignments[target.index()].push(*value);
                        // A `MakeCell` init is the cell's first write.
                        if let HirExprKind::MakeCell { init: Some(i) } = func.exprs[*value].kind {
                            cell_writes[target.index()].push(i);
                        }
                    }
                    HirStmt::CellSet { cell, value } => {
                        cell_writes[cell.index()].push(*value);
                    }
                    HirStmt::GlobalSet { var_id, value } => {
                        global_writes[*var_id as usize].push(*value);
                    }
                    _ => {}
                }
            }
        }

        Solver {
            func,
            resolve,
            ret_tys,
            closure_sigs,
            classes,
            interner,
            expr_ty: HashMap::new(),
            local_ty,
            authoritative,
            assignments,
            cell_writes,
            global_tys,
            global_demoted,
            global_writes,
        }
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
        self.join_writes(&self.assignments[i])
    }

    /// Join the types of a set of written values with the `Raw`-repr soundness
    /// guard. Shared by inferred locals and cell contents (Phase 6A).
    ///
    /// `Never` is the in-progress bottom: a slot still being computed (or one
    /// only fed by not-yet-evaluated values across a loop back-edge) must stay
    /// `Never`, never jump to a spurious `Dyn`. `join` treats `Never` as the
    /// identity, so a bottom contributor is correctly ignored by dependents;
    /// an injected `Dyn` would instead absorb and poison them irreversibly.
    /// Genuinely-unconstrained slots are mapped to `Dyn` once, in materialize.
    fn join_writes(&self, writes: &[Idx<HirExpr>]) -> SemTy {
        let mut joined = SemTy::Never;
        for &v in writes {
            joined = joined.join(&self.ety(v));
        }
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
            let uniform = writes.iter().all(|&v| {
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
            HirExprKind::MethodCall { recv, method_name, .. } => {
                // `super().m()` resolves against the enclosing class's MRO; a
                // `ClassName.m()` static/classmethod resolves on the class; an
                // ordinary receiver dispatches by its static type.
                if let HirExprKind::Super(cid) = self.func.exprs[*recv].kind {
                    self.classes
                        .resolve_super_method(cid, *method_name)
                        .map(|fid| self.ret_tys[fid.index()].clone())
                        .unwrap_or(SemTy::Dyn)
                } else if let Some(cid) = self.name_class_ref(*recv) {
                    self.classes
                        .get(cid)
                        .and_then(|i| {
                            i.static_method(*method_name).or_else(|| i.class_method(*method_name))
                        })
                        .map(|m| self.ret_tys[m.func_id.index()].clone())
                        .unwrap_or(SemTy::Dyn)
                } else {
                    self.method_call_ty(self.ety(*recv), *method_name)
                }
            }
            HirExprKind::Attribute { value, name } => {
                // `ClassName.attr` → a class attribute (or static-method ref).
                if let Some(cid) = self.name_class_ref(*value) {
                    self.classes
                        .get(cid)
                        .and_then(|i| i.class_attr(*name))
                        .map(|a| a.ty.clone())
                        .unwrap_or(SemTy::Dyn)
                } else {
                    self.attribute_ty(self.ety(*value), *name)
                }
            }
            // `super()` itself only ever feeds a MethodCall; type it as the class so
            // it has a representation, though it is never read as a value.
            HirExprKind::Super(cid) => self
                .classes
                .get(*cid)
                .map(|info| SemTy::Class { class_id: *cid, name: info.name })
                .unwrap_or(SemTy::Dyn),
            HirExprKind::IsInstance { .. } => SemTy::Bool,
            // `Stack[int](...)` → the generic instance type (args drive precise
            // field/method substitution; erased at repr to one shared layout).
            HirExprKind::GenericConstruct { class_id, type_args, .. } => {
                SemTy::Generic { base: *class_id, args: type_args.clone() }
            }
            // ── closures / cells / globals (Phase 6) ──
            HirExprKind::MakeClosure { func, .. } => {
                SemTy::Callable(Box::new(self.closure_sigs[func.index()].clone()))
            }
            HirExprKind::MakeCell { .. } => SemTy::Dyn,
            HirExprKind::CellGet { cell } => {
                let l = &self.func.locals[cell.index()];
                // A cell another function may write (`nonlocal`) is invisible to
                // per-function inference — its reads stay gradual (Dyn), never a
                // precise type a cross-function write would falsify (P6-2).
                if l.cell_shared {
                    SemTy::Dyn
                } else if l.ty != SemTy::Dyn {
                    // The cell's authoritative CONTENT type — an enclosing
                    // annotation carried across the capture boundary by the
                    // frontend (the slot itself is a pin_tagged cell pointer).
                    l.ty.clone()
                } else {
                    self.join_writes(&self.cell_writes[cell.index()])
                }
            }
            // ── generators (Phase 6E) ──
            // A generator object is an iterator value; its slot reads / sent
            // value are uniform tagged storage (Dyn), the state an int, the
            // closing flag a bool.
            HirExprKind::MakeGenerator { .. } => SemTy::Iterator(Box::new(SemTy::Dyn)),
            HirExprKind::GenQuery { op, .. } => match op.result() {
                pyaot_hir::GenResult::Value => SemTy::Dyn,
                pyaot_hir::GenResult::Int => SemTy::Int,
                pyaot_hir::GenResult::Bool => SemTy::Bool,
                pyaot_hir::GenResult::None => SemTy::NoneTy,
            },
            // Global slots are uniform tagged storage; the slot type is the
            // join of `__main__`'s writes (a live worklist join inside main
            // itself, the precomputed table elsewhere), `Dyn` once any other
            // function writes it (Phase 6B).
            HirExprKind::GlobalGet { var_id } => {
                let vid = *var_id as usize;
                if self.global_demoted.get(vid).copied().unwrap_or(false) {
                    SemTy::Dyn
                } else if !self.global_writes[vid].is_empty() {
                    self.join_writes(&self.global_writes[vid])
                } else {
                    self.global_tys.get(vid).cloned().unwrap_or(SemTy::Dyn)
                }
            }
            // ── exceptions (Phase 7) ──
            // `Current` keeps the frontend-assigned static type (the except
            // clause's class); the match queries are booleans. All ride the
            // Tagged baseline (Principle 2) — no new constraints.
            HirExprKind::ExcQuery(q) => match q {
                pyaot_hir::ExcQuery::Current => self.func.exprs[idx].ty.clone(),
                pyaot_hir::ExcQuery::MatchesBuiltin(_)
                | pyaot_hir::ExcQuery::MatchesClass(_) => SemTy::Bool,
            },
            HirExprKind::ExcInstanceStr { .. } => SemTy::Str,
        }
    }

    /// The type-parameter substitution implied by a generic-instance receiver
    /// (`Stack[int]` → `{T ↦ int}`), if its base is a user generic class (5E).
    fn subst_for(&self, recv: &SemTy) -> Option<HashMap<pyaot_utils::InternedString, SemTy>> {
        let SemTy::Generic { base, args } = recv else { return None };
        let info = self.classes.get(*base)?;
        if info.type_params.is_empty() {
            return None;
        }
        Some(info.type_params.iter().copied().zip(args.iter().cloned()).collect())
    }

    /// Apply the receiver's type-param substitution to a member type (5E).
    fn apply_subst(&self, recv: &SemTy, ty: SemTy) -> SemTy {
        match self.subst_for(recv) {
            Some(subst) => ty.substitute(&subst),
            None => ty,
        }
    }

    /// The class id a `Name` expr resolves to (a `Symbol::Class`), for
    /// `ClassName.attr` / `ClassName.method()` class-level access.
    fn name_class_ref(&self, idx: Idx<HirExpr>) -> Option<ClassId> {
        if let HirExprKind::Name(SymbolRef::Resolved(id)) = self.func.exprs[idx].kind {
            if let Symbol::Class(cid) = self.resolve.symbol(id) {
                return Some(cid);
            }
        }
        None
    }

    /// Type of `recv.method(args)`: a class receiver yields the resolved method's
    /// (instance / static / class) declared return; a container receiver resolves
    /// the name to a [`ContainerMethod`] and reuses the Phase-4D [`method_ty`].
    fn method_call_ty(&self, recv: SemTy, method_name: InternedString) -> SemTy {
        if let Some(cid) = class_of(&recv, self.classes) {
            let ret = self
                .classes
                .get(cid)
                .and_then(|info| {
                    info.method(method_name)
                        .or_else(|| info.static_method(method_name))
                        .or_else(|| info.class_method(method_name))
                })
                .map(|m| self.ret_tys[m.func_id.index()].clone());
            return match ret {
                // Substitute the generic type params for a `Stack[int]` receiver.
                Some(t) => self.apply_subst(&recv, t),
                None => SemTy::Dyn,
            };
        }
        match ContainerMethod::from_name(self.interner.resolve(method_name)) {
            Some(cm) => method_ty(&recv, cm),
            None => SemTy::Dyn,
        }
    }

    /// Type of an attribute read `value.name` from the receiver's class: a
    /// `@property` getter's return, an instance field's best-effort type (D5), or
    /// a class attribute's type; `Dyn` for an unknown receiver/attribute.
    fn attribute_ty(&self, recv: SemTy, name: InternedString) -> SemTy {
        // `e.args` on a caught builtin exception — or a tuple clause of only
        // builtins — (Phase 7B): the args tuple. Typed `Dyn` (not
        // `tuple[Dyn, ...]`) so a user annotation like `args: tuple[str]` is
        // admitted gradually rather than rejected by the tuple-arity contract.
        let builtin_exc = match &recv {
            SemTy::BuiltinException(_) => true,
            SemTy::Union(members) => {
                !members.is_empty()
                    && members.iter().all(|m| matches!(m, SemTy::BuiltinException(_)))
            }
            _ => false,
        };
        if builtin_exc {
            return SemTy::Dyn;
        }
        let Some(cid) = class_of(&recv, self.classes) else { return SemTy::Dyn };
        let Some(info) = self.classes.get(cid) else { return SemTy::Dyn };
        let raw = if let Some(p) = info.property(name) {
            p.ty.clone()
        } else if let Some(t) = info.field_ty(name) {
            t.clone()
        } else if let Some(a) = info.class_attr(name) {
            a.ty.clone()
        } else {
            return SemTy::Dyn;
        };
        // Substitute the generic type params for a `Stack[int]` receiver (5E).
        self.apply_subst(&recv, raw)
    }

    /// Join the types of every expr in `elems` (the lattice bottom for empty).
    fn join_all(&self, elems: &[Idx<HirExpr>]) -> SemTy {
        elems.iter().fold(SemTy::Never, |acc, e| acc.join(&self.ety(*e)))
    }

    /// The declared return type of a concrete-class dunder `name` on `ty`, if any
    /// (used to type class operator results precisely — `v + v → Vector`).
    fn class_dunder_ret(&self, ty: &SemTy, name: &str) -> Option<SemTy> {
        let cid = class_of(ty, self.classes)?;
        let info = self.classes.get(cid)?;
        let m = info.methods.iter().find(|m| self.interner.resolve(m.name) == name)?;
        Some(self.ret_tys[m.func_id.index()].clone())
    }

    /// Result type of a subscript read `base[index]`, from the base's container
    /// shape. A fixed-tuple indexed by an integer literal yields that slot's type.
    fn subscript_ty(&self, base: Idx<HirExpr>, index: Idx<HirExpr>) -> SemTy {
        let bt = self.ety(base);
        // A class with `__getitem__` yields that method's declared return (5C).
        if let Some(t) = self.class_dunder_ret(&bt, "__getitem__") {
            return t;
        }
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
            // ── ops the Phase-7E match desugar emits directly ──
            C::DictGet | C::DictPopM => {
                let a = arg0();
                match a.dict_kv() {
                    Some((_, v)) => v.clone(),
                    None => SemTy::Dyn,
                }
            }
            C::DictCopy => {
                let a = arg0();
                if a.dict_kv().is_some() {
                    a
                } else {
                    SemTy::dict_of(SemTy::Dyn, SemTy::Dyn)
                }
            }
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
        // A class with the corresponding dunder yields its declared return (5C).
        let dunder = match op {
            UnaryOp::Neg => Some("__neg__"),
            UnaryOp::Pos => Some("__pos__"),
            UnaryOp::Invert => Some("__invert__"),
            UnaryOp::Not => None,
        };
        if let Some(name) = dunder {
            if let Some(t) = self.class_dunder_ret(&operand, name) {
                return t;
            }
        }
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
        // A class left operand defining the operator's dunder yields that method's
        // declared return (`v + v → Vector`, `v * 2 → Vector`) — 5C.
        let dunder = match op {
            BinOp::Add => "__add__",
            BinOp::Sub => "__sub__",
            BinOp::Mul => "__mul__",
            BinOp::Div => "__truediv__",
            BinOp::FloorDiv => "__floordiv__",
            BinOp::Mod => "__mod__",
            BinOp::Pow => "__pow__",
            BinOp::BitAnd => "__and__",
            BinOp::BitOr => "__or__",
            BinOp::BitXor => "__xor__",
            BinOp::Shl => "__lshift__",
            BinOp::Shr => "__rshift__",
        };
        if let Some(t) = self.class_dunder_ret(&l, dunder) {
            return t;
        }
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

    /// Result type of a call: a compiled function's declared return, a
    /// per-builtin result type, or — for a callable VALUE (closure / lambda /
    /// thunk, Phase 6A) — the value's `Callable` return type.
    fn call_ty(&self, callee: Idx<HirExpr>, args: &[Idx<HirExpr>]) -> SemTy {
        if let HirExprKind::Name(SymbolRef::Resolved(id)) = &self.func.exprs[callee].kind {
            match self.resolve.symbol(*id) {
                Symbol::Function(fid) => return self.ret_tys[fid.index()].clone(),
                Symbol::Builtin(kind) => return self.builtin_ty(kind, args),
                Symbol::Container(op) => return self.container_op_ty(op, args),
                // `Cls(args)` constructs an instance of that class.
                Symbol::Class(cid) => {
                    return match self.classes.get(cid) {
                        Some(info) => SemTy::Class { class_id: cid, name: info.name },
                        None => SemTy::Dyn,
                    }
                }
                // `range(...)` used as a value is an iterable of ints.
                Symbol::BuiltinRange => return SemTy::Iterator(Box::new(SemTy::Int)),
                // A local holding a callable value → fall through to its type.
                Symbol::Local(_) => {}
                _ => return SemTy::Dyn,
            }
        }
        // Indirect call: type by the callee VALUE's signature.
        match self.ety(callee) {
            SemTy::Callable(sig) => sig.ret.clone(),
            SemTy::Never => SemTy::Never,
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
/// Erase any residual type variable to `Dyn` (→ `Tagged`) — a generic accessed
/// without instantiation args (a bare `Stack()`) leaves `Var`s the codegen has no
/// representation for (5E). `repr_of(Var)` is already `Tagged`, so this is a
/// clarity/soundness belt rather than a correctness requirement.
fn erase_vars(ty: &SemTy) -> SemTy {
    if !ty.contains_var() {
        return ty.clone();
    }
    match ty {
        SemTy::Var(_) => SemTy::Dyn,
        SemTy::Generic { base, args } => {
            SemTy::Generic { base: *base, args: args.iter().map(erase_vars).collect() }
        }
        SemTy::Iterator(t) => SemTy::Iterator(Box::new(erase_vars(t))),
        // Re-join union members through the lattice so an erased `Dyn` absorbs.
        SemTy::Union(ts) => ts.iter().fold(SemTy::Never, |acc, t| acc.join(&erase_vars(t))),
        other => other.clone(),
    }
}

fn materialize(func: &mut HirFunction, solution: &Solution) {
    for (i, local) in func.locals.iter_mut().enumerate() {
        let ty = erase_vars(&solution.local_ty[i]);
        let ty = &ty;
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
            let ty = erase_vars(ty);
            if ty != SemTy::Never {
                expr.ty = ty;
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
