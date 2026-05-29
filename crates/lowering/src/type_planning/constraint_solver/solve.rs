//! Worklist solver — drives [`Constraint`]s to a monotone fixpoint over the
//! solver [`Env`].
//!
//! S3 STATUS: worklist driver + all 11 JOIN-style reducers + all 5 compound
//! reducers (`Call`, `MethodCall`, `Attribute`, `Subscript`, `IterElem`).
//! Compound reducers consult a [`ReducerCtx`] trait whose default impls
//! return `None` (→ result widens to `Any`). Production wiring (S5) plugs
//! `Lowering` into the trait so the legacy `infer.rs` resolvers — already
//! pure — become the reducer bodies.

use std::collections::VecDeque;

use indexmap::{IndexMap, IndexSet};
use pyaot_hir::UnOp;
use pyaot_types::{Type, TypeLattice};
use pyaot_utils::{ClassId, InternedString};

use super::env::Env;
use super::key::TypeKey;
use super::vocab::{BuiltinId, CalleeRef, Constraint, ConstraintId, ContainerKind};

use crate::type_planning::helpers;

/// Resolution context passed into the solver. Compound reducers consult
/// this trait whenever they need information that isn't expressible in
/// pure lattice ops over the env — class field tables, builtin signatures,
/// constructor return types. Production wiring (S5) implements this on
/// `Lowering` so the existing `infer.rs` resolvers become the trait bodies.
///
/// Default impls return `None`, which makes the reducer fall back to
/// `Type::Any`. Tests provide trivial impls; the default is enough for
/// the `PermissiveCtx` unit tests in this file.
pub trait ReducerCtx {
    /// Returns `true` if class `class_id` is known (in the local module's
    /// `class_info` map) to define `dunder`. Cross-module classes — which
    /// the local Lowering can't introspect — return `true` (matching the
    /// `class_implements_protocol` policy in `infer.rs`).
    fn class_has_dunder(&self, class_id: ClassId, dunder: &str) -> bool;

    /// Resolve `recv.method(args)` → result type. Wraps
    /// `Lowering::method_call_result_type` / `resolve_method_on_type` in
    /// production. Default impl returns `None` (→ `Type::Any`).
    #[allow(unused_variables)]
    fn method_return(&self, recv: &Type, method: InternedString, args: &[Type]) -> Option<Type> {
        None
    }

    /// Resolve `recv.attr` → attribute type. Wraps
    /// `Lowering::attribute_result_type` / `resolve_attribute_on_type` in
    /// production.
    #[allow(unused_variables)]
    fn attribute_return(&self, recv: &Type, attr: InternedString) -> Option<Type> {
        None
    }

    /// True if `class_id` declares an INSTANCE field (`__slots__` / `self.x`
    /// assignment) named `attr`. Lets [`eval_attribute`] DEFER (return
    /// `Never`) instead of widening to `Any` when the solver-internal
    /// `ClassField(class_id, attr)` key hasn't been set yet — the
    /// `FieldWrite` reducer will set it, and the collector's dependency edge
    /// re-triggers the read. Widening to `Any` prematurely would poison the
    /// result (`Any` is top — `JOIN(Any, Float) = Any`). Default `false`
    /// (test contexts have no class metadata).
    #[allow(unused_variables)]
    fn class_has_instance_field(&self, class_id: ClassId, attr: InternedString) -> bool {
        false
    }

    /// Resolve `recv[index]` for user-defined `__getitem__`. Built-in
    /// containers (`list`/`dict`/`set`/`tuple`/`str`/`bytes`) are handled
    /// inline by [`eval_subscript`] without consulting this method.
    #[allow(unused_variables)]
    fn subscript_return(&self, recv: &Type, index: &Type) -> Option<Type> {
        None
    }

    /// Resolve a `BuiltinCall` — `len()`, `range()`, `str()`, etc. Wraps
    /// `Lowering::builtin_call_result_type` /
    /// `resolve_builtin_with_overrides`.
    #[allow(unused_variables)]
    fn builtin_return(&self, builtin: BuiltinId, args: &[Type]) -> Option<Type> {
        None
    }

    /// Resolve a class constructor `Class(args)` → instance type. The
    /// solver supplies the class id; production wiring looks up the name
    /// in `module.class_defs[class_id].name` and returns `Type::Class`.
    #[allow(unused_variables)]
    fn class_ctor_return(&self, class_id: ClassId) -> Option<Type> {
        None
    }

    /// Resolve the element type for `IterElem` constraints. The default
    /// impl handles every built-in iterable shape (list, set, dict,
    /// iterator, tuple, str, bytes) via the lattice's container
    /// accessors — sufficient for almost every iteration site. Production
    /// overrides this only to add user-class `__iter__` dispatch.
    fn iter_elem(&self, iter: &Type) -> Option<Type> {
        default_iter_elem(iter)
    }
}

/// Built-in iterator element-type resolution. Pure function over
/// [`Type`]; the same logic is used by `closure_scan::extract_iterable_element_type`
/// (which the production [`ReducerCtx`] impl will delegate to in S5).
///
/// `Iterator(T)` and `Generic{base, [T, ...]}` (list/set) yield their
/// element type. Dict yields its key type (Python `for k in d:` iterates
/// over keys). Tuple yields the union of its element types. `Str` and
/// `Bytes` yield `Str` (per-character iteration) and `Int` (byte values)
/// respectively. Everything else returns `None`.
pub(crate) fn default_iter_elem(iter: &Type) -> Option<Type> {
    if let Some(t) = iter.list_elem() {
        return Some(t.clone());
    }
    if let Some(t) = iter.set_elem() {
        return Some(t.clone());
    }
    if let Some((k, _v)) = iter.dict_kv() {
        return Some(k.clone());
    }
    if let Type::Iterator(t) = iter {
        return Some((**t).clone());
    }
    if let Some(elems) = iter.tuple_elems() {
        // Heterogeneous fixed tuple iterates over the union of its slots.
        return Some(elems.iter().fold(Type::bottom(), |acc, t| acc.join(t)));
    }
    if let Some(t) = iter.tuple_var_elem() {
        return Some(t.clone());
    }
    if matches!(iter, Type::Str) {
        return Some(Type::Str);
    }
    if matches!(iter, Type::Bytes) {
        return Some(Type::Int);
    }
    None
}

/// True iff `ty` is a container/iterator whose element (or dict value)
/// type is still bottom (`Never`). Such a type is *transitional* in the
/// monotone solver: it appears while an upstream producer (a generator
/// yield, an empty-then-appended list, …) hasn't resolved its element
/// type yet. Reductions over such a type must defer rather than widen to
/// `Any`. A genuinely-empty container left at `list[Never]` at fixpoint
/// is rare; accepting a minor precision loss there (it materializes to
/// `Any`) is far cheaper than the `Any`-absorption hazard.
pub(crate) fn element_is_bottom(ty: &Type) -> bool {
    if let Some(t) = ty.list_elem() {
        return matches!(t, Type::Never);
    }
    if let Some(t) = ty.set_elem() {
        return matches!(t, Type::Never);
    }
    if let Some(t) = ty.tuple_var_elem() {
        return matches!(t, Type::Never);
    }
    if let Some((_, v)) = ty.dict_kv() {
        return matches!(v, Type::Never);
    }
    if let Type::Iterator(t) = ty {
        return matches!(**t, Type::Never);
    }
    false
}

/// Trivial reducer context that accepts every dunder lookup. Used by the
/// solver's own unit tests where no class hierarchy is in scope.
#[cfg(test)]
pub struct PermissiveCtx;

#[cfg(test)]
impl ReducerCtx for PermissiveCtx {
    fn class_has_dunder(&self, _class_id: ClassId, _dunder: &str) -> bool {
        true
    }
}

