//! Production wire-in: bridges the constraint solver to [`Lowering`].
//!
//! Three components:
//!
//! 1. [`LoweringReducerCtx`] — implements [`ReducerCtx`] over `&Lowering` and
//!    `&hir::Module`, delegating each compound query to the existing
//!    `infer.rs` resolvers (already pure functions of their inputs).
//!
//! 2. [`apply_to_lowering`] — drains a [`MaterializeOutput`] into the
//!    corresponding [`Lowering`] / [`hir::Module`] fields. Writes
//!    `expr.ty` directly into the HIR arena so downstream lowering reads
//!    the solver's view.
//!
//! 3. [`run`] — top-level orchestrator: collect → solve → materialize →
//!    apply. Called from `build_lowering_seed_info` after the structural
//!    pre-passes (Phase-4 unsafe-funcs, decorator processing,
//!    annotation validation) have run.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::{Type, TypeLattice};
use pyaot_utils::{ClassId, InternedString};

use super::collect::collect;
use super::materialize::{materialize, MaterializeOutput};
use super::solve::{default_iter_elem, ReducerCtx, Solver};
use super::vocab::BuiltinId;

use crate::context::Lowering;
use crate::type_planning::helpers;

/// Reducer context for the production wiring. Holds the borrows the
/// `ReducerCtx` impl needs to consult the live [`Lowering`] and
/// [`hir::Module`] state.
pub(crate) struct LoweringReducerCtx<'a, 'b> {
    lowering: &'a Lowering<'b>,
    module: &'a hir::Module,
}

impl<'a, 'b> LoweringReducerCtx<'a, 'b> {
    pub(crate) fn new(lowering: &'a Lowering<'b>, module: &'a hir::Module) -> Self {
        Self { lowering, module }
    }
}

impl<'a, 'b> ReducerCtx for LoweringReducerCtx<'a, 'b> {
    /// Class-dunder lookup — mirrors `infer::binop_result_type`'s policy:
    /// cross-module classes (not in `class_info`) accept unconditionally
    /// to avoid dropping a Union variant whose dunder we can't see.
    fn class_has_dunder(&self, class_id: ClassId, dunder: &str) -> bool {
        match self.lowering.get_class_info(&class_id) {
            None => true,
            Some(ci) => ci.get_dunder_func(dunder).is_some(),
        }
    }

    /// Delegate to the existing `infer.rs` resolver, which consults the
    /// dispatched dunder's actual return type (forward / reflected,
    /// subclass-first, NotImplemented-stripped). Reuses the same logic the
    /// seed-inference layer (`binop_result_type`) and lowering dest
    /// (`alloc_dunder_result`) already rely on, so the solver's view of a
    /// binop-result variable now agrees with both.
    fn binop_class_return(&self, op: &hir::BinOp, lt: &Type, rt: &Type) -> Option<Type> {
        self.lowering.resolve_class_binop_return(op, lt, rt)
    }

    fn method_return(&self, recv: &Type, method: InternedString, _args: &[Type]) -> Option<Type> {
        let unwrapped = helpers::unwrap_optional(recv);
        let method_name = self.lowering.resolve(method);
        self.lowering
            .resolve_method_on_type(&unwrapped, method, method_name, self.module)
    }

    fn attribute_return(&self, recv: &Type, attr: InternedString) -> Option<Type> {
        self.lowering.resolve_attribute_on_type(recv, attr)
    }

    fn class_has_instance_field(&self, class_id: ClassId, attr: InternedString) -> bool {
        self.module
            .class_defs
            .get(&class_id)
            .is_some_and(|cdef| cdef.fields.iter().any(|f| f.name == attr))
    }

    /// Walk the `base_class` chain so an inherited field counts as present on
    /// the subclass — the subclass's `LoweredClassInfo` inherits the field
    /// layout, so the `FieldWriteDynamic` reducer must be allowed to widen it.
    fn class_has_field_in_hierarchy(&self, class_id: ClassId, attr: InternedString) -> bool {
        let mut current = Some(class_id);
        while let Some(cid) = current {
            let Some(cdef) = self.module.class_defs.get(&cid) else {
                break;
            };
            if cdef.fields.iter().any(|f| f.name == attr) {
                return true;
            }
            current = cdef.base_class;
        }
        false
    }

