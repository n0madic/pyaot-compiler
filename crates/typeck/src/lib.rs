//! # typeck — one constraint-based type inference
//!
//! Type inference is ONE constraint system over ONE lattice — never a fixpoint
//! of mutually recursive, re-collected passes (PITFALLS A3 / Principle 5). The
//! variables of the system are
//!
//! * the [`SemTy`] of every expression and every inferred local (per function),
//! * `ret_ty[fid]` — every unannotated function's inferred return type, and
//! * `global_ty[vid]` — every promoted global slot's type
//!
//! ([`ModuleVars`]; annotated returns / annotated or demoted globals are
//! *constants* of the system, not variables). It runs in three phases:
//!
//! 1. **collect** — ONE walk over each function's HIR builds its [`FuncState`]:
//!    the per-local assignment table, the cell / global write tables, and which
//!    locals are *authoritative* (carry a frontend annotation, so their type
//!    drives `Repr` and inference must not touch them). Collect happens exactly
//!    once; the tables never change across solve rounds.
//! 2. **solve** — chaotic iteration of the whole system in rounds. Each round
//!    re-solves every function **from bottom** against the current module
//!    variables (a per-function monotone Gauss-Seidel worklist,
//!    [`Sweeper::solve`]; local↔expr dependencies are cyclic across loop
//!    back-edges, the lattice join makes the sweep converge), then moves the
//!    module variables: `global_ty` is recomputed from `__main__`'s solved
//!    writes, `ret_ty` is lifted by joining each function's solved `return`
//!    types. The loop ends the first round no variable moves. A round only
//!    re-solves the functions whose dynamic read-set ([`ReadSet`] — the
//!    `ret_ty` / `global_ty` variables actually read during their last sweep)
//!    touched a moved variable: a sweep is a deterministic function of the
//!    variables it reads, so skipping a function whose reads are unchanged
//!    reproduces its solution (and its read-set) exactly — dirty-marking is a
//!    pure optimization, never a semantic knob. Re-solving from
//!    bottom (instead of resuming the previous round's state) is load-bearing:
//!    `join_writes`'s Raw-uniformity guard is non-monotone, so a transiently
//!    imprecise variable (a briefly-`Dyn` global feeding a cyclic local) must
//!    not absorb into a persistent solution — the final state is, by
//!    construction, each function's from-bottom solution at the converged
//!    variables.
//! 3. **materialize** — write the converged [`SemTy`] back onto each HIR expr
//!    **and** each inferred [`pyaot_hir::HirLocal`], so `repr_of` can pick
//!    `Raw(F64)` for float locals / `Raw(I8)` for bool locals. Authoritative
//!    (annotated) locals keep their declared type.
//!
//! **Termination** is by widening, not an iteration cap: a module variable
//! still moving after [`WIDEN_LIMIT`] strict moves is widened to `Dyn` and
//! pinned. Every round either moves at least one variable or is the last, and
//! each variable moves at most `WIDEN_LIMIT` times, so the loop runs at most
//! `(n_funcs + n_globals) · WIDEN_LIMIT + 1` rounds. The same widening bounds
//! the INNER sweep: an expr whose type keeps moving within one solve (a
//! self-recursive container constraint like `x = [x]`, re-joined through a
//! local / cell / global slot every iteration) is cut to `Dyn` and pinned for
//! the rest of that solve; locals need no counters of their own because a
//! local is a pure join of expr types. Precision lost to widening is only
//! performance, never correctness (Principle 2).
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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use la_arena::Idx;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    BinOp, BuiltinFunctionKind, ClassTable, ContainerMethod, ContainerOp, HirExpr, HirExprKind,
    HirFunction, HirModule, HirStmt, HirTerminator, ResolveResult, Symbol, SymbolRef, UnaryOp,
};
use pyaot_types::{repr_of, sig_repr, RawKind, Repr, SemTy, Sig, TypeLattice};
use pyaot_utils::{ClassId, InternedString, StringInterner};

/// Widening bound: how many moves a variable — a module-level `ret_ty` /
/// `global_ty` across rounds, or an expr type within one sweep — may make
/// before it is widened to `Dyn` and pinned. This is the standard
/// abstract-interpretation widening that makes both loops terminate on the
/// lattice's infinite-height container spine — a recursive constraint like
/// `def f(): return [f()]` (across rounds) or `x = [x]` (within a sweep)
/// climbs `list[Never] ⊏ list[list[Never]] ⊏ …` forever otherwise. Any value
/// is correct — `Dyn` rides the always-sound tagged baseline (Principle 2) —
/// and real code settles in 1–2 moves; 16 leaves generous headroom for deep
/// numeric/container towers.
const WIDEN_LIMIT: usize = 16;