/// The solver owns the constraint list, the dependents map, and the
/// environment. After [`Self::add`] is called for every constraint, the
/// caller invokes [`Self::run`] to drive the env to fixpoint.
pub struct Solver {
    /// Stable storage for all constraints. The index of each entry is its
    /// [`ConstraintId`].
    constraints: Vec<Constraint>,

    /// Reverse index — for each key, which constraints must be re-evaluated
    /// whenever that key's env value changes.
    deps: IndexMap<TypeKey, IndexSet<ConstraintId>>,

    /// Solver environment.
    env: Env,

    /// Anonymous-metavariable counter for [`TypeKey::Meta`].
    next_meta: u32,
}

impl Solver {
    pub fn new() -> Self {
        Self {
            constraints: Vec::new(),
            deps: IndexMap::new(),
            env: Env::new(),
            next_meta: 0,
        }
    }

    /// Allocate a fresh [`TypeKey::Meta`] — used for intermediate results
    /// inside reducers that have no HIR address of their own.
    pub fn fresh_meta(&mut self) -> TypeKey {
        let id = self.next_meta;
        self.next_meta = self
            .next_meta
            .checked_add(1)
            .expect("constraint solver: Meta key counter overflow (>4B metas)");
        TypeKey::Meta(id)
    }

    /// Register a constraint and record its dependents. Returns the
    /// [`ConstraintId`] the solver will use to enqueue this constraint.
    pub fn add(&mut self, c: Constraint) -> ConstraintId {
        let id = ConstraintId(
            u32::try_from(self.constraints.len())
                .expect("constraint solver: more than 4B constraints registered"),
        );
        // Record each input key as a trigger for re-evaluation of this
        // constraint. JOIN-only constraints (Concrete, FieldWrite, …) have
        // no inputs — they're still enqueued for the initial pass but
        // never re-scheduled.
        for input_key in inputs_of(&c) {
            self.deps.entry(input_key).or_default().insert(id);
        }
        self.constraints.push(c);
        id
    }

    /// Register an EXTRA dependency edge `input_key → cid` beyond the ones
    /// [`inputs_of`] derives from the constraint shape. Used when a reducer
    /// reads an env key that the collector can only determine dynamically
    /// (e.g. an `Attribute` on a receiver whose class is resolved during
    /// solving reads `ClassField(class, attr)` — the collector speculatively
    /// edges every class that defines `attr`, so the Attribute re-evaluates
    /// when that field's type is refined late).
    pub fn add_dep(&mut self, input_key: TypeKey, cid: ConstraintId) {
        self.deps.entry(input_key).or_default().insert(cid);
    }

    /// Convenience: emit a two-way [`Constraint::FlowsInto`] pair. The
    /// solver represents `Equal(a, b)` internally as `a → b` + `b → a`
    /// because the worklist algorithm only schedules dependents on a
    /// per-direction basis — keeping both directions as separate
    /// constraints means each gets its own dependents entry. Test-only: the
    /// collector emits the two `FlowsInto` edges directly.
    #[cfg(test)]
    pub fn add_equal(&mut self, a: TypeKey, b: TypeKey) {
        self.add(Constraint::FlowsInto { src: a, dst: b });
        self.add(Constraint::FlowsInto { src: b, dst: a });
    }

    /// Direct env access — used by materialization.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Test-only inspection of the registered constraint list.
    #[cfg(test)]
    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    /// Drive the env to fixpoint by repeatedly evaluating constraints whose
    /// inputs have changed.
    ///
    /// Algorithm:
    /// 1. Seed the queue with every constraint (initial pass).
    /// 2. Pop a constraint, evaluate it, JOIN its result into the
    ///    destination key. If the destination's env value changed,
    ///    enqueue every dependent constraint.
    /// 3. Repeat until the queue empties.
    ///
    /// Termination: every JOIN that returns `true` strictly increases the
    /// destination's lattice value. The `Type` lattice has finite height
    /// (≈ 5 levels for primitives, recursively bounded for containers),
    /// so each key changes at most O(height) times. With `|C|` constraints
    /// and `|K|` keys, the queue empties in O(|C| × height(|K|)) steps —
    /// no `cap=10` hack needed.
    pub fn run<C: ReducerCtx>(&mut self, ctx: &C) {
        // Seed queue with every constraint id, in stable insertion order.
        let mut queue: VecDeque<ConstraintId> = (0..self.constraints.len() as u32)
            .map(ConstraintId)
            .collect();
        // Membership set to avoid duplicate enqueues (a key can have many
        // dependents that all share the same constraint as an upstream
        // input — without this guard the queue grows quadratically).
        let mut on_queue: IndexSet<ConstraintId> = queue.iter().copied().collect();

        // Backstop against non-termination. The depth cap in
        // `Env::join_into` bounds container nesting, which is the known
        // unbounded-height hazard; this counter is defense-in-depth for any
        // other growth source (e.g. Union arity). The bound is far above
        // the worst-case monotone-convergence cost (each key rises a
        // bounded number of lattice levels), so it never trips on
        // well-behaved input. If it ever fires, leaving the env at its
        // current (sound, monotone) state degrades precision but keeps the
        // compiler responsive.
        let update_cap: u64 = (self.constraints.len() as u64).saturating_mul(256) + 100_000;
        let mut updates: u64 = 0;

        while let Some(cid) = queue.pop_front() {
            on_queue.swap_remove(&cid);

            let constraint = &self.constraints[cid.0 as usize];
            let Some((dst, new_val)) = evaluate(&self.env, constraint, ctx) else {
                continue;
            };

            if !self.env.join_into(dst, new_val) {
                continue;
            }

            updates += 1;
            if updates > update_cap {
                debug_assert!(
                    false,
                    "constraint solver exceeded update cap ({update_cap}) — \
                     likely an unbounded type-growth cycle not caught by the \
                     depth cap"
                );
                break;
            }

            // Destination changed → schedule every constraint that reads
            // this key. Clone the set so we can drop the immutable borrow
            // on `self.deps` before re-entering the queue loop (which
            // requires `&mut self` via `env`).
            if let Some(deps) = self.deps.get(&dst) {
                for &dep_cid in deps {
                    if on_queue.insert(dep_cid) {
                        queue.push_back(dep_cid);
                    }
                }
            }
        }
    }
}