    fn subscript_return(&self, recv: &Type, _index: &Type) -> Option<Type> {
        // The legacy resolver inspects the index expression to handle
        // literal-int tuple subscripts precisely. The solver doesn't
        // carry an `ExprId` for the index here — the eval_subscript
        // inline path covers list/dict/set/tuple/str/bytes. Anything
        // else (user `__getitem__`) returns `None` and falls back to
        // `Type::Any`, matching the legacy behaviour for non-class
        // subscripts on unknown receivers.
        //
        // S6 soak refinement: if this loss of precision shows up in a
        // failing example, extend the constraint with an
        // `Option<i64>` const-index field at collection time.
        let _ = recv;
        None
    }

    fn builtin_return(&self, builtin: BuiltinId, args: &[Type]) -> Option<Type> {
        // The legacy resolver also takes `&[hir::ExprId]` for literal
        // inspection (e.g. `int("42")` recognizing a string-literal
        // arg). The solver doesn't have ExprIds at this layer, so we
        // pass an empty arg slice. Most builtins resolve precisely
        // from `arg_types` alone; the literal-inspection path
        // degrades to the type-only fallback.
        self.lowering
            .resolve_builtin_with_overrides(&builtin.0, &[], args, self.module)
    }

    /// Constructor call → `Type::Class { class_id, name }` for known
    /// classes, `Type::Any` for unknown. Matches `infer::class_ref_type`.
    fn class_ctor_return(&self, class_id: ClassId) -> Option<Type> {
        self.module
            .class_defs
            .get(&class_id)
            .map(|def| Type::Class {
                class_id,
                name: def.name,
            })
    }

    /// User-class `__iter__` dispatch on top of the built-in
    /// `default_iter_elem`. The default impl handles list/dict/set/
    /// iterator/tuple/str/bytes; this override checks if a `Class`
    /// receiver has a `__next__` dunder whose return type tells us
    /// the element type.
    fn iter_elem(&self, iter: &Type) -> Option<Type> {
        if let Some(t) = default_iter_elem(iter) {
            return Some(t);
        }
        if let Type::Class { class_id, .. } = iter {
            if let Some(class_info) = self.lowering.get_class_info(class_id) {
                // Try __next__ first; fall back to __iter__'s ret
                // (which typically returns `self`).
                if let Some(next_fid) = class_info.get_dunder_func("__next__") {
                    if let Some(ret_ty) = self.lowering.get_func_return_type(&next_fid) {
                        return Some(ret_ty.clone());
                    }
                }
            }
        }
        None
    }
}