/// Run inference over every function, mutating each node's [`SemTy`] in place.
///
/// One constraint system (see the module docs): per-function expr/local types,
/// cross-function inferred return types, and promoted-global slot types are
/// variables of the same lattice, solved by one round loop to convergence.
/// Class field/method/return types are consulted through the [`ClassTable`]
/// oracle (D4); nominal subtyping is MRO-aware inside the lattice itself, which
/// consults the same table through the `ClassHierarchy` env (D8) — the MRO data
/// still lives only in [`ClassTable`].
pub fn infer(
    module: &mut HirModule,
    resolve: &ResolveResult,
    classes: &mut ClassTable,
    interner: &StringInterner,
) -> Result<()> {
    let n_funcs = module.functions.len();
    // The `ret_ty` variables (Phase 8E). An annotated function (`ret_ty != Dyn`)
    // is a constant of the system; an unannotated one starts at `Never` (bottom)
    // and climbs to the join of its `return` expression types over the rounds
    // below. This refines how *callers* type a call result — so a `v.method()`
    // / dunder result is usable as its real class instead of `Dyn` — WITHOUT
    // changing any function's ABI: `func.ret_ty` (hence the lowered signature)
    // is untouched, and the precise type rides the tagged return, reinterpreted
    // at the use site (`Tagged → Heap(Class)`).
    let ret_annotated: Vec<bool> = module
        .functions
        .iter()
        .map(|f| f.ret_ty != SemTy::Dyn)
        .collect();
    let ret_ty: Vec<SemTy> = module
        .functions
        .iter()
        .map(|f| {
            if f.ret_ty != SemTy::Dyn {
                f.ret_ty.clone()
            } else {
                SemTy::Never
            }
        })
        .collect();
    // The visible Callable signature of each function when used as a closure
    // target (Phase 6A): declared params MINUS the env param 0 (every
    // `MakeClosure` target carries one), plus the 6C varargs/kwargs flags.
    // Built from DECLARED types only — an inferred return never changes an ABI.
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
    // ── promoted-global slot discovery (Phase 6B) ──
    // A slot any function other than `__main__` writes (`global` declaration)
    // is demoted to `Dyn` — that write's type is invisible to main's solve, so
    // a precise type would be unsound. `main_writes` marks the slots whose type
    // is a genuine variable, recomputed from main's solution each round.
    fn grow(
        vid: u32,
        n_globals: &mut usize,
        demoted: &mut Vec<bool>,
        main_writes: &mut Vec<bool>,
    ) -> usize {
        let vid = vid as usize;
        if vid >= *n_globals {
            *n_globals = vid + 1;
            demoted.resize(*n_globals, false);
            main_writes.resize(*n_globals, false);
        }
        vid
    }
    let main_idx = module.main.index();
    let mut n_globals = 0usize;
    let mut demoted: Vec<bool> = Vec::new();
    let mut main_writes: Vec<bool> = Vec::new();
    for (i, f) in module.functions.iter().enumerate() {
        for (_b, block) in f.blocks.iter() {
            for stmt in &block.stmts {
                if let HirStmt::GlobalSet { var_id, .. } = stmt {
                    let vid = grow(*var_id, &mut n_globals, &mut demoted, &mut main_writes);
                    if i == main_idx {
                        main_writes[vid] = true;
                    } else {
                        demoted[vid] = true;
                    }
                }
            }
        }
        for (_e, expr) in f.exprs.iter() {
            if let HirExprKind::GlobalGet { var_id } = expr.kind {
                grow(var_id, &mut n_globals, &mut demoted, &mut main_writes);
            }
        }
    }
    // The `global_ty` variables: bottom (`Never`) for a slot `__main__` writes
    // — recomputed from main's solved writes each round, and `Never` is the
    // join identity, so a not-yet-known slot never poisons a reader. A demoted
    // slot, or one main never writes, is the constant `Dyn`.
    let mut global_ty: Vec<SemTy> = (0..n_globals)
        .map(|vid| {
            if main_writes[vid] && !demoted[vid] {
                SemTy::Never
            } else {
                SemTy::Dyn
            }
        })
        .collect();
    // Module-level annotated globals are authoritative (Phase 8): their declared
    // type holds even when a function writes the slot (which otherwise demotes
    // inference to `Dyn`). The annotation is a contract; `check_repr_boundaries`
    // validates each write against it.
    let mut global_authoritative: Vec<bool> = vec![false; n_globals];
    for (vid, ty) in &module.global_annotations {
        let vid = *vid as usize;
        if vid < n_globals {
            global_ty[vid] = ty.clone();
            global_authoritative[vid] = true;
        }
    }
    // ── field-type variables (B10) ──
    // One solver variable per `(defining class, field name)` pair, where the
    // defining class is the LAST class in the MRO whose layout has the field
    // (fields are flattened parent-first, so the most-base declarer owns the
    // variable; a write through a subclass receiver feeds the base's
    // variable). Constants of the system — no variable — are: fields
    // annotated anywhere in the hierarchy (`name: T` is a contract,
    // `check_repr_boundaries` enforces every write), and fields defined by a
    // GENERIC class (their types mention type params; `apply_subst` keeps the
    // static path).
    let mut field_const: HashSet<(ClassId, InternedString)> = HashSet::new();
    for c in &module.classes {
        for (name, _) in &c.field_annotations {
            if let Some(d) = defining_class(classes, c.class_id, *name) {
                field_const.insert((d, *name));
            }
        }
    }
    let mut field_vars: HashMap<(ClassId, InternedString), usize> = HashMap::new();
    let mut field_var_names: Vec<InternedString> = Vec::new();
    for info in classes.iter() {
        if !info.type_params.is_empty() {
            continue;
        }
        for f in &info.fields {
            let Some(d) = defining_class(classes, info.class_id, f.name) else {
                continue;
            };
            // Create the variable when visiting the definer itself, so the
            // generic-class skip applies to the DEFINING class.
            if d != info.class_id || field_const.contains(&(d, f.name)) {
                continue;
            }
            field_vars.entry((d, f.name)).or_insert_with(|| {
                field_var_names.push(f.name);
                field_var_names.len() - 1
            });
        }
    }
    let n_field_vars = field_var_names.len();

    let mut vars = ModuleVars {
        ret_ty,
        global_ty,
        field_ty: vec![SemTy::Never; n_field_vars],
        ret_moves: vec![0; n_funcs],
        global_moves: vec![0; n_globals],
        field_moves: vec![0; n_field_vars],
        global_pinned: vec![false; n_globals],
        field_pinned: vec![false; n_field_vars],
    };

    // `__main__` is solved FIRST each round: its solution defines the global
    // slot types every other function reads.
    let mut order: Vec<usize> = Vec::with_capacity(n_funcs);
    order.push(main_idx);
    order.extend((0..n_funcs).filter(|i| *i != main_idx));

    // **collect** — exactly once per function; only the solution (expr / local
    // types) is recomputed across rounds, never the constraint tables.
    let mut states: Vec<FuncState> = module
        .functions
        .iter()
        .map(|f| FuncState::collect(f, n_globals, interner))
        .collect();

    // **solve** — chaotic iteration of the module variables (see the module
    // docs). Each round re-solves every DIRTY function FROM BOTTOM against the
    // current `vars`, then moves `global_ty` (right after main's sweep, so the
    // other functions already read this round's slot types) and `ret_ty`. The
    // loop ends the first round no variable moves; the final state is each
    // function's from-bottom solution at the converged variables, so transient
    // imprecision in an earlier round cannot survive into materialize.
    // Termination is by the WIDEN_LIMIT widening, not an iteration cap.
    //
    // Dirty-marking: a sweep is a deterministic function of the module
    // variables it reads (its `ReadSet`, recorded during the sweep), so a
    // function none of whose read variables moved would re-solve to the
    // identical state — skip it. Everything starts dirty.
    let mut dirty: Vec<bool> = vec![true; n_funcs];
    loop {
        let mut moved_rets = vec![false; n_funcs];
        let mut moved_globals = vec![false; n_globals];
        let mut moved_fields = vec![false; n_field_vars];
        for &idx in &order {
            if !dirty[idx] {
                continue;
            }
            states[idx].reset();
            Sweeper {
                st: &mut states[idx],
                func: &module.functions[idx],
                vars: &vars,
                resolve,
                closure_sigs: &closure_sigs,
                classes: &*classes,
                interner,
                global_demoted: &demoted,
                global_authoritative: &global_authoritative,
                field_vars: &field_vars,
                reads: RefCell::new(ReadSet::default()),
            }
            .solve();
            if idx == main_idx {
                move_global_tys(
                    &states[main_idx],
                    &demoted,
                    &global_authoritative,
                    &main_writes,
                    &mut vars,
                    &mut moved_globals,
                    classes,
                );
            }
        }
        // Move the `ret_ty` variables: join-with-old keeps the move monotone
        // even when a from-bottom re-solve transiently reports less. Only a
        // re-solved function can contribute a new return type.
        for (i, annotated) in ret_annotated.iter().enumerate() {
            if *annotated || !dirty[i] {
                continue;
            }
            let lifted = vars.ret_ty[i].join(
                &inferred_return_ty(&module.functions[i], &states[i], classes),
                classes,
            );
            if lifted != vars.ret_ty[i] {
                vars.ret_moves[i] += 1;
                // Widening: a return still climbing after WIDEN_LIMIT strict
                // moves is on an unbounded spine — go straight to `Dyn` (the
                // absorbing top, so the variable never moves again).
                vars.ret_ty[i] = if vars.ret_moves[i] >= WIDEN_LIMIT {
                    SemTy::Dyn
                } else {
                    lifted
                };
                moved_rets[i] = true;
            }
        }
        // Move the field variables (B10): a direct recompute from EVERY
        // function's writes (any function can write `obj.field`), after all
        // sweeps — like returns, so readers re-solve next round.
        move_field_tys(
            &states,
            classes,
            &field_vars,
            &field_var_names,
            &mut vars,
            &mut moved_fields,
        );
        if !moved_rets.contains(&true)
            && !moved_globals.contains(&true)
            && !moved_fields.contains(&true)
        {
            // Convergence — close the two `Never`-materializes-to-`Dyn` gaps
            // the per-round recompute cannot see (a still-`Never` expr keeps
            // its frontend type — `Dyn` — at materialize). Doing this earlier
            // would be premature: a transiently-`Never` receiver/value may
            // still climb. Each demotion pins, so this adds at most one extra
            // round per variable.
            //
            // 1. A `Never` RECEIVER lowers to a by-name write
            //    (`SetFieldNamed` can hit any class with that field name) —
            //    demote every same-named variable.
            // 2. A `Never` VALUE through a class receiver materializes `Dyn`
            //    into that one field — demote its variable (e.g. microgpt's
            //    `self.data = data` with an unannotated, never-written
            //    parameter, next to a float write of the same field).
            let demote_pin = |vi: usize, vars: &mut ModuleVars, moved: &mut [bool]| {
                if vars.field_pinned[vi] {
                    return;
                }
                if vars.field_ty[vi] != SemTy::Dyn {
                    moved[vi] = true;
                }
                vars.field_ty[vi] = SemTy::Dyn;
                vars.field_pinned[vi] = true;
            };
            for st in &states {
                for (base, name, value) in &st.attr_writes {
                    let bt = st.ety(*base);
                    if bt == SemTy::Never {
                        for (vi, vn) in field_var_names.iter().enumerate() {
                            if vn == name {
                                demote_pin(vi, &mut vars, &mut moved_fields);
                            }
                        }
                        continue;
                    }
                    if st.ety(*value) == SemTy::Never {
                        let Some(cid) = class_of(&bt, classes) else {
                            continue;
                        };
                        let Some(d) = defining_class(classes, cid, *name) else {
                            continue;
                        };
                        if let Some(&vi) = field_vars.get(&(d, *name)) {
                            demote_pin(vi, &mut vars, &mut moved_fields);
                        }
                    }
                }
            }
            if !moved_fields.contains(&true) {
                break;
            }
        }
        // Mark next round's dirty set: exactly the readers of what moved.
        // Globals move right after `__main__`'s sweep — the FIRST of the round
        // — so a function solved this round already read the post-move slot
        // types and needs no global-driven re-solve (`__main__` itself reads
        // its own variable slots through the live `global_writes` join, never
        // through `vars`). Returns and fields move after all sweeps, so their
        // readers re-solve unconditionally.
        for f in 0..n_funcs {
            let reads = &states[f].reads;
            dirty[f] = reads.rets.iter().any(|&r| moved_rets[r])
                || reads.fields.iter().any(|&v| moved_fields[v])
                || (!dirty[f] && reads.globals.iter().any(|&g| moved_globals[g]));
        }
    }

    // ── write the solved field types back into the ClassTable (B10) ──
    // A terminal forward step: downstream consumers (the SetAttr reinterpret
    // check below, lowering's field-read legalization, codegen) all read the
    // same solved types the final sweep used. Every flattened copy of a field
    // (the defining class AND each subclass layout) gets the type; `Never` (a
    // field never written with an evaluated value) maps to `Dyn` like any
    // genuinely-unconstrained slot.
    let field_updates: Vec<(ClassId, InternedString, SemTy)> = classes
        .iter()
        .flat_map(|info| {
            let cid = info.class_id;
            info.fields
                .iter()
                .filter_map(|f| {
                    let d = defining_class(classes, cid, f.name)?;
                    let &vi = field_vars.get(&(d, f.name))?;
                    let t = vars.field_ty[vi].clone();
                    Some((cid, f.name, if t == SemTy::Never { SemTy::Dyn } else { t }))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for (cid, name, ty) in field_updates {
        if let Some(info) = classes.get_mut(cid) {
            if let Some(f) = info.fields.iter_mut().find(|f| f.name == name) {
                f.ty = ty;
            }
        }
    }

    for (idx, st) in states.iter().enumerate() {
        materialize(&mut module.functions[idx], st, classes);
    }
    // Types are now materialized on every node; validate the unboxed-slot
    // boundaries before lowering can emit an unsound coercion.
    check_repr_boundaries(module, resolve, classes, interner)?;
    // Terminal, A3-safe range proof (Phase 3c, now whole-program): a forward
    // integer-interval analysis over the finalized types + CFG flags every `int`
    // slot, derived `int` BinOp, and — across direct call edges — the params and
    // return of a specializable function, when each provably stays within
    // `±RAW_I64_NARROW_BOUND` (→ `Raw(I64)` at lowering). It only writes the
    // `raw_int_ok` / `ret_raw_int` eligibility flags off a `ClassTable`-derived
    // address-taken gate; it never changes a `SemTy` or feeds back into inference.
    intervals::narrow_raw_ints(module, resolve, classes);
    Ok(())
}

/// The class that *defines* field `name` for `cid`: the LAST class in `cid`'s
/// MRO (self-first) whose layout contains the field. Fields are flattened
/// parent-first (semantics), so the most-base declarer owns the B10 inference
/// variable, and a write through a subclass receiver feeds it.
fn defining_class(classes: &ClassTable, cid: ClassId, name: InternedString) -> Option<ClassId> {
    let info = classes.get(cid)?;
    info.mro
        .iter()
        .rev()
        .find(|a| {
            classes
                .get(**a)
                .is_some_and(|ai| ai.field_slot(name).is_some())
        })
        .copied()
}

/// Move the field-type variables (B10): a DIRECT recompute of each variable —
/// the [`raw_uniform`]-guarded join of every `obj.field = value` write whose
/// receiver resolves to a class defining the field — never a join with the old
/// value (the guard is non-monotone, same reasoning as [`move_global_tys`]).
/// A write through a `Dyn`/`Union` receiver goes by NAME at runtime
/// (`SetFieldNamed` can hit any class with that field name), so it demotes
/// every same-named variable to `Dyn` and pins it. Oscillation is cut by the
/// per-variable move counter (widen to `Dyn` + pin at [`WIDEN_LIMIT`]).
fn move_field_tys(
    states: &[FuncState],
    classes: &ClassTable,
    field_vars: &HashMap<(ClassId, InternedString), usize>,
    field_var_names: &[InternedString],
    vars: &mut ModuleVars,
    moved: &mut [bool],
) {
    let nvars = field_var_names.len();
    let mut contribs: Vec<Vec<SemTy>> = vec![Vec::new(); nvars];
    let mut demote: Vec<bool> = vec![false; nvars];
    for st in states {
        for (base, name, value) in &st.attr_writes {
            let bt = st.ety(*base);
            if bt == SemTy::Never {
                continue; // receiver not evaluated yet — contributes nothing
            }
            if matches!(bt, SemTy::Dyn | SemTy::Union(_)) {
                for (vi, vn) in field_var_names.iter().enumerate() {
                    if vn == name {
                        demote[vi] = true;
                    }
                }
                continue;
            }
            let Some(cid) = class_of(&bt, classes) else {
                continue;
            };
            let Some(d) = defining_class(classes, cid, *name) else {
                continue;
            };
            let Some(&vi) = field_vars.get(&(d, *name)) else {
                continue;
            };
            let vt = erase_vars(&st.ety(*value), classes);
            if vt == SemTy::Never {
                continue;
            }
            contribs[vi].push(vt);
        }
    }
    for vi in 0..nvars {
        if vars.field_pinned[vi] {
            continue;
        }
        if demote[vi] {
            if vars.field_ty[vi] != SemTy::Dyn {
                moved[vi] = true;
            }
            vars.field_ty[vi] = SemTy::Dyn;
            vars.field_pinned[vi] = true;
            continue;
        }
        let tys = &contribs[vi];
        let joined = raw_uniform(
            tys.iter().fold(SemTy::Never, |acc, t| acc.join(t, classes)),
            tys,
        );
        if joined == SemTy::Never {
            continue; // no write evaluated yet — the variable stays at bottom
        }
        if joined != vars.field_ty[vi] {
            vars.field_moves[vi] += 1;
            if vars.field_moves[vi] >= WIDEN_LIMIT {
                vars.field_ty[vi] = SemTy::Dyn;
                vars.field_pinned[vi] = true;
            } else {
                vars.field_ty[vi] = joined;
            }
            moved[vi] = true;
        }
    }
}

/// Move the `global_ty` variables from `__main__`'s freshly-solved writes: a
/// DIRECT recompute of each slot — the join of its write types with the same
/// `Raw`-uniformity guard as locals ([`FuncState::join_writes`]) — never a join
/// with the old value. The guard is non-monotone (`{Int, Float} → Dyn` but
/// `{Float, Float} → Float`), so joining with the old value would let a
/// transient `Dyn` stick forever — and a sticky `Dyn` is not just lost
/// precision: it flows into `check_reinterpret`'s Strict slots and would reject
/// programs that compile today. The direct recompute may therefore oscillate;
/// the per-slot move counter widens a still-moving slot to `Dyn` and PINS it
/// (excluded from further recomputes), restoring guaranteed termination.
///
/// Annotated slots keep their declared type (the Phase-8 contract); demoted
/// slots stay `Dyn` (Phase 6B) — both are constants, never moved. Each slot
/// that moved is marked in `moved` (feeding the dirty-marking).
fn move_global_tys(
    main_st: &FuncState,
    demoted: &[bool],
    authoritative: &[bool],
    main_writes: &[bool],
    vars: &mut ModuleVars,
    moved: &mut [bool],
    classes: &ClassTable,
) {
    for vid in 0..vars.global_ty.len() {
        if vars.global_pinned[vid] || demoted[vid] || authoritative[vid] || !main_writes[vid] {
            continue;
        }
        let joined = erase_vars(
            &main_st.join_writes(&main_st.global_writes[vid], classes),
            classes,
        );
        if joined == SemTy::Never {
            continue; // no write evaluated yet — the slot stays at bottom
        }
        if joined != vars.global_ty[vid] {
            vars.global_moves[vid] += 1;
            if vars.global_moves[vid] >= WIDEN_LIMIT {
                vars.global_ty[vid] = SemTy::Dyn;
                vars.global_pinned[vid] = true;
            } else {
                vars.global_ty[vid] = joined;
            }
            moved[vid] = true;
        }
    }
}

/// The Raw-uniformity guard ("stay Tagged when in doubt", PITFALLS A2/B6),
/// shared by every join that decides a slot representation — locals
/// ([`FuncState::join_writes`]), container element slots (literals and
/// element-level writes), and inferred returns: if the joined type would take
/// a `Raw` representation, every contributor must *already* have that
/// representation; otherwise the slot falls back to `Dyn` (→ `Tagged`).
/// Without this, a numerically-promoted contributor (a tagged int feeding a
/// `Float` slot) would be silently unboxed. A still-`Never` contributor (not
/// yet evaluated this sweep) adds nothing to the join, so it must not
/// spuriously block the narrowing and force a sticky `Dyn`.
fn raw_uniform(joined: SemTy, contribs: &[SemTy]) -> SemTy {
    if joined == SemTy::Never {
        return SemTy::Never;
    }
    if matches!(repr_of(&joined), Repr::Raw(_)) {
        let target = repr_of(&joined);
        let uniform = contribs
            .iter()
            .all(|t| *t == SemTy::Never || repr_of(t) == target);
        if !uniform {
            return SemTy::Dyn;
        }
    }
    joined
}

/// The inferred return type of a function (Phase 8E): the join of every
/// `return <v>` expression's solved type, with a value-less `return` / fall-off
/// (`Return(None)`) contributing `NoneTy`. `Never` (no contributors yet) means
/// "not known", and `join` treats it as the identity, so it never poisons a
/// caller — the round loop lifts it as the body's expr types settle. The
/// result is [`raw_uniform`]-guarded: a function returning both `2.25` and
/// `16` must NOT get a `Raw(F64)` return ABI (the int return would be
/// blindly unboxed at the boundary) — it demotes to `Dyn`.
fn inferred_return_ty(func: &HirFunction, st: &FuncState, classes: &ClassTable) -> SemTy {
    let mut contribs: Vec<SemTy> = Vec::new();
    for (_b, block) in func.blocks.iter() {
        match &block.term {
            HirTerminator::Return(Some(v)) => contribs.push(erase_vars(&st.ety(*v), classes)),
            HirTerminator::Return(None) => contribs.push(SemTy::NoneTy),
            _ => {}
        }
    }
    let joined = contribs
        .iter()
        .fold(SemTy::Never, |acc, t| acc.join(t, classes));
    raw_uniform(joined, &contribs)
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
/// to `Tagged` when a call site proves it polymorphic — PITFALLS B10. The
/// FIELD half of B10 is now solved: unannotated instance fields are inference
/// variables joined over every module-wide write ([`move_field_tys`]).
/// Still deferred: inferring unannotated `__init__` PARAMETER types from call
/// sites — that is parameter inference, a separate feature; a field fed by a
/// `Dyn` parameter simply stays `Dyn`, which is safe.)
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
                                &func.exprs[*value],
                                target_ty,
                                kind,
                                "assigned to",
                                classes,
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
                                &func.exprs[*value],
                                &local.ty,
                                kind,
                                "assigned to",
                                classes,
                            )?;
                        }
                    }
                    // Writes into a module-level annotated global slot (Phase 8):
                    // the annotation is a contract, so a `GlobalSet` of a
                    // mismatched type is rejected like any other reinterpret seam.
                    // (Globals are physically tagged, but a `GlobalGet` reinterprets
                    // by the annotated type, so a wrong write would later misread.)
                    HirStmt::GlobalSet { var_id, value } => {
                        if let Some(slot_ty) = module.global_annotations.get(var_id) {
                            if let Some(kind) = reinterpret_kind(slot_ty) {
                                check_reinterpret(
                                    &func.exprs[*value],
                                    slot_ty,
                                    kind,
                                    "assigned to global",
                                    classes,
                                )?;
                            }
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
                                            &func.exprs[*value],
                                            field_ty,
                                            kind,
                                            "assigned to field",
                                            classes,
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
                    check_reinterpret(
                        &func.exprs[*v],
                        &func.ret_ty,
                        kind,
                        "returned from",
                        classes,
                    )?;
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
                HirExprKind::MethodCall {
                    recv,
                    method_name,
                    args,
                    kwargs,
                } => match method_call_target(func, resolve, classes, *recv, *method_name) {
                    Some((fid, skip_self)) => {
                        let callee = &module.functions[fid.index()];
                        let p = &callee.params[..];
                        let p = if skip_self { &p[1.min(p.len())..] } else { p };
                        let cut =
                            p.len() - usize::from(callee.varargs) - usize::from(callee.kwargs);
                        let p = &p[..cut.min(p.len())];
                        // Keyword args pair with their NAMED parameter (Phase
                        // 10); names without a fixed param feed the `**kwargs`
                        // dict (uniform tagged — no repr boundary to check).
                        for (kname, kexpr) in kwargs {
                            if let Some(param) = p.iter().find(|pp| pp.name == *kname) {
                                if let Some(kind) = reinterpret_kind(&param.ty) {
                                    check_reinterpret(
                                        &func.exprs[*kexpr],
                                        &param.ty,
                                        kind,
                                        "passed to",
                                        classes,
                                    )?;
                                }
                            }
                        }
                        (p, args)
                    }
                    None => continue,
                },
                // `Cls[T](args)` → `__init__(self, args…)`: skip `self`.
                HirExprKind::GenericConstruct { class_id, args, .. } => {
                    match init_params(classes, module, interner, *class_id) {
                        Some(p) => (&p[1.min(p.len())..], args),
                        None => continue,
                    }
                }
                // A stdlib runtime call (Phase 8B): every provided arg must
                // satisfy its declarative param `TypeSpec`. Raw-register params
                // (NO_AUTO_BOX float/int/bool) reject gradual values loudly —
                // lowering's `Tagged → Raw` coercion would mis-read a
                // mismatched `Value` (PITFALLS A2).
                HirExprKind::CallRuntime { target, args, .. } => {
                    check_call_runtime_args(func, target, args)?;
                    continue;
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

/// True iff `t` is a Union with at least one numeric member — admissible at a
/// checked raw-ABI boundary (Phase 8H, D3).
fn union_has_numeric(t: &SemTy) -> bool {
    match t {
        SemTy::Union(ms) => ms
            .iter()
            .any(|m| matches!(m, SemTy::Float | SemTy::Int | SemTy::Bool)),
        _ => false,
    }
}

/// Validate a stdlib runtime call's args against the descriptor's declarative
/// param `TypeSpec`s (Phase 8B). The frontend's call adaptation has already
/// aligned `args` positionally with the descriptor params (filling defaults);
/// absent optional slots (`None`) lower to the null-pointer sentinel and need
/// no check. The rule per param spec:
/// * `Float` / `Int` (raw ABI slots) admit the numeric types, gradual `Dyn`,
///   and numeric-containing Unions — lowering emits a CHECKED unbox for the
///   gradual cases (Phase 8H, D3); `Bool` / `Str` require the exact semantic
///   type (`Bool` is admitted where `Int` is expected, matching Python's
///   bool ⊂ int).
/// * Object specs (`StructTime`, `Match`, …) require the matching
///   `RuntimeObject` (or gradual `Dyn`, carried Tagged).
/// * `Any` / `Optional` / containers are uniform tagged storage — any type.
fn check_call_runtime_args(
    func: &HirFunction,
    target: &pyaot_hir::RuntimeCallTarget,
    args: &[Option<Idx<HirExpr>>],
) -> Result<()> {
    use pyaot_stdlib_defs::TypeSpec;
    let params: &[pyaot_stdlib_defs::ParamDef] = match target {
        // A `variadic_to_list` call (`os.path.join`) passes ONE compiler-built
        // list (not per-element values), so element typing does not apply.
        pyaot_hir::RuntimeCallTarget::Func(f) if f.hints.variadic_to_list => return Ok(()),
        pyaot_hir::RuntimeCallTarget::Func(f) => f.params,
        // Attr getters take no Python-level args; Field receivers are checked
        // by the attribute-typing path that produced them.
        pyaot_hir::RuntimeCallTarget::Attr(_) | pyaot_hir::RuntimeCallTarget::Field(_) => &[],
    };
    for (slot, param) in args.iter().zip(params) {
        let Some(arg_idx) = slot else { continue };
        let got = &func.exprs[*arg_idx].ty;
        if matches!(got, SemTy::Never) {
            continue;
        }
        let ok = match &param.ty {
            // Float/Int raw-ABI params admit gradual (`Dyn`), numeric, and
            // numeric-containing-Union arguments (Phase 8H, D3): lowering
            // emits a CHECKED unbox (`rt_unbox_float`/`rt_unbox_int` —
            // TypeError on a bad tag), so the seam stays safe without an
            // annotation. A union with NO numeric member stays a loud
            // compile-time error.
            TypeSpec::Float => {
                matches!(got, SemTy::Float | SemTy::Int | SemTy::Bool | SemTy::Dyn)
                    || union_has_numeric(got)
            }
            TypeSpec::Int => {
                matches!(got, SemTy::Int | SemTy::Bool | SemTy::Dyn) || union_has_numeric(got)
            }
            TypeSpec::Bool => matches!(got, SemTy::Bool),
            TypeSpec::Str => matches!(got, SemTy::Str),
            spec => {
                let want = pyaot_hir::semty_from_typespec(spec);
                match want {
                    SemTy::Dyn => true,
                    SemTy::RuntimeObject(tag) => {
                        matches!(got, SemTy::RuntimeObject(t) if *t == tag)
                            || matches!(got, SemTy::Dyn)
                    }
                    // Container params travel Tagged — gradual by design.
                    _ => true,
                }
            }
        };
        if !ok {
            let symbol = target.codegen().symbol;
            return Err(CompilerError::type_error(
                format!(
                    "stdlib call `{symbol}`: parameter `{}` expects {:?} but the \
                     argument is `{got:?}` (annotate the value — implicit \
                     conversion at a raw-ABI boundary is not performed)",
                    param.name, param.ty,
                ),
                func.exprs[*arg_idx].span,
            ));
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
    let arity_ok = if sig.varargs {
        args.len() >= fixed
    } else {
        args.len() == fixed
    };
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
    let m = info
        .methods
        .iter()
        .find(|m| interner.resolve(m.name) == "__init__")?;
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
        return classes
            .resolve_super_method(cid, method_name)
            .map(|f| (f, true));
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
    if let Some(m) = info
        .static_method(method_name)
        .or_else(|| info.class_method(method_name))
    {
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
                || value.ty.is_subtype_of(target, classes)
                || (kind == ReinterpretKind::Gradual && value.ty == SemTy::Dyn)
        }
    };
    if ok {
        return Ok(());
    }
    let detail = match kind {
        ReinterpretKind::Strict => {
            "this compiler unboxes annotated `float`/`bool` slots, so a mismatched \
             value would be misread. Pass a matching type, e.g. `3.0` instead of `3`."
        }
        ReinterpretKind::Gradual => {
            "this compiler stores annotated container/`str`/`bytes` slots as typed \
             heap pointers, so a mismatched value would be reinterpreted as one and \
             crash at the first operation on it. Pass a matching type."
        }
        ReinterpretKind::Closure => {
            "this compiler compiles a `Callable[...]` slot to that exact native \
             call signature, so the stored function's parameter/return types must \
             match the annotation exactly."
        }
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
        SemTy::Callable(_) => "Callable",
        _ if ty.list_elem().is_some() => "list",
        _ if ty.dict_kv().is_some() => "dict",
        _ if ty.set_elem().is_some() => "set",
        _ if ty.tuple_elems().is_some() || ty.tuple_var_elem().is_some() => "tuple",
        _ => "<other>",
    }
}

/// The module-level variables of the constraint system, shared (read-only) by
/// every function's sweep: inferred return types and promoted-global slot
/// types. Annotated returns, annotated (authoritative) globals, and demoted
/// globals are constants — initialized once in [`infer`] and never moved.
struct ModuleVars {
    /// `ret_ty[fid]` — the declared return if annotated, else the climbing
    /// join of the function's solved `return` types (Phase 8E).
    ret_ty: Vec<SemTy>,
    /// `global_ty[vid]` — the slot's annotation (authoritative), `Dyn`
    /// (demoted / never written by main), or the recomputed join of
    /// `__main__`'s writes, starting at `Never` (Phase 6B / 8).
    global_ty: Vec<SemTy>,
    /// `field_ty[vi]` — an unannotated, non-generic instance field's type
    /// (B10): the recomputed join of every `obj.field = value` write across
    /// the module, starting at `Never`. Indexed by the variable table built
    /// in [`infer`] (one variable per defining class + field name).
    field_ty: Vec<SemTy>,
    /// Strict-move counters driving the [`WIDEN_LIMIT`] widening.
    ret_moves: Vec<usize>,
    global_moves: Vec<usize>,
    field_moves: Vec<usize>,
    /// Globals widened to `Dyn` — excluded from further recomputes (the direct
    /// recompute in [`move_global_tys`] is non-monotone, so without the pin it
    /// could oscillate forever).
    global_pinned: Vec<bool>,
    /// Field variables widened/demoted to `Dyn` — same pinning rationale as
    /// globals, plus the `Dyn`-receiver demotion (see [`move_field_tys`]).
    field_pinned: Vec<bool>,
}

/// The module variables one sweep actually READ: which `ret_ty[fid]` /
/// `global_ty[vid]` entries flowed into the solution. A sweep is a
/// deterministic function of these reads, so a function whose read variables
/// did not move needs no re-solve — it would reproduce the same solution and
/// the same read-set. Drives the dirty-marking in [`infer`].
#[derive(Default)]
struct ReadSet {
    rets: HashSet<usize>,
    globals: HashSet<usize>,
    /// Field variables read through `attribute_ty` (B10).
    fields: HashSet<usize>,
}

/// One write into a local slot (Phase 8H, D1). Beyond plain assignments, the
/// element-level container writes contribute to an inferred local's type:
/// `xs.append(v)` / comprehension pushes constrain `xs` to `list[type(v)]`.
#[derive(Clone, Copy)]
enum Write {
    /// `x = value` — a whole-value assignment.
    Val(Idx<HirExpr>),
    /// `x.append(v)` / `x.add(v)` / `x.insert(i, v)` / comprehension
    /// `ContainerPush` — one element pushed into the container held by `x`.
    Push(Idx<HirExpr>),
    /// `x.extend(it)` — every element of an iterable pushed.
    PushIter(Idx<HirExpr>),
    /// `x[k] = v` / dict-comprehension `ContainerInsert` — a keyed write.
    Insert(Idx<HirExpr>, Idx<HirExpr>),
}

/// Per-function constraint state. Built ONCE by [`FuncState::collect`]; the
/// constraint tables (`authoritative` / `assignments` / `cell_writes` /
/// `global_writes`) are immutable afterwards, while the solution (`expr_ty` /
/// `local_ty`) is [`FuncState::reset`] to bottom and re-solved each round.
struct FuncState {
    /// `true` for locals whose frontend type is authoritative (a parameter or an
    /// explicit annotation): their type is fixed and never inferred.
    authoritative: Vec<bool>,
    /// The solution bottom per local: the declared type for authoritative
    /// locals, `Never` otherwise — what `reset` restores before a re-solve.
    local_bottom: Vec<SemTy>,
    /// Writes into each local, indexed by `LocalId`: whole-value assignments
    /// plus element-level container writes (Phase 8H, D1).
    assignments: Vec<Vec<Write>>,
    /// Value expressions written into the cell held by each local (`CellSet`
    /// plus the `MakeCell` init), indexed by the cell local's `LocalId` — the
    /// per-cell constraint of Phase 6A (the B10 field-join shape).
    cell_writes: Vec<Vec<Idx<HirExpr>>>,
    /// This function's own `GlobalSet` writes per slot — only `__main__` has
    /// any when the slot is not demoted, making its reads a live worklist join
    /// (and feeding the module-level `global_ty` recompute).
    global_writes: Vec<Vec<Idx<HirExpr>>>,
    /// Every `SetAttr` in this function — `(base, field name, value)` — the
    /// raw material of the B10 field-variable recompute ([`move_field_tys`]).
    /// Augmented writes (`x.f += v`) are already desugared to `SetAttr` by the
    /// frontend, so they contribute for free.
    attr_writes: Vec<(Idx<HirExpr>, InternedString, Idx<HirExpr>)>,
    /// Current per-expr type (absent = `Never`, the lattice bottom).
    expr_ty: HashMap<Idx<HirExpr>, SemTy>,
    /// Current per-local type.
    local_ty: Vec<SemTy>,
    /// The module variables this function's LAST sweep read (its dynamic
    /// dependency set) — kept across rounds so `infer`'s dirty-marking can
    /// consult it even on rounds that skip the function.
    reads: ReadSet,
}

impl FuncState {
    /// **collect** — seed the assignment tables and the authoritative-local set.
    fn collect(func: &HirFunction, n_globals: usize, interner: &StringInterner) -> Self {
        let n = func.locals.len();
        // A frontend type other than `Dyn` is authoritative: it comes from a
        // parameter annotation, a `name: T` annotation, or a synthetic local the
        // frontend deliberately typed (e.g. `__name__: str`, chained-compare
        // results). Plain `x = ...` locals are `Dyn` and get inferred.
        let authoritative: Vec<bool> = func.locals.iter().map(|l| l.ty != SemTy::Dyn).collect();
        let local_bottom: Vec<SemTy> = func
            .locals
            .iter()
            .enumerate()
            .map(|(i, l)| {
                if authoritative[i] {
                    l.ty.clone()
                } else {
                    SemTy::Never
                }
            })
            .collect();

        // A direct read of a local — the only receiver shape whose container
        // writes can soundly be attributed back to the slot (an alias through
        // another local contributes to THAT local instead).
        let direct_local = |e: Idx<HirExpr>| -> Option<usize> {
            match func.exprs[e].kind {
                HirExprKind::Local(lid) => Some(lid.index()),
                _ => None,
            }
        };

        let mut assignments: Vec<Vec<Write>> = vec![Vec::new(); n];
        let mut cell_writes: Vec<Vec<Idx<HirExpr>>> = vec![Vec::new(); n];
        let mut global_writes: Vec<Vec<Idx<HirExpr>>> = vec![Vec::new(); n_globals];
        let mut attr_writes: Vec<(Idx<HirExpr>, InternedString, Idx<HirExpr>)> = Vec::new();
        for (_bidx, block) in func.blocks.iter() {
            for stmt in &block.stmts {
                match stmt {
                    // ── field writes (B10): feed the module-level field vars ──
                    HirStmt::SetAttr { base, name, value } => {
                        attr_writes.push((*base, *name, *value));
                    }
                    HirStmt::Assign { target, value } => {
                        assignments[target.index()].push(Write::Val(*value));
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
                    // ── element-level container writes (Phase 8H, D1) ──
                    HirStmt::ContainerPush { container, value } => {
                        assignments[container.index()].push(Write::Push(*value));
                    }
                    HirStmt::ContainerInsert {
                        container,
                        key,
                        value,
                    } => {
                        assignments[container.index()].push(Write::Insert(*key, *value));
                    }
                    HirStmt::SetItem { base, index, value } => {
                        if let Some(i) = direct_local(*base) {
                            assignments[i].push(Write::Insert(*index, *value));
                        }
                    }
                    // `xs.append(v)` / `s.add(v)` / `xs.insert(i, v)` /
                    // `xs.extend(it)` as a statement-expression on a direct local.
                    HirStmt::Expr(e) => {
                        if let HirExprKind::MethodCall {
                            recv,
                            method_name,
                            args,
                            kwargs: _,
                        } = &func.exprs[*e].kind
                        {
                            if let Some(i) = direct_local(*recv) {
                                match (interner.resolve(*method_name), args.as_slice()) {
                                    ("append" | "add", [v]) => {
                                        assignments[i].push(Write::Push(*v));
                                    }
                                    ("insert", [_, v]) => {
                                        assignments[i].push(Write::Push(*v));
                                    }
                                    ("extend", [it]) => {
                                        assignments[i].push(Write::PushIter(*it));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let local_ty = local_bottom.clone();
        FuncState {
            authoritative,
            local_bottom,
            assignments,
            cell_writes,
            global_writes,
            attr_writes,
            expr_ty: HashMap::new(),
            local_ty,
            reads: ReadSet::default(),
        }
    }

    /// Reset the solution to bottom for a fresh from-bottom re-solve (the
    /// constraint tables are untouched — see the module docs for why resuming
    /// the previous round's state would be unsound).
    fn reset(&mut self) {
        self.expr_ty.clear();
        self.local_ty.clone_from(&self.local_bottom);
    }

    /// The current type of an expr (bottom = `Never` if not yet computed).
    fn ety(&self, idx: Idx<HirExpr>) -> SemTy {
        self.expr_ty.get(&idx).cloned().unwrap_or(SemTy::Never)
    }

    /// Join the types of a set of written values with the `Raw`-repr soundness
    /// guard. Shared by inferred locals, cell contents (Phase 6A), and global
    /// slots (Phase 6B).
    ///
    /// `Never` is the in-progress bottom: a slot still being computed (or one
    /// only fed by not-yet-evaluated values across a loop back-edge) must stay
    /// `Never`, never jump to a spurious `Dyn`. `join` treats `Never` as the
    /// identity, so a bottom contributor is correctly ignored by dependents;
    /// an injected `Dyn` would instead absorb and poison them irreversibly.
    /// Genuinely-unconstrained slots are mapped to `Dyn` once, in materialize.
    fn join_writes(&self, writes: &[Idx<HirExpr>], classes: &ClassTable) -> SemTy {
        // The Raw-uniformity discipline (PITFALLS A2/B6) lives in
        // [`raw_uniform`], shared with element-slot and return-type joins.
        let tys: Vec<SemTy> = writes.iter().map(|&v| self.ety(v)).collect();
        let joined = tys.iter().fold(SemTy::Never, |acc, t| acc.join(t, classes));
        raw_uniform(joined, &tys)
    }

    /// Join a local's writes — whole-value assignments plus the element-level
    /// container writes (Phase 8H, D1). The base comes from the `Val` writes
    /// (with the Raw-uniformity guard); each push then contributes to the
    /// matching element slot in the base's container family. A `Never` base
    /// (seed literal not evaluated yet) stays bottom — monotone; a `Dyn` base
    /// absorbs the pushes outright. Each element slot is [`raw_uniform`]-
    /// guarded independently: `xs = [2.25]; xs.append(16)` must not leave a
    /// `Raw(F64)` element slot holding a tagged int — the slot demotes to
    /// `Dyn` (elements stay `Tagged`).
    fn join_local_writes(&self, writes: &[Write], classes: &ClassTable) -> SemTy {
        let vals: Vec<Idx<HirExpr>> = writes
            .iter()
            .filter_map(|w| match w {
                Write::Val(v) => Some(*v),
                _ => None,
            })
            .collect();
        let base = self.join_writes(&vals, classes);
        if base == SemTy::Never || base == SemTy::Dyn {
            return base;
        }
        let is_list = base.list_elem().is_some();
        let is_set = base.set_elem().is_some();
        let is_dict = base.dict_kv().is_some();
        // A non-container base ignores the element writes outright: a `.add()`
        // / `.append()` on a class instance is a USER method, and a `[k] = v`
        // on it is `__setitem__` — neither says anything about the slot type.
        if !is_list && !is_set && !is_dict {
            return base;
        }
        // Per-slot contributor lists: the base's own element slot(s) plus each
        // element-level write that lands in that slot.
        let mut elems: Vec<SemTy> = Vec::new();
        let mut keys: Vec<SemTy> = Vec::new();
        let mut dvals: Vec<SemTy> = Vec::new();
        if is_list {
            elems.push(base.list_elem().expect("is_list").clone());
        } else if is_set {
            elems.push(base.set_elem().expect("is_set").clone());
        } else {
            let (k, v) = base.dict_kv().expect("is_dict");
            keys.push(k.clone());
            dvals.push(v.clone());
        }
        for w in writes {
            match w {
                Write::Val(_) => {}
                Write::Push(v) => {
                    if is_list || is_set {
                        elems.push(self.ety(*v));
                    }
                    // dict has no push
                }
                Write::PushIter(it) => {
                    if is_list || is_set {
                        elems.push(iter_elem_ty(&self.ety(*it), classes));
                    }
                }
                Write::Insert(k, v) => {
                    if is_dict {
                        keys.push(self.ety(*k));
                        dvals.push(self.ety(*v));
                    } else if is_list {
                        // `xs[i] = v` constrains the element type only.
                        elems.push(self.ety(*v));
                    }
                }
            }
        }
        let join_slot = |tys: &[SemTy]| {
            let joined = tys.iter().fold(SemTy::Never, |acc, t| acc.join(t, classes));
            raw_uniform(joined, tys)
        };
        if is_list {
            SemTy::list_of(join_slot(&elems))
        } else if is_set {
            SemTy::set_of(join_slot(&elems))
        } else {
            SemTy::dict_of(join_slot(&keys), join_slot(&dvals))
        }
    }
}

/// One per-function monotone worklist sweep over the [`TypeLattice`]: a
/// borrowed view tying a function's [`FuncState`] to the current (read-only)
/// [`ModuleVars`] for the duration of one from-bottom re-solve — so the
/// module variables thread through the eval helpers without widening every
/// signature.
struct Sweeper<'a> {
    st: &'a mut FuncState,
    func: &'a HirFunction,
    /// The module-level variables, frozen for this sweep.
    vars: &'a ModuleVars,
    resolve: &'a ResolveResult,
    /// Each function's visible Callable signature (params minus env) — the
    /// type a `MakeClosure` over it produces (Phase 6A).
    closure_sigs: &'a [Sig],
    classes: &'a ClassTable,
    interner: &'a StringInterner,
    /// Slots written outside `__main__` — always `Dyn` (Phase 6B).
    global_demoted: &'a [bool],
    /// Slots with a module-level annotation — their declared type is
    /// authoritative and overrides demotion (Phase 8).
    global_authoritative: &'a [bool],
    /// The B10 field-variable table: `(defining class, field name)` → index
    /// into [`ModuleVars::field_ty`]. Absent pairs (annotated / generic-class
    /// fields) keep the static `ClassInfo::field_ty` path.
    field_vars: &'a HashMap<(ClassId, InternedString), usize>,
    /// Module variables read so far in this sweep (interior mutability — the
    /// recording happens inside `&self` eval helpers). Moved into
    /// [`FuncState::reads`] when the sweep finishes.
    reads: RefCell<ReadSet>,
}

impl<'a> Sweeper<'a> {
    /// **solve** — iterate the monotone worklist to a fixpoint in `self.st`.
    fn solve(self) {
        // Gauss-Seidel sweeps: recompute every expr type, then every inferred
        // local type, until a full sweep changes nothing. Recomputations climb
        // the lattice (`Dyn` is the absorbing top), but the container spine is
        // infinitely tall, so a self-recursive constraint (`x = [x]`,
        // re-joined through a local / cell / global slot every iteration)
        // would climb forever — an expr still moving after WIDEN_LIMIT moves
        // is widened to `Dyn` and pinned for the rest of this solve. Locals
        // need no counters of their own: a local is a pure join of expr types
        // (`join_writes`), so once every expr is stable — converged or pinned
        // — the locals converge right after, bounding the whole sweep.
        let expr_indices: Vec<Idx<HirExpr>> = self.func.exprs.iter().map(|(i, _)| i).collect();
        let mut expr_moves: HashMap<Idx<HirExpr>, usize> = HashMap::new();
        loop {
            let mut changed = false;
            for idx in &expr_indices {
                let m = expr_moves.get(idx).copied().unwrap_or(0);
                if m >= WIDEN_LIMIT {
                    continue; // widened to `Dyn` — pinned for this solve
                }
                let new = self.eval_expr(*idx);
                if self.st.expr_ty.get(idx) != Some(&new) {
                    let m = m + 1;
                    expr_moves.insert(*idx, m);
                    let new = if m >= WIDEN_LIMIT { SemTy::Dyn } else { new };
                    self.st.expr_ty.insert(*idx, new);
                    changed = true;
                }
            }
            for i in 0..self.st.local_ty.len() {
                if self.st.authoritative[i] {
                    continue;
                }
                // Recompute the local from its assigned values, applying the
                // `Raw`-repr soundness guard (see the module docs).
                let new = self
                    .st
                    .join_local_writes(&self.st.assignments[i], self.classes);
                if self.st.local_ty[i] != new {
                    self.st.local_ty[i] = new;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Persist this sweep's dynamic dependency set for the dirty-marking.
        self.st.reads = self.reads.into_inner();
    }

    /// The current type of an expr (bottom = `Never` if not yet computed).
    fn ety(&self, idx: Idx<HirExpr>) -> SemTy {
        self.st.ety(idx)
    }

    /// A callee's current `ret_ty` module variable, recording the read in the
    /// sweep's [`ReadSet`].
    fn callee_ret_ty(&self, fid: pyaot_utils::FuncId) -> SemTy {
        self.reads.borrow_mut().rets.insert(fid.index());
        self.vars.ret_ty[fid.index()].clone()
    }

    /// A global slot's current `global_ty` module variable, recording the read
    /// in the sweep's [`ReadSet`].
    fn global_slot_ty(&self, vid: usize) -> SemTy {
        self.reads.borrow_mut().globals.insert(vid);
        self.vars.global_ty.get(vid).cloned().unwrap_or(SemTy::Dyn)
    }

    /// A field's current `field_ty` module variable (B10), recording the read
    /// in the sweep's [`ReadSet`].
    fn field_var_ty(&self, vi: usize) -> SemTy {
        self.reads.borrow_mut().fields.insert(vi);
        self.vars.field_ty[vi].clone()
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
            HirExprKind::Local(lid) => self.st.local_ty[lid.index()].clone(),
            HirExprKind::Name(symref) => self.name_ty(*symref),
            HirExprKind::Unary { op, operand } => self.unary_ty(*op, self.ety(*operand)),
            HirExprKind::BinOp { op, l, r } => self.binop_ty(*op, self.ety(*l), self.ety(*r)),
            HirExprKind::Call { callee, args } => self.call_ty(*callee, args),
            // ── containers (Phase 4) ──
            // `sum(iterable[, start])` (Phase 8H, D2). Class elements with
            // `__add__`/`__radd__` take the join of the inferred dunder returns
            // (rides the ret_ty fixpoint via `callee_ret_ty`); numeric elements
            // promote (any Float contributor → Float, else Int); anything else
            // is gradual `Dyn`. `Never` propagates while operands are unsolved.
            HirExprKind::Sum { iterable, start } => {
                let it = self.ety(*iterable);
                if it == SemTy::Never {
                    return SemTy::Never;
                }
                let elem = iter_elem_ty(&it, self.classes);
                let start_ty = match start {
                    Some(s) => self.ety(*s),
                    None => SemTy::Int,
                };
                if elem == SemTy::Never || start_ty == SemTy::Never {
                    return SemTy::Never;
                }
                if class_of(&elem, self.classes).is_some() {
                    let mut joined = SemTy::Never;
                    for d in ["__add__", "__radd__"] {
                        if let Some(t) = self.class_dunder_ret(&elem, d) {
                            joined = joined.join(&t, self.classes);
                        }
                    }
                    return if joined == SemTy::Never {
                        SemTy::Dyn
                    } else {
                        joined
                    };
                }
                let numeric = |t: &SemTy| matches!(t, SemTy::Int | SemTy::Float | SemTy::Bool);
                if numeric(&elem) && numeric(&start_ty) {
                    if elem == SemTy::Float || start_ty == SemTy::Float {
                        SemTy::Float
                    } else {
                        SemTy::Int
                    }
                } else {
                    SemTy::Dyn
                }
            }
            HirExprKind::ListLit { elems } => SemTy::list_of(self.join_all(elems)),
            HirExprKind::SetLit { elems } => SemTy::set_of(self.join_all(elems)),
            HirExprKind::TupleLit { elems } => {
                SemTy::tuple_of(elems.iter().map(|e| self.ety(*e)).collect())
            }
            HirExprKind::DictLit { pairs } => {
                // Key/value slots take the same Raw-uniformity guard as list
                // elements — stored values keep their own representation.
                let kt: Vec<SemTy> = pairs.iter().map(|(k, _)| self.ety(*k)).collect();
                let vt: Vec<SemTy> = pairs.iter().map(|(_, v)| self.ety(*v)).collect();
                let k = raw_uniform(
                    kt.iter()
                        .fold(SemTy::Never, |acc, t| acc.join(t, self.classes)),
                    &kt,
                );
                let v = raw_uniform(
                    vt.iter()
                        .fold(SemTy::Never, |acc, t| acc.join(t, self.classes)),
                    &vt,
                );
                SemTy::dict_of(k, v)
            }
            HirExprKind::BytesLit(_) => SemTy::Bytes,
            HirExprKind::Subscript { base, index } => self.subscript_ty(*base, *index),
            HirExprKind::Slice { base, .. } => self.slice_ty(*base),
            HirExprKind::FormatValue { .. } => SemTy::Str,
            HirExprKind::ContainerExpr { op, args } => self.container_op_ty(*op, args),
            HirExprKind::MethodCall {
                recv, method_name, ..
            } => {
                // `super().m()` resolves against the enclosing class's MRO; a
                // `ClassName.m()` static/classmethod resolves on the class; an
                // ordinary receiver dispatches by its static type.
                if let HirExprKind::Super(cid) = self.func.exprs[*recv].kind {
                    self.classes
                        .resolve_super_method(cid, *method_name)
                        .map(|fid| self.callee_ret_ty(fid))
                        .unwrap_or(SemTy::Dyn)
                } else if let Some(cid) = self.name_class_ref(*recv) {
                    self.classes
                        .get(cid)
                        .and_then(|i| {
                            i.static_method(*method_name)
                                .or_else(|| i.class_method(*method_name))
                        })
                        .map(|m| self.callee_ret_ty(m.func_id))
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
                .map(|info| SemTy::Class {
                    class_id: *cid,
                    name: info.name,
                })
                .unwrap_or(SemTy::Dyn),
            HirExprKind::IsInstance { .. } => SemTy::Bool,
            HirExprKind::IsInstanceBuiltin { .. } => SemTy::Bool,
            HirExprKind::IsNone { .. } => SemTy::Bool,
            HirExprKind::Is { .. } => SemTy::Bool,
            // A stdlib runtime call types as its descriptor's declared return
            // `TypeSpec` (Phase 8B); arg/param compatibility is enforced in
            // `check_repr_boundaries` (the contract seam, like calls).
            HirExprKind::CallRuntime { target, args, .. } => {
                let base = target.result_semty();
                // `open(...)`'s File is text or binary by its (constant) mode
                // literal — a `b` in the mode string (Phase 8C).
                if let SemTy::File { .. } = base {
                    let binary = args
                        .get(1)
                        .and_then(|slot| *slot)
                        .map(|idx| match &self.func.exprs[idx].kind {
                            HirExprKind::StrLit(s) => self.interner.resolve(*s).contains('b'),
                            _ => false,
                        })
                        .unwrap_or(false);
                    SemTy::File { binary }
                } else {
                    base
                }
            }
            // `Stack[int](...)` → the generic instance type (args drive precise
            // field/method substitution; erased at repr to one shared layout). A
            // bare construction of a *non-generic* class (no type args, no type
            // params — e.g. the Phase-8 `M.Cls(...)` qualified path) types as the
            // nominal `Class` so it unifies with a `Cls`-typed annotation.
            HirExprKind::GenericConstruct {
                class_id,
                type_args,
                ..
            } => match self.classes.get(*class_id) {
                Some(info) if type_args.is_empty() && info.type_params.is_empty() => SemTy::Class {
                    class_id: *class_id,
                    name: info.name,
                },
                _ => SemTy::Generic {
                    base: *class_id,
                    args: type_args.clone(),
                },
            },
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
                    self.st
                        .join_writes(&self.st.cell_writes[cell.index()], self.classes)
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
            // itself, the `global_ty` module variable elsewhere), `Dyn` once
            // any other function writes it (Phase 6B).
            HirExprKind::GlobalGet { var_id } => {
                let vid = *var_id as usize;
                if self.global_authoritative.get(vid).copied().unwrap_or(false) {
                    // A module-level annotation is the slot's contract type
                    // everywhere — never inferred from writes (Phase 8).
                    self.global_slot_ty(vid)
                } else if self.global_demoted.get(vid).copied().unwrap_or(false) {
                    SemTy::Dyn
                } else if !self.st.global_writes[vid].is_empty() {
                    self.st
                        .join_writes(&self.st.global_writes[vid], self.classes)
                } else {
                    self.global_slot_ty(vid)
                }
            }
            // ── exceptions (Phase 7) ──
            // `Current` keeps the frontend-assigned static type (the except
            // clause's class); the match queries are booleans. All ride the
            // Tagged baseline (Principle 2) — no new constraints.
            HirExprKind::ExcQuery(q) => match q {
                pyaot_hir::ExcQuery::Current => self.func.exprs[idx].ty.clone(),
                pyaot_hir::ExcQuery::MatchesBuiltin(_) | pyaot_hir::ExcQuery::MatchesClass(_) => {
                    SemTy::Bool
                }
            },
            HirExprKind::ExcInstanceStr { .. } => SemTy::Str,
        }
    }

    /// The type-parameter substitution implied by a generic-instance receiver
    /// (`Stack[int]` → `{T ↦ int}`), if its base is a user generic class (5E).
    fn subst_for(&self, recv: &SemTy) -> Option<HashMap<pyaot_utils::InternedString, SemTy>> {
        let SemTy::Generic { base, args } = recv else {
            return None;
        };
        let info = self.classes.get(*base)?;
        if info.type_params.is_empty() {
            return None;
        }
        Some(
            info.type_params
                .iter()
                .copied()
                .zip(args.iter().cloned())
                .collect(),
        )
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
        // `Never` is the in-progress bottom (the receiver's type is not yet
        // solved this sweep) — stay `Never`; jumping to `Dyn` here would
        // absorb through a self-referential constraint (`x = [xi.m() for xi
        // in x]`) and poison the receiver irreversibly (PITFALLS A2/B6).
        if recv == SemTy::Never {
            return SemTy::Never;
        }
        if let Some(cid) = class_of(&recv, self.classes) {
            let ret = self
                .classes
                .get(cid)
                .and_then(|info| {
                    info.method(method_name)
                        .or_else(|| info.static_method(method_name))
                        .or_else(|| info.class_method(method_name))
                })
                .map(|m| self.callee_ret_ty(m.func_id));
            return match ret {
                // Substitute the generic type params for a `Stack[int]` receiver.
                Some(t) => self.apply_subst(&recv, t),
                None => SemTy::Dyn,
            };
        }
        // str-receiver methods routed through runtime descriptors (Phase 8B/8C;
        // the full str-method surface lands with 8E).
        if matches!(recv, SemTy::Str) {
            match self.interner.resolve(method_name) {
                "upper" | "lower" | "strip" | "title" | "capitalize" | "swapcase" | "zfill"
                | "center" | "ljust" | "rjust" => return SemTy::Str,
                "startswith" | "endswith" => return SemTy::Bool,
                "find" | "rfind" | "index" | "count" => return SemTy::Int,
                _ => {}
            }
        }
        // `bytes.decode([encoding])` → `str` (Phase 8D).
        if matches!(recv, SemTy::Bytes) && self.interner.resolve(method_name) == "decode" {
            return SemTy::Str;
        }
        // A stdlib runtime object's method (`m.group()`, Phase 8C): typed from
        // its `StdlibMethodDef` in the object-type registry.
        if let SemTy::RuntimeObject(tag) = &recv {
            return pyaot_stdlib_defs::object_types::lookup_object_method(
                *tag,
                self.interner.resolve(method_name),
            )
            .map(|m| pyaot_hir::semty_from_typespec(&m.return_type))
            .unwrap_or(SemTy::Dyn);
        }
        // File object methods (Phase 8C): a fixed surface routed to `rt_file_*`
        // at lowering. `read`/`readline`/`readlines` yield bytes in binary mode.
        if let SemTy::File { binary } = &recv {
            return file_method_ty(self.interner.resolve(method_name), *binary);
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
        // In-progress bottom receiver: propagate `Never` (see method_call_ty).
        if recv == SemTy::Never {
            return SemTy::Never;
        }
        // `e.args` on a caught builtin exception — or a tuple clause of only
        // builtins — (Phase 7B): the args tuple. Typed `Dyn` (not
        // `tuple[Dyn, ...]`) so a user annotation like `args: tuple[str]` is
        // admitted gradually rather than rejected by the tuple-arity contract.
        let builtin_exc = match &recv {
            SemTy::BuiltinException(_) => true,
            SemTy::Union(members) => {
                !members.is_empty()
                    && members
                        .iter()
                        .all(|m| matches!(m, SemTy::BuiltinException(_)))
            }
            _ => false,
        };
        if builtin_exc {
            return SemTy::Dyn;
        }
        // A stdlib runtime object's field (`t.tm_year`, Phase 8B): declared by
        // its `ObjectFieldDef` in `stdlib-defs`.
        if let SemTy::RuntimeObject(tag) = &recv {
            if let Some(obj) = pyaot_stdlib_defs::object_types::lookup_object_type(*tag) {
                if let Some(field) = obj.get_field(self.interner.resolve(name)) {
                    return pyaot_hir::semty_from_typespec(&field.field_type);
                }
            }
            return SemTy::Dyn;
        }
        let Some(cid) = class_of(&recv, self.classes) else {
            return SemTy::Dyn;
        };
        let Some(info) = self.classes.get(cid) else {
            return SemTy::Dyn;
        };
        let raw = if let Some(p) = info.property(name) {
            p.ty.clone()
        } else if let Some(t) = info.field_ty(name) {
            // B10: an unannotated field of a non-generic defining class reads
            // its solver variable (`Never` while still climbing — the working
            // bottom, exactly like `callee_ret_ty`). Annotated /
            // generic-class fields keep the static type.
            if let Some(d) = defining_class(self.classes, cid, name) {
                if let Some(&vi) = self.field_vars.get(&(d, name)) {
                    return self.field_var_ty(vi);
                }
            }
            t.clone()
        } else if let Some(a) = info.class_attr(name) {
            a.ty.clone()
        } else {
            return SemTy::Dyn;
        };
        // Substitute the generic type params for a `Stack[int]` receiver (5E).
        self.apply_subst(&recv, raw)
    }

    /// Join the types of every expr in `elems` (the lattice bottom for empty),
    /// as a container ELEMENT slot type — [`raw_uniform`]-guarded, because the
    /// elements are stored as-is: `[2.25, 16]` must be `list[Dyn]` (tagged
    /// elements), never `list[float]` whose reads would blindly unbox the
    /// stored tagged int.
    fn join_all(&self, elems: &[Idx<HirExpr>]) -> SemTy {
        let tys: Vec<SemTy> = elems.iter().map(|e| self.ety(*e)).collect();
        let joined = tys
            .iter()
            .fold(SemTy::Never, |acc, t| acc.join(t, self.classes));
        raw_uniform(joined, &tys)
    }

    /// The declared return type of a concrete-class dunder `name` on `ty`, if any
    /// (used to type class operator results precisely — `v + v → Vector`).
    fn class_dunder_ret(&self, ty: &SemTy, name: &str) -> Option<SemTy> {
        let cid = class_of(ty, self.classes)?;
        let info = self.classes.get(cid)?;
        let m = info
            .methods
            .iter()
            .find(|m| self.interner.resolve(m.name) == name)?;
        Some(self.callee_ret_ty(m.func_id))
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
            return elems
                .iter()
                .fold(SemTy::Never, |acc, t| acc.join(t, self.classes));
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

    /// The type of a slice `base[a:b:c]` (Phase 8E): the same kind as `base`. A
    /// `list[T]` slices to `list[T]`; a tuple slices to a homogeneous tuple of
    /// the joined element type; `str`/`bytes` preserve; anything else is `Dyn`
    /// (the runtime-dispatched `rt_obj_slice`).
    fn slice_ty(&self, base: Idx<HirExpr>) -> SemTy {
        let bt = self.ety(base);
        if bt.list_elem().is_some() {
            return bt;
        }
        if let Some(e) = bt.tuple_var_elem() {
            return SemTy::tuple_var_of(e.clone());
        }
        if let Some(elems) = bt.tuple_elems() {
            let joined = elems
                .iter()
                .fold(SemTy::Never, |acc, t| acc.join(t, self.classes));
            return SemTy::tuple_var_of(joined);
        }
        match bt {
            SemTy::Str => SemTy::Str,
            SemTy::Bytes => SemTy::Bytes,
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
            C::Iter => SemTy::Iterator(Box::new(iter_elem_ty(&arg0(), self.classes))),
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
                iter_elem_ty(&arg0(), self.classes),
            ]))),
            C::Zip => {
                let a = iter_elem_ty(&arg0(), self.classes);
                let b = args
                    .get(1)
                    .map(|x| iter_elem_ty(&self.ety(*x), self.classes))
                    .unwrap_or(SemTy::Dyn);
                SemTy::Iterator(Box::new(SemTy::tuple_of(vec![a, b])))
            }
            C::ListFromIter => SemTy::list_of(iter_elem_ty(&arg0(), self.classes)),
            C::TupleFromIter => SemTy::tuple_var_of(iter_elem_ty(&arg0(), self.classes)),
            C::DictFromPairs => {
                let a = arg0();
                // `dict(d)` on a known dict copies it (lowering routes to
                // DictCopy); the result keeps the source's key/value types.
                if a.dict_kv().is_some() {
                    return a;
                }
                match iter_elem_ty(&a, self.classes).tuple_elems() {
                    // `dict([(k, v), …])` — the element is a 2-tuple of (key, value).
                    Some(kv) if kv.len() == 2 => SemTy::dict_of(kv[0].clone(), kv[1].clone()),
                    _ => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
                }
            }
            C::BytesFromList => SemTy::Bytes,
            C::Sorted => SemTy::list_of(iter_elem_ty(&arg0(), self.classes)),
            // Mutating tandem sort in expression position (the `sort(key=)`
            // frontend desugar) — yields None.
            C::ListSortByKeys => SemTy::NoneTy,
            C::Reversed => SemTy::Iterator(Box::new(iter_elem_ty(&arg0(), self.classes))),
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
                return self.st.local_ty[lid.index()].clone();
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
                    l.join(&r, self.classes)
                }
            }
            // `+` over two same-base containers already joins to that container
            // (covariant lattice join), so list/tuple/bytes concatenation types
            // correctly without a special case.
            BinOp::Add | BinOp::Sub | BinOp::FloorDiv | BinOp::Mod | BinOp::Pow => {
                l.join(&r, self.classes)
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
                    l.join(&r, self.classes)
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
                Symbol::Function(fid) => return self.callee_ret_ty(fid),
                Symbol::Builtin(kind) => return self.builtin_ty(kind, args),
                Symbol::Container(op) => return self.container_op_ty(op, args),
                // `Cls(args)` constructs an instance of that class.
                Symbol::Class(cid) => {
                    return match self.classes.get(cid) {
                        Some(info) => SemTy::Class {
                            class_id: cid,
                            name: info.name,
                        },
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
/// The static return type of a File method (Phase 8C). `read`/`readline` give
/// `bytes` in binary mode, `str` in text mode; `readlines` a list of those.
fn file_method_ty(method: &str, binary: bool) -> SemTy {
    let elem = if binary { SemTy::Bytes } else { SemTy::Str };
    match method {
        "read" | "readline" => elem,
        "readlines" => SemTy::list_of(elem),
        "write" => SemTy::Int,
        "close" | "flush" => SemTy::NoneTy,
        // Context-manager dunders: `__enter__` returns self (the File), and
        // `__exit__` returns a bool (truthy ⇒ swallow the exception).
        "__enter__" => SemTy::File { binary },
        "__exit__" => SemTy::Bool,
        _ => SemTy::Dyn,
    }
}

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
            // `dict.get(k)` returns the value OR `None` on a miss — type it
            // `Optional[V]` so the result rides `Tagged` (a `None` is then a safe
            // tagged sentinel, not misread as a heap `V` pointer → SEGV, the bug
            // that hit `os.environ.get(missing)` whose value type is `str`).
            M::Get => SemTy::optional(v.clone()),
            // `pop` raises `KeyError` on a miss (no `None`), `setdefault` inserts.
            M::Pop | M::Setdefault => v.clone(),
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
fn iter_elem_ty(t: &SemTy, classes: &ClassTable) -> SemTy {
    let elem = iter_elem_raw(t, classes);
    if elem == SemTy::Never && *t != SemTy::Never {
        SemTy::Dyn
    } else {
        elem
    }
}

fn iter_elem_raw(t: &SemTy, classes: &ClassTable) -> SemTy {
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
        return elems
            .iter()
            .fold(SemTy::Never, |acc, x| acc.join(x, classes));
    }
    if let Some((k, _)) = t.dict_kv() {
        // Iterating a dict yields its keys.
        return k.clone();
    }
    match t {
        SemTy::Str => SemTy::Str,
        SemTy::Bytes => SemTy::Int,
        SemTy::Iterator(e) => (**e).clone(),
        // Iterating a file yields its lines (Phase 8H). Binary mode is out of
        // scope — `for b in open(p, "rb")` would need bytes lines.
        SemTy::File { binary: false } => SemTy::Str,
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
fn erase_vars(ty: &SemTy, classes: &ClassTable) -> SemTy {
    if !ty.contains_var() {
        return ty.clone();
    }
    match ty {
        SemTy::Var(_) => SemTy::Dyn,
        SemTy::Generic { base, args } => SemTy::Generic {
            base: *base,
            args: args.iter().map(|a| erase_vars(a, classes)).collect(),
        },
        SemTy::Iterator(t) => SemTy::Iterator(Box::new(erase_vars(t, classes))),
        // Re-join union members through the lattice so an erased `Dyn` absorbs.
        SemTy::Union(ts) => ts.iter().fold(SemTy::Never, |acc, t| {
            acc.join(&erase_vars(t, classes), classes)
        }),
        other => other.clone(),
    }
}

fn materialize(func: &mut HirFunction, st: &FuncState, classes: &ClassTable) {
    for (i, local) in func.locals.iter_mut().enumerate() {
        let ty = erase_vars(&st.local_ty[i], classes);
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
        if let Some(ty) = st.expr_ty.get(&idx) {
            let ty = erase_vars(ty, classes);
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
            let HirStmt::Assign { target, value } = stmt else {
                continue;
            };
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

mod intervals;

#[cfg(test)]
mod tests;