impl Default for Solver {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate one constraint against the current env. Returns
/// `Some((dst, new_value))` for the JOIN target, or `None` if this
/// constraint produces no env update (deferred reducer not yet
/// implemented, or input keys haven't been bound).
///
/// Pure function of `(env, constraint, ctx)` — does not mutate env.
fn evaluate<C: ReducerCtx>(env: &Env, c: &Constraint, ctx: &C) -> Option<(TypeKey, Type)> {
    match c {
        Constraint::Concrete(k, ty) => Some((*k, ty.clone())),
        Constraint::FlowsInto { src, dst } => Some((*dst, env.get(*src))),
        Constraint::Equal(a, b) => {
            // Equal is normally expanded by `Solver::add_equal` into two
            // FlowsInto edges. If raw Equal(a, b) was added directly, we
            // implement it as a → b here (the b → a direction is the
            // caller's responsibility to add separately). This matches
            // the "two FlowsInto" expansion semantically.
            Some((*b, env.get(*a)))
        }
        Constraint::Return { func, value } => Some((TypeKey::FuncReturn(*func), env.get(*value))),
        Constraint::Yield { func, value } => Some((TypeKey::FuncYield(*func), env.get(*value))),
        Constraint::GeneratorReturn { func } => {
            // Wrap the (possibly still-`Never`) yield element so callers
            // see `Iterator[T]` during solving. Covariant `join` then
            // sharpens `Iterator[Never]` → `Iterator[T]` as the yield
            // type resolves on later worklist passes.
            let yielded = env.get(TypeKey::FuncYield(*func));
            Some((
                TypeKey::FuncReturn(*func),
                Type::Iterator(Box::new(yielded)),
            ))
        }
        Constraint::WrapIterator { result, elem } => {
            Some((*result, Type::Iterator(Box::new(env.get(*elem)))))
        }
        Constraint::TupleProject {
            result,
            tuple,
            index,
        } => {
            let t = env.get(*tuple);
            // Fixed tuple: per-position element. Variable tuple: the
            // homogeneous element for any index. Anything else (not yet
            // resolved to a tuple, or genuinely not a tuple) defers —
            // returning `None` leaves the result at its current value so
            // a later pass can sharpen it once `tuple` resolves.
            let projected = if let Some(elems) = t.tuple_elems() {
                elems.get(*index).cloned()
            } else {
                t.tuple_var_elem().cloned()
            };
            projected.map(|ty| (*result, ty))
        }
        Constraint::FieldWrite { class, name, value } => {
            Some((TypeKey::ClassField(*class, *name), env.get(*value)))
        }
        Constraint::Capture { func, slot, src } => {
            Some((TypeKey::Capture(*func, *slot), env.get(*src)))
        }
        Constraint::LambdaParamHint {
            func,
            param_ix,
            hint,
        } => Some((TypeKey::LambdaParam(*func, *param_ix), env.get(*hint))),
        Constraint::BinOp {
            result,
            op,
            lhs,
            rhs,
        } => Some((*result, eval_binop(env, op, *lhs, *rhs, ctx))),
        Constraint::UnaryOp {
            result,
            op,
            operand,
        } => Some((*result, eval_unop(env, *op, *operand))),
        Constraint::ContainerLiteral {
            result,
            kind,
            elems,
            kv,
        } => Some((*result, eval_container_literal(env, *kind, elems, kv))),

        Constraint::Call {
            result,
            callee,
            args,
            kwargs,
        } => Some((*result, eval_call(env, callee, args, kwargs, ctx))),
        Constraint::MethodCall {
            result,
            recv,
            name,
            args,
        } => Some((*result, eval_method_call(env, *recv, *name, args, ctx))),
        Constraint::Attribute { result, recv, name } => {
            Some((*result, eval_attribute(env, *recv, *name, ctx)))
        }
        Constraint::Subscript {
            result,
            recv,
            index,
        } => Some((*result, eval_subscript(env, *recv, *index, ctx))),
        Constraint::IterElem { result, iter } => Some((*result, eval_iter_elem(env, *iter, ctx))),
    }
}

/// Resolve a `Call` constraint.
///
/// `CalleeRef::Func(fid)` reads `env[FuncReturn(fid)]` directly — the
/// solver bootstraps itself: as `Return` constraints land, FuncReturn
/// updates, and every dependent `Call` is rescheduled by the worklist.
///
/// `CalleeRef::Dynamic(k)` reads the callee's env type. If it's a
/// `Type::Function`, the return type is the `ret` field; otherwise the
/// call result widens to `Any`. Lambda dispatch eventually narrows this
/// once `LambdaParam`/`Capture` constraints settle.
///
/// `CalleeRef::Builtin` and `CalleeRef::ClassCtor` defer entirely to the
/// reducer context — built-in signatures and class-name lookups need
/// access to module state that the solver doesn't carry.
fn eval_call<C: ReducerCtx>(
    env: &Env,
    callee: &CalleeRef,
    args: &[TypeKey],
    kwargs: &[(InternedString, TypeKey)],
    ctx: &C,
) -> Type {
    match callee {
        CalleeRef::Func(fid) => env.get(TypeKey::FuncReturn(*fid)),
        CalleeRef::Dynamic(callee_key) => match env.get(*callee_key) {
            // Callee type not yet computed — defer rather than collapsing
            // to `Any`. (See note in `eval_binop` for why early `Any` is
            // catastrophic in a monotone JOIN lattice.)
            Type::Never => Type::Never,
            Type::Function { ret, .. } => (*ret).clone(),
            // Any / Var / non-function callee: result is Any. The solver
            // cannot narrow further without a more specific signature.
            _ => Type::Any,
        },
        CalleeRef::Builtin(b) => {
            let arg_types: Vec<Type> = args.iter().map(|k| env.get(*k)).collect();
            // If any arg is still bottom, defer — production builtin
            // signatures may depend on argument types (e.g. `iter(xs)`
            // returns `Iterator[T]`).
            if arg_types.iter().any(|t| matches!(t, Type::Never)) {
                return Type::Never;
            }
            match ctx.builtin_return(*b, &arg_types) {
                Some(t) => t,
                // The builtin couldn't resolve from the current arg
                // types. If an arg is a container/iterator whose element
                // is still bottom (e.g. `Iterator[Never]` from a not-yet-
                // solved generator yield), the resolution is transitional:
                // defer with `Never` rather than polluting with `Any`.
                // `Any` absorbs in the monotone lattice, so an early
                // element-bottom widening of a reduction like
                // `sum(x for x in gen)` would pin the result to `Any`
                // forever, even after the yield type sharpens to `V`.
                // Element-agnostic builtins (`len`, etc.) always return
                // `Some(_)`, so they never hit this defer path.
                None if arg_types.iter().any(element_is_bottom) => Type::Never,
                None => Type::Any,
            }
        }
        CalleeRef::ClassCtor(class_id) => {
            // kwargs are part of the constructor signature but don't
            // affect the return type — they're consumed to populate
            // instance fields, not to compute the result. Drop them.
            let _ = kwargs;
            ctx.class_ctor_return(*class_id).unwrap_or(Type::Any)
        }
    }
}

/// Resolve `recv.method(args)`. Built-in receiver types (str/list/dict/
/// set/tuple) have a known method table that the `Lowering` impl of
/// `ReducerCtx` consults via `resolve_method_on_type`. The solver itself
/// is type-agnostic here — it just forwards env types to `method_return`.
fn eval_method_call<C: ReducerCtx>(
    env: &Env,
    recv: TypeKey,
    name: InternedString,
    args: &[TypeKey],
    ctx: &C,
) -> Type {
    let recv_ty = env.get(recv);
    if matches!(recv_ty, Type::Never) {
        return Type::Never;
    }
    let arg_types: Vec<Type> = args.iter().map(|k| env.get(*k)).collect();
    ctx.method_return(&recv_ty, name, &arg_types)
        .unwrap_or(Type::Any)
}

/// Resolve `recv.attr`.
///
/// When `recv` resolves to `Type::Class { class_id, .. }`, the reducer
/// first consults the solver's own `ClassField(class_id, attr)` key —
/// this is the unified routing for `self.x` reads inside methods and
/// `instance.field` reads across the program. If the field has never
/// been written (key is `Never`), the reducer falls through to the
/// reducer context, which handles methods, properties, and built-in
/// attribute lookups.
///
/// The static dependency edge from `ClassField(class_id, attr)` to the
/// `Attribute` constraint is registered by [`inputs_of`] when the
/// collector emits an Attribute on a `Var` whose declared class is
/// known. For dynamic cases (recv resolves to Class via cross-function
/// inference), the reducer still reads the correct ClassField value —
/// it just won't be re-scheduled if that value updates later. The
/// static path covers the common `self.x` case.
fn eval_attribute<C: ReducerCtx>(env: &Env, recv: TypeKey, attr: InternedString, ctx: &C) -> Type {
    let recv_ty = env.get(recv);
    if matches!(recv_ty, Type::Never) {
        return Type::Never;
    }
    // Class field path: solver-internal lookup before delegating to ctx.
    if let Type::Class { class_id, .. } = &recv_ty {
        let field_ty = env.get(TypeKey::ClassField(*class_id, attr));
        if !matches!(field_ty, Type::Never) {
            return field_ty;
        }
        // `ClassField` not set yet. If the class DECLARES this instance
        // field, DEFER (return `Never`) rather than widening to `Any` via
        // the ctx: the `FieldWrite` reducer will set the field type later,
        // and the collector's `ClassField → Attribute` dependency edge
        // re-triggers this read. Widening to `Any` here is unrecoverable —
        // `Any` is the lattice top, so a later `JOIN(Any, Float)` stays
        // `Any` and the read is permanently poisoned (the `keys[0][0].data`
        // cross-function field-read bug). Methods / properties / built-in
        // attributes are NOT instance fields, so they fall through to the
        // ctx as before.
        if ctx.class_has_instance_field(*class_id, attr) {
            return Type::Never;
        }
    }
    ctx.attribute_return(&recv_ty, attr).unwrap_or(Type::Any)
}

/// Resolve `recv[index]`. Handles built-in containers inline (no ctx
/// needed for `list[T] → T`, `dict[K,V] → V`, etc.); falls back to
/// `ctx.subscript_return` for user-defined `__getitem__`.
///
/// Tuple subscript is homogenized to the union of all element types —
/// per-position precision requires knowing the index is a constant
/// literal, which the constraint layer doesn't expose. The legacy
/// planner gets per-position via the HIR; we accept this small loss of
/// precision in S3 and tighten it in S6 if needed.
fn eval_subscript<C: ReducerCtx>(env: &Env, recv: TypeKey, index: TypeKey, ctx: &C) -> Type {
    let recv_ty = env.get(recv);
    let index_ty = env.get(index);
    if matches!(recv_ty, Type::Never) {
        return Type::Never;
    }

    if let Some(t) = recv_ty.list_elem() {
        return t.clone();
    }
    if let Some((_, v)) = recv_ty.dict_kv() {
        return v.clone();
    }
    if let Some(t) = recv_ty.set_elem() {
        // `set[T]` doesn't support subscription in Python, but ranges
        // and other iterators that present as `set_elem` may. Return
        // the element type rather than fail — matches the legacy
        // resolver's permissive fallback.
        return t.clone();
    }
    if let Some(elems) = recv_ty.tuple_elems() {
        // Homogenize — see fn doc.
        return elems.iter().fold(Type::bottom(), |acc, t| acc.join(t));
    }
    if let Some(t) = recv_ty.tuple_var_elem() {
        return t.clone();
    }
    if matches!(recv_ty, Type::Str) {
        return Type::Str;
    }
    if matches!(recv_ty, Type::Bytes) {
        return Type::Int;
    }
    // User class with __getitem__, or unknown.
    ctx.subscript_return(&recv_ty, &index_ty)
        .unwrap_or(Type::Any)
}

/// Resolve the element type for an `IterElem` constraint. Built-in
/// iterables are handled by [`default_iter_elem`]; the ctx provides
/// user-class `__iter__` dispatch when needed.
fn eval_iter_elem<C: ReducerCtx>(env: &Env, iter: TypeKey, ctx: &C) -> Type {
    let iter_ty = env.get(iter);
    if matches!(iter_ty, Type::Never) {
        return Type::Never;
    }
    ctx.iter_elem(&iter_ty).unwrap_or(Type::Any)
}

fn eval_binop<C: ReducerCtx>(
    env: &Env,
    op: &pyaot_hir::BinOp,
    lhs: TypeKey,
    rhs: TypeKey,
    ctx: &C,
) -> Type {
    let lt = env.get(lhs);
    let rt = env.get(rhs);
    // Defer evaluation while either operand is still bottom — the resolver
    // returning `None` on a `Never` operand would mistakenly widen the
    // result to `Any`, and `Any` absorbs in the lattice, so the worklist
    // could never narrow back when the operand becomes concrete. Returning
    // `Never` lets the reducer re-evaluate on the next propagation tick.
    if matches!(lt, Type::Never) || matches!(rt, Type::Never) {
        return Type::Never;
    }
    // Defer to the existing pure resolver in `helpers`. The class-dunder
    // callback consults the production Lowering in S5 wire-in time; the
    // permissive ctx used by unit tests accepts every dunder.
    let check =
        |class_id: ClassId, dunder: &str| -> bool { ctx.class_has_dunder(class_id, dunder) };
    helpers::resolve_binop_type_class_aware(op, &lt, &rt, &check).unwrap_or(Type::Any)
}

fn eval_unop(env: &Env, op: UnOp, operand: TypeKey) -> Type {
    let op_ty = env.get(operand);
    match op {
        UnOp::Not => Type::Bool,
        UnOp::Neg | UnOp::Pos => op_ty,
        UnOp::Invert => Type::Int,
    }
}

fn eval_container_literal(
    env: &Env,
    kind: ContainerKind,
    elems: &[TypeKey],
    kv: &[(TypeKey, TypeKey)],
) -> Type {
    match kind {
        ContainerKind::List => {
            let elem_ty = join_keys(env, elems);
            Type::list_of(elem_ty)
        }
        ContainerKind::Set => {
            let elem_ty = join_keys(env, elems);
            Type::set_of(elem_ty)
        }
        ContainerKind::Tuple => {
            // Fixed-length heterogeneous tuple: preserve per-position
            // types so `(1, "a", 3.0)[0]` can stay typed `Int`.
            let elem_types: Vec<Type> = elems.iter().map(|k| env.get(*k)).collect();
            Type::tuple_of(elem_types)
        }
        ContainerKind::Dict => {
            let k_ty = kv
                .iter()
                .fold(Type::bottom(), |acc, (k, _)| acc.join(&env.get(*k)));
            let v_ty = kv
                .iter()
                .fold(Type::bottom(), |acc, (_, v)| acc.join(&env.get(*v)));
            Type::dict_of(k_ty, v_ty)
        }
    }
}

fn join_keys(env: &Env, keys: &[TypeKey]) -> Type {
    keys.iter()
        .fold(Type::bottom(), |acc, k| acc.join(&env.get(*k)))
}

/// Enumerate the keys this constraint reads (its dependencies). The
/// `result`/destination key is NOT an input — only the keys whose env
/// values feed into the reducer's evaluation. JOIN-only constraints
/// (`Concrete`, `FieldWrite`, `Return`, `Yield`, etc.) read no env
/// state — their value is statically encoded in the constraint.
pub(crate) fn inputs_of(c: &Constraint) -> Vec<TypeKey> {
    match c {
        Constraint::Concrete(_, _) => Vec::new(),
        Constraint::FlowsInto { src, .. } => vec![*src],
        Constraint::Equal(a, b) => vec![*a, *b],
        Constraint::BinOp { lhs, rhs, .. } => vec![*lhs, *rhs],
        Constraint::UnaryOp { operand, .. } => vec![*operand],
        Constraint::Call {
            callee,
            args,
            kwargs,
            ..
        } => {
            let mut out: Vec<TypeKey> = args.clone();
            for (_, k) in kwargs {
                out.push(*k);
            }
            // `CalleeRef::Func(fid)` reads `env[FuncReturn(fid)]` in
            // `eval_call`. Without this dependency edge the worklist
            // would never reschedule a `Call` when its callee's return
            // type updates — the entire self-bootstrap argument
            // depends on this.
            match callee {
                CalleeRef::Func(fid) => out.push(TypeKey::FuncReturn(*fid)),
                CalleeRef::Dynamic(k) => out.push(*k),
                CalleeRef::Builtin(_) | CalleeRef::ClassCtor(_) => {}
            }
            out
        }
        Constraint::MethodCall { recv, args, .. } => {
            let mut out = vec![*recv];
            out.extend(args.iter().copied());
            out
        }
        Constraint::Attribute { recv, .. } => vec![*recv],
        Constraint::Subscript { recv, index, .. } => vec![*recv, *index],
        Constraint::ContainerLiteral { elems, kv, .. } => {
            let mut out: Vec<TypeKey> = elems.clone();
            for (k, v) in kv {
                out.push(*k);
                out.push(*v);
            }
            out
        }
        Constraint::IterElem { iter, .. } => vec![*iter],
        Constraint::WrapIterator { elem, .. } => vec![*elem],
        Constraint::TupleProject { tuple, .. } => vec![*tuple],
        Constraint::FieldWrite { value, .. } => vec![*value],
        Constraint::LambdaParamHint { hint, .. } => vec![*hint],
        Constraint::Capture { src, .. } => vec![*src],
        Constraint::Return { value, .. } => vec![*value],
        Constraint::Yield { value, .. } => vec![*value],
        Constraint::GeneratorReturn { func } => vec![TypeKey::FuncYield(*func)],
    }
}

#[cfg(test)]
mod tests {
    use super::super::vocab::{Constraint, ConstraintId};
    use super::*;
    use pyaot_hir::{BinOp as HirBinOp, ExprId, UnOp as HirUnOp};
    use pyaot_types::Type;
    use pyaot_utils::{FuncId, VarId};