/// Apply a [`MaterializeOutput`] to the production [`Lowering`] and
/// [`hir::Module`] state. Pure side-effect — no return value.
///
/// Writes performed:
/// - `hir_module.exprs[eid].ty = Some(ty)` for each cached expr.
/// - `lowering.lowering_seed_info.expr_types[eid] = ty`.
/// - `lowering.lowering_seed_info.base_var_types[v] = ty`.
/// - `lowering.lowering_seed_info.refined_class_field_types[cid][name] = ty`.
/// - `lowering.func_return_types.inner[fid] = ty`.
/// - `lowering.closures.lambda_param_type_hints[fid]: Vec<Type>` (slot-
///   indexed; missing slots filled with `Type::Any`).
/// - `lowering.closures.closure_capture_types[fid]: Vec<Type>` (same).
/// - Generator wrapping: `func_yield_types[fid]` is wrapped in
///   `Iterator(_)` and written to `func_return_types[fid]` when
///   `func.is_generator` and no explicit return annotation is set.
/// - `per_function_local_seed_types`: each function's Var locals are
///   harvested by walking its params + bind statements.
pub(crate) fn apply_to_lowering(
    lowering: &mut Lowering,
    hir_module: &mut hir::Module,
    out: MaterializeOutput,
) {
    // 1. Cache expression types into LoweringSeedInfo (mirrors legacy
    //    `eagerly_populate_expr_types`). We deliberately do NOT also
    //    write into `hir::Expr.ty` — verified empirically that doing
    //    so does not affect call-dest typing (legacy planner skips
    //    the HIR write too, and downstream consumers read through the
    //    `expr_types` cache or `func_return_types` lookups).
    //
    //    CRITICAL: skip `Var`-reference expressions. The legacy
    //    `eagerly_populate_expr_types` explicitly skips `Var` arms
    //    because a variable's *effective* type at a use site is
    //    context-sensitive (isinstance narrowing, and — decisively —
    //    loop-variable rebinding). The solver stores ONE global type per
    //    `Var`, so caching `Expr(Var(x)) = <global type>` pins a
    //    use-site to that global type. When the global type is wider
    //    than the use-site type (e.g. `x` is a loop element typed `V`
    //    locally but the global `Var(x)` merged to `list[V] | V`),
    //    lowering then mis-resolves `x.attr`. Leaving Var refs out of the
    //    cache lets lowering recompute the use-site type on demand via
    //    `get_var_type`, exactly as it did under the legacy planner.
    for (eid, ty) in &out.expr_types {
        if matches!(hir_module.exprs[*eid].kind, hir::ExprKind::Var(_)) {
            continue;
        }
        lowering
            .lowering_seed_info
            .expr_types
            .insert(*eid, ty.clone());
    }

    // 2. Base var types.
    for (v, ty) in &out.base_var_types {
        lowering
            .lowering_seed_info
            .base_var_types
            .insert(*v, ty.clone());
    }

    // 2a. Container-typed vars also seed `refined_container_types`. The
    //     legacy planner's `refine_empty_container_types` filled this map
    //     for every `x = []; x.append(elem)` refinement, and it is the
    //     FIRST source consulted both by `get_var_type` (lowering-time
    //     effective type) and by the assignment-lowering priority chain
    //     (`statements/assign/mod.rs`). `base_var_types` is NOT in the
    //     `get_var_type` fallback chain, so a container var refined only
    //     in `base_var_types` is invisible to `get_var_type`. Mirroring
    //     the solver's container types here makes the solver env the
    //     read-only view the plan calls for: `for x in acc` then resolves
    //     `acc`'s element type to the refined `V`, not the `[]` literal's
    //     `list[Never]`.
    //     Restricted to `list`- and `deque`-shaped vars with a concrete
    //     element type. The broad form (all container shapes) regressed
    //     `test_collections`: mirroring a `dict`/`defaultdict` var pinned
    //     a `defaultdict` slot to a plain `Dict` shape, producing a
    //     `Heap(DefaultDict) → Heap(Dict)` Copy verifier error. Lists are
    //     the shape that actually needs this bridge (`x = []; x.append`),
    //     and a degenerate `list[Never]`/`list[Any]` carries no refinement
    //     worth pinning. `deque` is safe to include too: `deque[T]` and
    //     `deque[Any]` translate to the SAME physical MIR type
    //     (`Heap(RuntimeObj(Deque))`), so mirroring `deque[Int]` cannot
    //     introduce a shape-mismatch Copy error — it just lets the
    //     empty-then-appended `dq = deque(); dq.append(1)` bootstrap reach
    //     the prescan / `get_var_type` chain with the refined element type
    //     (otherwise iteration / `dq[i]` see `deque[Any]` and the loop var
    //     stays tagged).
    for (v, ty) in &out.base_var_types {
        let elem = if ty.is_list_like() {
            ty.list_elem()
        } else if ty.is_deque_like() {
            ty.deque_elem()
        } else {
            continue;
        };
        let elem_trivial = elem
            .map(|e| matches!(e, Type::Never | Type::Any))
            .unwrap_or(true);
        if elem_trivial {
            continue;
        }
        lowering
            .lowering_seed_info
            .refined_container_types
            .insert(*v, ty.clone());
    }

    // 2b. Globals adapter (plan Risk #4). The legacy planner ran
    //     `propagate_globals_from_prescan` to copy module-level global
    //     types into `symbols.global_var_types`, which `get_var_type`
    //     consults for cross-function references (`def f(): return g`
    //     where `g` is a module global). The solver computes a single
    //     global type per VarId in `base_var_types`; mirror the non-`Any`
    //     ones into `global_var_types` so cross-function reads resolve
    //     precisely instead of degrading to `Any`. Only non-`Any` writes,
    //     matching legacy's guard — a transient `Any` must never clobber
    //     a concrete type set elsewhere.
    {
        let globals: Vec<pyaot_utils::VarId> = lowering.symbols.globals.iter().copied().collect();
        for g in globals {
            if let Some(ty) = out.base_var_types.get(&g) {
                if !matches!(ty, Type::Any) {
                    lowering.symbols.global_var_types.insert(g, ty.clone());
                }
            }
        }
    }

    // 3. Refined class field types — solver stores `(ClassId, name) → Type`
    //    flat; legacy contract is `IndexMap<ClassId, IndexMap<name, Type>>`.
    //    Group by class id.
    for ((cid, name), ty) in &out.refined_class_field_types {
        lowering
            .lowering_seed_info
            .refined_class_field_types
            .entry(*cid)
            .or_default()
            .insert(*name, ty.clone());
    }

    // 4. Function return types (regular). Generator returns are
    //    overridden below.
    for (fid, ty) in &out.func_return_types {
        lowering.func_return_types.inner.insert(*fid, ty.clone());
    }
    // 4b. Generator return type = `Iterator(yield_element_type)`.
    //
    //     Walks every generator function in the HIR (not just those
    //     with a FuncYield entry). A generator with no Yield constraints
    //     (e.g. a body that only contains `return`) still has return
    //     type `Iterator(Never)` — `desugar_generators` requires the
    //     `Iterator(_)` shape regardless of the inner element type.
    //     Generators with an explicit annotation keep their declared
    //     type. The collector deliberately suppresses Return constraints
    //     inside generators (Python's `return v` raises StopIteration(v),
    //     it does NOT contribute to the iterator's element type).
    for (fid, func) in &hir_module.func_defs {
        if !func.is_generator || func.return_type.is_some() {
            continue;
        }
        let yield_ty = out
            .func_yield_types
            .get(fid)
            .cloned()
            .unwrap_or(Type::Never);
        let iter_ty = Type::Iterator(Box::new(yield_ty));
        lowering.func_return_types.inner.insert(*fid, iter_ty);
    }

    // 5. Lambda parameter hints — group by FuncId, ordered by slot.
    {
        let mut by_func: IndexMap<pyaot_utils::FuncId, Vec<(usize, Type)>> = IndexMap::new();
        for ((fid, ix), ty) in &out.lambda_param_type_hints {
            by_func.entry(*fid).or_default().push((*ix, ty.clone()));
        }
        for (fid, mut slots) in by_func {
            slots.sort_by_key(|(ix, _)| *ix);
            // The legacy contract is a dense Vec; fill missing slots
            // with `Type::Any` so a hint at index 3 doesn't silently
            // shift indices 0..2.
            let max_ix = slots.last().map(|(ix, _)| *ix).unwrap_or(0);
            let mut dense = vec![Type::Any; max_ix + 1];
            for (ix, ty) in slots {
                dense[ix] = ty;
            }
            lowering.closures.lambda_param_type_hints.insert(fid, dense);
        }
    }

    // 6. Closure capture types — same shape transformation.
    {
        let mut by_func: IndexMap<pyaot_utils::FuncId, Vec<(usize, Type)>> = IndexMap::new();
        for ((fid, slot), ty) in &out.closure_capture_types {
            by_func.entry(*fid).or_default().push((*slot, ty.clone()));
        }
        for (fid, mut slots) in by_func {
            slots.sort_by_key(|(slot, _)| *slot);
            let max_slot = slots.last().map(|(slot, _)| *slot).unwrap_or(0);
            let mut dense = vec![Type::Any; max_slot + 1];
            for (slot, ty) in slots {
                dense[slot] = ty;
            }
            // Mirror capture types into the HIR closure params. The
            // lifted-closure ABI makes captures the leading positional
            // params (`params[0..N]`), so `params[i].ty` should reflect
            // the capture type in slot `i`. The legacy planner wrote these
            // annotations directly into the HIR; several downstream
            // consumers read the HIR `param.ty` rather than this
            // side-table — most importantly `collect_generator_vars`,
            // which types a generator's persistent state slots from
            // `param.ty`. Without this mirror, a generator-expression
            // capture (e.g. `(wi * xi for wi, xi in zip(wo, x))`) loses
            // the element type of its captured iterables, and the resume
            // function's loop variables degrade to `Any` — forcing a
            // class-instance binop onto the raw-arithmetic path the MIR
            // verifier rejects. Only unannotated params are filled, and
            // only with concrete (non-`Any`) capture types.
            if let Some(func) = hir_module.func_defs.get_mut(&fid) {
                for (i, cap_ty) in dense.iter().enumerate() {
                    if matches!(cap_ty, Type::Any) {
                        continue;
                    }
                    if let Some(param) = func.params.get_mut(i) {
                        if param.ty.is_none() {
                            param.ty = Some(cap_ty.clone());
                        }
                    }
                }
            }
            lowering.closures.closure_capture_types.insert(fid, dense);
        }
    }

    // NOTE: `per_function_local_seed_types` is intentionally NOT produced
    // here. The post-desugar prescan (`precompute_all_local_var_types`,
    // called from `lower_module` after `desugar_generators`) walks every
    // function in `hir_module.functions` and unconditionally overwrites
    // that map with flow-sensitive `precompute_var_types` output. Since
    // nothing reads `per_function_local_seed_types` between this point and
    // that overwrite, any per-function seeding here would be dead work.
    // The solver owns the *global* views (base_var_types, func_return_types,
    // closure contracts); the legacy prescan owns the per-function view.
}