    fn ek(i: u32) -> TypeKey {
        TypeKey::Expr(ExprId::from_raw(i.into()))
    }

    // -----------------------------------------------------------------
    // S1 carry-over: structural / registration tests.
    // -----------------------------------------------------------------

    #[test]
    fn fresh_meta_keys_are_distinct() {
        let mut s = Solver::new();
        let a = s.fresh_meta();
        let b = s.fresh_meta();
        let c = s.fresh_meta();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn add_registers_dependents_for_input_keys() {
        let mut s = Solver::new();
        let cid_concrete = s.add(Constraint::Concrete(ek(0), Type::Int));
        let cid_flow = s.add(Constraint::FlowsInto {
            src: ek(0),
            dst: ek(1),
        });
        let deps_of_0 = s.deps.get(&ek(0)).expect("ek(0) has dependents");
        assert!(!deps_of_0.contains(&cid_concrete));
        assert!(deps_of_0.contains(&cid_flow));
        assert_eq!(s.constraints().len(), 2);
    }

    #[test]
    fn add_registers_multiple_dependents() {
        let mut s = Solver::new();
        let lhs = ek(0);
        let rhs = ek(1);
        let result = ek(2);
        let cid = s.add(Constraint::BinOp {
            result,
            op: HirBinOp::Add,
            lhs,
            rhs,
        });
        assert!(s.deps.get(&lhs).unwrap().contains(&cid));
        assert!(s.deps.get(&rhs).unwrap().contains(&cid));
        assert!(s.deps.get(&result).is_none());
    }

    #[test]
    fn constraint_id_monotone() {
        let mut s = Solver::new();
        let id0 = s.add(Constraint::Concrete(ek(0), Type::Int));
        let id1 = s.add(Constraint::Concrete(ek(1), Type::Int));
        let id2 = s.add(Constraint::Concrete(ek(2), Type::Int));
        assert_eq!(id0, ConstraintId(0));
        assert_eq!(id1, ConstraintId(1));
        assert_eq!(id2, ConstraintId(2));
    }

    // -----------------------------------------------------------------
    // S2: end-to-end run() behaviour on synthetic constraint graphs.
    // -----------------------------------------------------------------

    #[test]
    fn run_concrete_seeds_env() {
        let mut s = Solver::new();
        s.add(Constraint::Concrete(ek(0), Type::Int));
        s.add(Constraint::Concrete(ek(1), Type::Str));
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(ek(0)), Type::Int);
        assert_eq!(s.env().get(ek(1)), Type::Str);
    }