/// Top-level orchestrator: collect constraints from `hir_module`, solve
/// to fixpoint with [`LoweringReducerCtx`], materialize, then apply.
///
/// The Phase-4 unsafe-funcs analysis and decorator processing must run
/// BEFORE this — they set structural state the solver doesn't compute.
/// `populate_generator_return_types_on_funcdef` must run AFTER —
/// it copies `func_return_types[gen]` into `hir_module.func_defs[gen].return_type`
/// for downstream desugar to read.
pub(crate) fn run(lowering: &mut Lowering, hir_module: &mut hir::Module) {
    // Build the constraint graph and solve, scoping the borrows so
    // we can take a mutable borrow on `hir_module` during apply.
    let out = {
        let mut solver = Solver::new();
        // The collector needs an immutable interner reference for
        // method-name dispatch (`append` / `add` etc.). We borrow it
        // from `lowering` for the duration of collect/solve, then
        // release before `apply_to_lowering` takes the mutable borrow.
        // Re-borrow through `&*` to get `&StringInterner` from the
        // `&mut StringInterner` field — this is a no-op at runtime
        // but releases the mutable lock for the duration of the borrow.
        let interner: &pyaot_utils::StringInterner = &*lowering.interner;
        collect(&mut solver, hir_module, interner);
        let ctx = LoweringReducerCtx::new(lowering, hir_module);
        solver.run(&ctx);
        materialize(solver.env())
    };
    apply_to_lowering(lowering, hir_module, out);

    // Silence unused-import warnings for the lattice trait — it's
    // pulled in for the `Type::bottom()` reference that some future
    // post-processing here may need. The trait must be in scope for
    // the materialized types to compose cleanly.
    let _ = Type::bottom();
}