    #[test]
    fn run_propagates_along_flows_into_chain() {
        // Build chain: ek(0) = Int → ek(1) → ek(2) → ek(3).
        // Expected: every key ends up as Int.
        let mut s = Solver::new();
        s.add(Constraint::Concrete(ek(0), Type::Int));
        s.add(Constraint::FlowsInto {
            src: ek(0),
            dst: ek(1),
        });
        s.add(Constraint::FlowsInto {
            src: ek(1),
            dst: ek(2),
        });
        s.add(Constraint::FlowsInto {
            src: ek(2),
            dst: ek(3),
        });
        s.run(&PermissiveCtx);
        for i in 0..=3 {
            assert_eq!(s.env().get(ek(i)), Type::Int, "key {i}");
        }
    }

    #[test]
    fn run_joins_two_inputs_into_union_then_widens_to_top() {
        // ek(0)=Int, ek(1)=Str feed into ek(2).
        // Expected: ek(2) = Union[Int, Str].
        // Then we add ek(3)=Any → ek(2). Expected: ek(2) = Any.
        let mut s = Solver::new();
        s.add(Constraint::Concrete(ek(0), Type::Int));
        s.add(Constraint::Concrete(ek(1), Type::Str));
        s.add(Constraint::FlowsInto {
            src: ek(0),
            dst: ek(2),
        });
        s.add(Constraint::FlowsInto {
            src: ek(1),
            dst: ek(2),
        });
        s.run(&PermissiveCtx);
        match s.env().get(ek(2)) {
            Type::Union(members) => {
                assert_eq!(members.len(), 2);
                assert!(members.contains(&Type::Int));
                assert!(members.contains(&Type::Str));
            }
            other => panic!("expected Union, got {other:?}"),
        }

        s.add(Constraint::Concrete(ek(3), Type::Any));
        s.add(Constraint::FlowsInto {
            src: ek(3),
            dst: ek(2),
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(ek(2)), Type::Any);
    }

    #[test]
    fn run_binop_primitive_numeric_tower() {
        // Synthesize: result = lhs + rhs, lhs=Int, rhs=Float
        // Expected: Float (numeric tower).
        let mut s = Solver::new();
        let lhs = ek(0);
        let rhs = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(lhs, Type::Int));
        s.add(Constraint::Concrete(rhs, Type::Float));
        s.add(Constraint::BinOp {
            result,
            op: HirBinOp::Add,
            lhs,
            rhs,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Float);
    }

    #[test]
    fn run_binop_str_concat() {
        let mut s = Solver::new();
        let lhs = ek(0);
        let rhs = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(lhs, Type::Str));
        s.add(Constraint::Concrete(rhs, Type::Str));
        s.add(Constraint::BinOp {
            result,
            op: HirBinOp::Add,
            lhs,
            rhs,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Str);
    }

    #[test]
    fn run_binop_reacts_when_operand_widens_later() {
        // Initial: lhs=Int, rhs=Int → result=Int.
        // Then: add a second feed making lhs widen to Float.
        // Expected: result rescheduled, becomes Float.
        let mut s = Solver::new();
        let lhs = ek(0);
        let rhs = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(lhs, Type::Int));
        s.add(Constraint::Concrete(rhs, Type::Int));
        s.add(Constraint::BinOp {
            result,
            op: HirBinOp::Add,
            lhs,
            rhs,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Int);

        // Widen lhs by adding a Float source flowing into it.
        s.add(Constraint::Concrete(ek(99), Type::Float));
        s.add(Constraint::FlowsInto {
            src: ek(99),
            dst: lhs,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(lhs), Type::Float);
        assert_eq!(
            s.env().get(result),
            Type::Float,
            "BinOp must be re-scheduled when an operand widens"
        );
    }

    #[test]
    fn run_unary_neg_preserves_operand_type() {
        let mut s = Solver::new();
        let operand = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(operand, Type::Float));
        s.add(Constraint::UnaryOp {
            result,
            op: HirUnOp::Neg,
            operand,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Float);
    }

    #[test]
    fn run_unary_not_returns_bool() {
        let mut s = Solver::new();
        let operand = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(operand, Type::Int));
        s.add(Constraint::UnaryOp {
            result,
            op: HirUnOp::Not,
            operand,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Bool);
    }

    #[test]
    fn run_container_literal_list_homogeneous() {
        // result = [Int, Int, Int]  →  list[Int]
        let mut s = Solver::new();
        let e0 = ek(0);
        let e1 = ek(1);
        let e2 = ek(2);
        let result = ek(10);
        s.add(Constraint::Concrete(e0, Type::Int));
        s.add(Constraint::Concrete(e1, Type::Int));
        s.add(Constraint::Concrete(e2, Type::Int));
        s.add(Constraint::ContainerLiteral {
            result,
            kind: ContainerKind::List,
            elems: vec![e0, e1, e2],
            kv: Vec::new(),
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::list_of(Type::Int));
    }

    #[test]
    fn run_container_literal_list_heterogeneous_widens_via_numeric_tower() {
        // [Int, Float, Int]  →  list[Float] via numeric tower.
        let mut s = Solver::new();
        let e0 = ek(0);
        let e1 = ek(1);
        let e2 = ek(2);
        let result = ek(10);
        s.add(Constraint::Concrete(e0, Type::Int));
        s.add(Constraint::Concrete(e1, Type::Float));
        s.add(Constraint::Concrete(e2, Type::Int));
        s.add(Constraint::ContainerLiteral {
            result,
            kind: ContainerKind::List,
            elems: vec![e0, e1, e2],
            kv: Vec::new(),
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::list_of(Type::Float));
    }

    #[test]
    fn run_container_literal_tuple_preserves_per_position_types() {
        // (Int, Str, Float)  →  tuple[Int, Str, Float] (fixed shape, NOT widened).
        let mut s = Solver::new();
        let e0 = ek(0);
        let e1 = ek(1);
        let e2 = ek(2);
        let result = ek(10);
        s.add(Constraint::Concrete(e0, Type::Int));
        s.add(Constraint::Concrete(e1, Type::Str));
        s.add(Constraint::Concrete(e2, Type::Float));
        s.add(Constraint::ContainerLiteral {
            result,
            kind: ContainerKind::Tuple,
            elems: vec![e0, e1, e2],
            kv: Vec::new(),
        });
        s.run(&PermissiveCtx);
        assert_eq!(
            s.env().get(result),
            Type::tuple_of(vec![Type::Int, Type::Str, Type::Float])
        );
    }

    #[test]
    fn run_container_literal_dict_joins_keys_and_values() {
        // {Str: Int, Str: Float}  →  dict[Str, Float]
        let mut s = Solver::new();
        let k0 = ek(0);
        let v0 = ek(1);
        let k1 = ek(2);
        let v1 = ek(3);
        let result = ek(10);
        s.add(Constraint::Concrete(k0, Type::Str));
        s.add(Constraint::Concrete(v0, Type::Int));
        s.add(Constraint::Concrete(k1, Type::Str));
        s.add(Constraint::Concrete(v1, Type::Float));
        s.add(Constraint::ContainerLiteral {
            result,
            kind: ContainerKind::Dict,
            elems: Vec::new(),
            kv: vec![(k0, v0), (k1, v1)],
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::dict_of(Type::Str, Type::Float));
    }

    #[test]
    fn run_return_joins_into_func_return_key() {
        // Two return sites in func 0: Int and Float → FuncReturn(0) = Float.
        let mut s = Solver::new();
        let fid = FuncId::new(0);
        let r0 = ek(0);
        let r1 = ek(1);
        s.add(Constraint::Concrete(r0, Type::Int));
        s.add(Constraint::Concrete(r1, Type::Float));
        s.add(Constraint::Return {
            func: fid,
            value: r0,
        });
        s.add(Constraint::Return {
            func: fid,
            value: r1,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(TypeKey::FuncReturn(fid)), Type::Float);
    }

    #[test]
    fn run_field_write_accumulates_across_writes() {
        let mut s = Solver::new();
        let cls = ClassId::new(7);
        // Build a real InternedString through a StringInterner so the
        // key is well-defined.
        let mut interner = pyaot_utils::StringInterner::new();
        let name = interner.intern("acc");
        let v0 = ek(0);
        let v1 = ek(1);
        s.add(Constraint::Concrete(v0, Type::Int));
        s.add(Constraint::Concrete(v1, Type::Float));
        s.add(Constraint::FieldWrite {
            class: cls,
            name,
            value: v0,
        });
        s.add(Constraint::FieldWrite {
            class: cls,
            name,
            value: v1,
        });
        s.run(&PermissiveCtx);
        assert_eq!(
            s.env().get(TypeKey::ClassField(cls, name)),
            Type::Float,
            "field type widens through numeric tower across stores"
        );
        // Suppress unused-mut warning on interner.
        let _ = interner.len();
    }

    #[test]
    fn run_capture_joins_into_capture_key() {
        let mut s = Solver::new();
        let fid = FuncId::new(0);
        let src = TypeKey::Var(VarId::new(0));
        let cap = TypeKey::Capture(fid, 0);
        s.add(Constraint::Concrete(src, Type::Int));
        s.add(Constraint::Capture {
            func: fid,
            slot: 0,
            src,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(cap), Type::Int);
    }

    #[test]
    fn run_lambda_param_hint_joins_into_lambda_param_key() {
        let mut s = Solver::new();
        let fid = FuncId::new(0);
        let hint = ek(0);
        s.add(Constraint::Concrete(hint, Type::Int));
        s.add(Constraint::LambdaParamHint {
            func: fid,
            param_ix: 0,
            hint,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(TypeKey::LambdaParam(fid, 0)), Type::Int);
    }

    #[test]
    fn run_equal_via_add_equal_propagates_both_ways() {
        // Equal(a, b): a = Int initially. After run, b should also be Int.
        // Then set b = Float; after re-run, a should widen to Float.
        let mut s = Solver::new();
        let a = ek(0);
        let b = ek(1);
        s.add(Constraint::Concrete(a, Type::Int));
        s.add_equal(a, b);
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(a), Type::Int);
        assert_eq!(s.env().get(b), Type::Int);

        s.add(Constraint::Concrete(b, Type::Float));
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(a), Type::Float, "Equal propagates b → a too");
        assert_eq!(s.env().get(b), Type::Float);
    }

    #[test]
    fn run_terminates_on_cyclic_flows() {
        // ek(0) → ek(1) → ek(0): without on_queue dedup this would loop.
        let mut s = Solver::new();
        s.add(Constraint::Concrete(ek(0), Type::Int));
        s.add(Constraint::FlowsInto {
            src: ek(0),
            dst: ek(1),
        });
        s.add(Constraint::FlowsInto {
            src: ek(1),
            dst: ek(0),
        });
        s.run(&PermissiveCtx);
        // Both keys settle on Int (the unique fixpoint).
        assert_eq!(s.env().get(ek(0)), Type::Int);
        assert_eq!(s.env().get(ek(1)), Type::Int);
    }

    #[test]
    fn run_idempotent_re_running_changes_nothing() {
        let mut s = Solver::new();
        s.add(Constraint::Concrete(ek(0), Type::Int));
        s.add(Constraint::FlowsInto {
            src: ek(0),
            dst: ek(1),
        });
        s.run(&PermissiveCtx);
        let snapshot: Vec<_> = s.env().iter().map(|(k, v)| (*k, v.clone())).collect();
        s.run(&PermissiveCtx);
        let after: Vec<_> = s.env().iter().map(|(k, v)| (*k, v.clone())).collect();
        assert_eq!(snapshot, after, "re-running on a fixpoint is a no-op");
    }

    // -----------------------------------------------------------------
    // S3: compound reducers (Call / MethodCall / Attribute / Subscript /
    // IterElem).
    // -----------------------------------------------------------------

    use super::super::vocab::{BuiltinId, CalleeRef};
    use std::sync::Mutex;

    /// Programmable test context. Records every ctx call so tests can
    /// verify the trait was actually consulted; returns pre-set types
    /// for each operation key.
    ///
    /// `Type` doesn't implement `Hash`, so the lookup tables are
    /// `Vec<((key…), value)>` with linear search — fine for the small
    /// test inputs we use.
    #[derive(Default)]
    struct TestCtx {
        method_table: Vec<((Type, InternedString), Type)>,
        attribute_table: Vec<((Type, InternedString), Type)>,
        subscript_table: Vec<((Type, Type), Type)>,
        builtin_table: Vec<(BuiltinId, Type)>,
        class_ctor_table: Vec<(ClassId, Type)>,
        method_calls: Mutex<Vec<(Type, InternedString, Vec<Type>)>>,
    }

    impl TestCtx {
        fn add_method(&mut self, recv: Type, method: InternedString, ret: Type) {
            self.method_table.push(((recv, method), ret));
        }
        fn add_attribute(&mut self, recv: Type, attr: InternedString, ret: Type) {
            self.attribute_table.push(((recv, attr), ret));
        }
        fn add_subscript(&mut self, recv: Type, index: Type, ret: Type) {
            self.subscript_table.push(((recv, index), ret));
        }
        fn add_builtin(&mut self, b: BuiltinId, ret: Type) {
            self.builtin_table.push((b, ret));
        }
        fn add_class_ctor(&mut self, class_id: ClassId, ret: Type) {
            self.class_ctor_table.push((class_id, ret));
        }
    }

    impl ReducerCtx for TestCtx {
        fn class_has_dunder(&self, _: ClassId, _: &str) -> bool {
            true
        }
        fn method_return(
            &self,
            recv: &Type,
            method: InternedString,
            args: &[Type],
        ) -> Option<Type> {
            self.method_calls
                .lock()
                .unwrap()
                .push((recv.clone(), method, args.to_vec()));
            self.method_table
                .iter()
                .find(|((r, m), _)| r == recv && *m == method)
                .map(|(_, ret)| ret.clone())
        }
        fn attribute_return(&self, recv: &Type, attr: InternedString) -> Option<Type> {
            self.attribute_table
                .iter()
                .find(|((r, a), _)| r == recv && *a == attr)
                .map(|(_, ret)| ret.clone())
        }
        fn subscript_return(&self, recv: &Type, index: &Type) -> Option<Type> {
            self.subscript_table
                .iter()
                .find(|((r, i), _)| r == recv && i == index)
                .map(|(_, ret)| ret.clone())
        }
        fn builtin_return(&self, b: BuiltinId, _args: &[Type]) -> Option<Type> {
            self.builtin_table
                .iter()
                .find(|(k, _)| *k == b)
                .map(|(_, ret)| ret.clone())
        }
        fn class_ctor_return(&self, class_id: ClassId) -> Option<Type> {
            self.class_ctor_table
                .iter()
                .find(|(k, _)| *k == class_id)
                .map(|(_, ret)| ret.clone())
        }
    }

    #[test]
    fn run_call_func_reads_from_func_return_key() {
        let mut s = Solver::new();
        let fid = FuncId::new(42);
        let result = ek(0);
        // Seed FuncReturn(fid) = Int via a Return constraint.
        let return_value = ek(1);
        s.add(Constraint::Concrete(return_value, Type::Int));
        s.add(Constraint::Return {
            func: fid,
            value: return_value,
        });
        s.add(Constraint::Call {
            result,
            callee: CalleeRef::Func(fid),
            args: Vec::new(),
            kwargs: Vec::new(),
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Int);
    }

    #[test]
    fn run_call_func_reschedules_when_func_return_widens() {
        // Order matters: register Call BEFORE Return so the worklist
        // has to reschedule the Call when FuncReturn updates.
        let mut s = Solver::new();
        let fid = FuncId::new(7);
        let result = ek(0);
        s.add(Constraint::Call {
            result,
            callee: CalleeRef::Func(fid),
            args: Vec::new(),
            kwargs: Vec::new(),
        });
        // Now plumb a Return into FuncReturn — Call must reschedule.
        let rv = ek(1);
        s.add(Constraint::Concrete(rv, Type::Float));
        s.add(Constraint::Return {
            func: fid,
            value: rv,
        });
        s.run(&PermissiveCtx);
        assert_eq!(
            s.env().get(result),
            Type::Float,
            "Call(Func) must be rescheduled when FuncReturn widens"
        );
    }

    #[test]
    fn run_call_dynamic_with_function_type_uses_ret() {
        let mut s = Solver::new();
        let callee_key = TypeKey::Var(VarId::new(0));
        let result = ek(0);
        s.add(Constraint::Concrete(
            callee_key,
            Type::Function {
                params: vec![Type::Int],
                ret: Box::new(Type::Str),
            },
        ));
        s.add(Constraint::Call {
            result,
            callee: CalleeRef::Dynamic(callee_key),
            args: Vec::new(),
            kwargs: Vec::new(),
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Str);
    }

    #[test]
    fn run_call_dynamic_with_non_function_callee_yields_any() {
        let mut s = Solver::new();
        let callee_key = TypeKey::Var(VarId::new(0));
        let result = ek(0);
        s.add(Constraint::Concrete(callee_key, Type::Int)); // not callable
        s.add(Constraint::Call {
            result,
            callee: CalleeRef::Dynamic(callee_key),
            args: Vec::new(),
            kwargs: Vec::new(),
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Any);
    }

    #[test]
    fn run_call_builtin_delegates_to_ctx() {
        let mut s = Solver::new();
        let result = ek(0);
        let bid = BuiltinId(pyaot_hir::Builtin::Len);
        s.add(Constraint::Call {
            result,
            callee: CalleeRef::Builtin(bid),
            args: Vec::new(),
            kwargs: Vec::new(),
        });
        let mut ctx = TestCtx::default();
        ctx.add_builtin(bid, Type::Int);
        s.run(&ctx);
        assert_eq!(s.env().get(result), Type::Int);
    }

    #[test]
    fn run_call_class_ctor_delegates_to_ctx() {
        let mut s = Solver::new();
        let result = ek(0);
        let class_id = ClassId::new(99);
        let mut interner = pyaot_utils::StringInterner::new();
        let class_name = interner.intern("Foo");
        let class_ty = Type::Class {
            class_id,
            name: class_name,
        };
        s.add(Constraint::Call {
            result,
            callee: CalleeRef::ClassCtor(class_id),
            args: Vec::new(),
            kwargs: Vec::new(),
        });
        let mut ctx = TestCtx::default();
        ctx.add_class_ctor(class_id, class_ty.clone());
        s.run(&ctx);
        assert_eq!(s.env().get(result), class_ty);
    }

    #[test]
    fn run_method_call_delegates_to_ctx_with_recv_and_args() {
        let mut s = Solver::new();
        let recv = ek(0);
        let arg = ek(1);
        let result = ek(2);
        let mut interner = pyaot_utils::StringInterner::new();
        let method = interner.intern("upper");
        s.add(Constraint::Concrete(recv, Type::Str));
        s.add(Constraint::Concrete(arg, Type::Int));
        s.add(Constraint::MethodCall {
            result,
            recv,
            name: method,
            args: vec![arg],
        });
        let mut ctx = TestCtx::default();
        ctx.add_method(Type::Str, method, Type::Str);
        s.run(&ctx);
        assert_eq!(s.env().get(result), Type::Str);
        let calls = ctx.method_calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "ctx.method_return must be called exactly once"
        );
        assert_eq!(calls[0], (Type::Str, method, vec![Type::Int]));
    }

    #[test]
    fn run_method_call_falls_back_to_any_when_ctx_returns_none() {
        let mut s = Solver::new();
        let recv = ek(0);
        let result = ek(1);
        let mut interner = pyaot_utils::StringInterner::new();
        let method = interner.intern("unknown");
        s.add(Constraint::Concrete(recv, Type::Str));
        s.add(Constraint::MethodCall {
            result,
            recv,
            name: method,
            args: Vec::new(),
        });
        s.run(&PermissiveCtx); // PermissiveCtx::method_return → None
        assert_eq!(s.env().get(result), Type::Any);
    }

    #[test]
    fn run_attribute_delegates_to_ctx() {
        let mut s = Solver::new();
        let recv = ek(0);
        let result = ek(1);
        let mut interner = pyaot_utils::StringInterner::new();
        let attr = interner.intern("x");
        let class_id = ClassId::new(5);
        let class_name = interner.intern("Point");
        let class_ty = Type::Class {
            class_id,
            name: class_name,
        };
        s.add(Constraint::Concrete(recv, class_ty.clone()));
        s.add(Constraint::Attribute {
            result,
            recv,
            name: attr,
        });
        let mut ctx = TestCtx::default();
        ctx.add_attribute(class_ty, attr, Type::Float);
        s.run(&ctx);
        assert_eq!(s.env().get(result), Type::Float);
    }

    #[test]
    fn run_subscript_list_inline_returns_elem_type() {
        let mut s = Solver::new();
        let recv = ek(0);
        let index = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(recv, Type::list_of(Type::Int)));
        s.add(Constraint::Concrete(index, Type::Int));
        s.add(Constraint::Subscript {
            result,
            recv,
            index,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Int);
    }

    #[test]
    fn run_subscript_dict_inline_returns_value_type() {
        let mut s = Solver::new();
        let recv = ek(0);
        let index = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(
            recv,
            Type::dict_of(Type::Str, Type::Float),
        ));
        s.add(Constraint::Concrete(index, Type::Str));
        s.add(Constraint::Subscript {
            result,
            recv,
            index,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Float);
    }

    #[test]
    fn run_subscript_tuple_inline_homogenizes_to_join() {
        let mut s = Solver::new();
        let recv = ek(0);
        let index = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(
            recv,
            Type::tuple_of(vec![Type::Int, Type::Int, Type::Float]),
        ));
        s.add(Constraint::Concrete(index, Type::Int));
        s.add(Constraint::Subscript {
            result,
            recv,
            index,
        });
        s.run(&PermissiveCtx);
        // Int join Float via numeric tower = Float.
        assert_eq!(s.env().get(result), Type::Float);
    }

    #[test]
    fn run_subscript_str_inline_returns_str() {
        let mut s = Solver::new();
        let recv = ek(0);
        let index = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(recv, Type::Str));
        s.add(Constraint::Concrete(index, Type::Int));
        s.add(Constraint::Subscript {
            result,
            recv,
            index,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Str);
    }

    #[test]
    fn run_subscript_bytes_inline_returns_int() {
        let mut s = Solver::new();
        let recv = ek(0);
        let index = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(recv, Type::Bytes));
        s.add(Constraint::Concrete(index, Type::Int));
        s.add(Constraint::Subscript {
            result,
            recv,
            index,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Int);
    }

    #[test]
    fn run_subscript_user_class_delegates_to_ctx() {
        let mut s = Solver::new();
        let recv = ek(0);
        let index = ek(1);
        let result = ek(2);
        let mut interner = pyaot_utils::StringInterner::new();
        let class_id = ClassId::new(3);
        let class_name = interner.intern("Matrix");
        let class_ty = Type::Class {
            class_id,
            name: class_name,
        };
        s.add(Constraint::Concrete(recv, class_ty.clone()));
        s.add(Constraint::Concrete(index, Type::Int));
        s.add(Constraint::Subscript {
            result,
            recv,
            index,
        });
        let mut ctx = TestCtx::default();
        ctx.add_subscript(class_ty, Type::Int, Type::Float);
        s.run(&ctx);
        assert_eq!(s.env().get(result), Type::Float);
    }

    #[test]
    fn run_iter_elem_list_default_impl() {
        let mut s = Solver::new();
        let iter = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(iter, Type::list_of(Type::Int)));
        s.add(Constraint::IterElem { result, iter });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Int);
    }

    #[test]
    fn run_iter_elem_dict_default_impl_yields_keys() {
        let mut s = Solver::new();
        let iter = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(
            iter,
            Type::dict_of(Type::Str, Type::Int),
        ));
        s.add(Constraint::IterElem { result, iter });
        s.run(&PermissiveCtx);
        // Python: `for k in d` iterates over keys.
        assert_eq!(s.env().get(result), Type::Str);
    }

    #[test]
    fn run_iter_elem_set_default_impl() {
        let mut s = Solver::new();
        let iter = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(iter, Type::set_of(Type::Bool)));
        s.add(Constraint::IterElem { result, iter });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Bool);
    }

    #[test]
    fn run_iter_elem_tuple_default_impl_joins_elements() {
        let mut s = Solver::new();
        let iter = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(
            iter,
            Type::tuple_of(vec![Type::Int, Type::Float, Type::Bool]),
        ));
        s.add(Constraint::IterElem { result, iter });
        s.run(&PermissiveCtx);
        // Int join Float join Bool = Float (numeric tower).
        assert_eq!(s.env().get(result), Type::Float);
    }

    #[test]
    fn run_iter_elem_iterator_default_impl() {
        let mut s = Solver::new();
        let iter = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(
            iter,
            Type::Iterator(Box::new(Type::Str)),
        ));
        s.add(Constraint::IterElem { result, iter });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Str);
    }

    #[test]
    fn run_iter_elem_str_default_impl_yields_str() {
        let mut s = Solver::new();
        let iter = ek(0);
        let result = ek(1);
        s.add(Constraint::Concrete(iter, Type::Str));
        s.add(Constraint::IterElem { result, iter });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Str);
    }

    #[test]
    fn run_iter_elem_unknown_type_yields_any() {
        let mut s = Solver::new();
        let iter = ek(0);
        let result = ek(1);
        // Int is not iterable; default_iter_elem returns None.
        s.add(Constraint::Concrete(iter, Type::Int));
        s.add(Constraint::IterElem { result, iter });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Any);
    }

    #[test]
    fn default_iter_elem_returns_none_for_non_iterable() {
        assert!(default_iter_elem(&Type::Int).is_none());
        assert!(default_iter_elem(&Type::Float).is_none());
        assert!(default_iter_elem(&Type::Bool).is_none());
        assert!(default_iter_elem(&Type::None).is_none());
        assert!(default_iter_elem(&Type::Any).is_none());
    }

    #[test]
    fn run_subscript_reschedules_when_container_widens() {
        // recv starts as list[Int]; widen to list[Float] mid-solve.
        let mut s = Solver::new();
        let recv = ek(0);
        let index = ek(1);
        let result = ek(2);
        s.add(Constraint::Concrete(recv, Type::list_of(Type::Int)));
        s.add(Constraint::Concrete(index, Type::Int));
        s.add(Constraint::Subscript {
            result,
            recv,
            index,
        });
        s.run(&PermissiveCtx);
        assert_eq!(s.env().get(result), Type::Int);

        // Widen the container.
        s.add(Constraint::Concrete(recv, Type::list_of(Type::Float)));
        s.run(&PermissiveCtx);
        assert_eq!(
            s.env().get(result),
            Type::Float,
            "Subscript must reschedule on container widening"
        );
    }

    #[test]
    fn run_method_call_reschedules_when_recv_widens() {
        let mut s = Solver::new();
        let recv = ek(0);
        let result = ek(1);
        let mut interner = pyaot_utils::StringInterner::new();
        let method = interner.intern("m");
        s.add(Constraint::Concrete(recv, Type::Int));
        s.add(Constraint::MethodCall {
            result,
            recv,
            name: method,
            args: Vec::new(),
        });
        let mut ctx = TestCtx::default();
        ctx.add_method(Type::Int, method, Type::Int);
        ctx.add_method(Type::Float, method, Type::Float);
        s.run(&ctx);
        assert_eq!(s.env().get(result), Type::Int);

        // Widen recv: re-runs ctx with new recv type.
        s.add(Constraint::Concrete(recv, Type::Float));
        s.run(&ctx);
        // recv is now Float (after numeric-tower join); ctx returns Float.
        assert_eq!(s.env().get(result), Type::Float);
    }
}
