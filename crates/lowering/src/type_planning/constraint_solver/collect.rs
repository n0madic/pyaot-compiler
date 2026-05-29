//! HIR → constraint walker.
//!
//! Single pass over [`hir::Module`] emitting solver constraints. S2 scope
//! covers literals, variable read/write, `BinOp`, `UnOp`, container literals
//! (`List`/`Tuple`/`Dict`/`Set`), comparisons (always `Bool`), return
//! statements, and parameter type annotations.
//!
//! Unsupported `ExprKind` variants (calls, methods, attributes, subscripts,
//! comprehensions, lambdas, format specs, intrinsics, …) are **skipped** at
//! S2 — their `Expr(eid)` key stays at `Never` (bottom). Materialization
//! falls back to `expr.ty` for skipped keys, preserving any frontend-set
//! types. S3 adds `Call`/`MethodCall`/`Attribute`/`Subscript`/`IterElem`
//! constraints; S4 adds class fields, captures, dunders, generator yields.

use std::collections::HashMap;

use indexmap::IndexSet;
use pyaot_hir as hir;
use pyaot_hir::{BindingTarget, ExprId, ExprKind, HirTerminator, StmtKind};
use pyaot_types::Type;
use pyaot_utils::{ClassId, FuncId, InternedString, StringInterner, VarId};

use super::key::TypeKey;
use super::solve::Solver;
use super::vocab::{CalleeRef, Constraint, ContainerKind};

/// Discriminator for `propagate_mutator_elem_refinement`'s argument-to-
/// element-type conversion. `SingleElem` wraps the arg directly;
/// `IterableArg` extracts the arg's iterator element type via an
/// `IterElem` constraint.
enum RefineKind {
    SingleElem(ExprId),
    IterableArg(ExprId),
}

/// Single-pass HIR walker. Owns nothing except short-lived references to
/// the module and the solver; constraint emission is the only side effect.
pub struct Collector<'a> {
    solver: &'a mut Solver,
    module: &'a hir::Module,
    /// Read-only interner for `InternedString → &str` resolution. Needed
    /// for method-name dispatch (`append` / `add` / etc.) without baking
    /// the interner indices into the collector.
    interner: &'a StringInterner,
    /// Function currently being walked. Required by `Return` / `Yield`
    /// constraints which must reference an outer `FuncId`.
    current_func: Option<FuncId>,
    /// Expressions whose constraint has already been emitted. The HIR is a
    /// DAG (shared subexpressions are possible in principle), and emitting
    /// the same constraint twice would just be wasted work — not incorrect,
    /// since constraints are pure functions of their inputs.
    visited: IndexSet<ExprId>,
    /// Per-function static `VarId → ClassId` hints, populated from typed
    /// parameters and typed `Bind` statements. Powers the S4 class-field
    /// routing: when the collector sees `obj.x = …` or `obj.x` and `obj`
    /// is a `Var(v)` with a known class, it emits a `FieldWrite`
    /// constraint or a `FlowsInto` from `ClassField(class_id, x)`
    /// instead of relying on dynamic ctx dispatch.
    var_class_hints: HashMap<VarId, ClassId>,
    /// `FuncId → owning ClassId` for instance methods / `__init__` /
    /// property accessors. Lets `collect_function` class-hint a method's
    /// `self` parameter so `self.field = …` routes through a `FieldWrite`
    /// (the `obj_class_hint` lookup needs `self`'s class). Built once in
    /// [`Collector::collect_module`]. Mirrors the legacy `method_self_types`
    /// map in `closure_scan`.
    method_owner: HashMap<FuncId, ClassId>,
    /// `VarId → FuncId` for variables that hold a directly-assigned,
    /// **capture-free** lambda / function reference (`f = lambda x: …` or
    /// `f = some_func`). Lets the `Call` collector resolve an indirect
    /// call `f(args)` (`CalleeRef::Dynamic(Var f)`) to the concrete target
    /// so the call args hint the lambda's params and the result reads its
    /// `FuncReturn`. Without this the lambda's params stay `Any`/Tagged
    /// while a direct `CallDirect` passes them as primitives → the result
    /// is a tagged `Value` the `int`-typed dest never unboxes.
    ///
    /// Restricted to capture-free targets — a `Var` holding a *capturing*
    /// closure value isn't reliably lowered to a `CallDirect`, so resolving
    /// it here could mis-type the call. (The lifted-closure capture offset
    /// itself is now handled uniformly by the `CalleeRef::Func` hint loop via
    /// `callee_cap_count`.) Also restricted to variables assigned exactly
    /// once (a `VarId` re-bound to a different target is removed —
    /// `Some`→absent — so an ambiguous holder never mis-hints).
    var_to_lambda: HashMap<VarId, FuncId>,
    /// `FuncId → set of param indices` that are mutated in-place via a
    /// container mutator (`append`/`add`/`insert`/`extend`/`update`),
    /// possibly through `Index` chains (`keys[i].append(x)`). Python passes
    /// lists/dicts/sets by reference, so such a mutation refines the
    /// CALLER's argument object too. The `Call` collector flows the
    /// (mutation-refined) param `Var` back into a `Var` argument for these
    /// indices — inter-procedural mutation flow-back. Gated on "param is
    /// mutated" so a merely-reassigned param never widens the caller's var.
    mutated_param_indices: HashMap<FuncId, std::collections::HashSet<usize>>,
    /// Functions whose body directly `return NotImplemented`. A
    /// `self.method()` call to such a method is routed through the method's
    /// `FuncReturn` (which carries `NotImplementedType`) rather than the
    /// `MethodCall` ctx path (which reads the stale Lowering return and
    /// reports `None`). Narrowly scoped to NI-returning methods: routing
    /// *every* self-method call through `FuncReturn` regressed the verifier
    /// (precise `Raw(F64)`/`Raw(I64)` returns clashed with `Tagged` call
    /// dests), whereas NI-returners are union/tagged-typed and safe.
    ni_methods: std::collections::HashSet<FuncId>,
    /// `field name → classes defining it` (from `ClassDef.fields`). Lets the
    /// `Attribute` collector speculatively edge `ClassField(class, name)` for
    /// every candidate class, so an `obj.name` read on a receiver whose class
    /// is resolved DURING solving (e.g. `keys[0][0].data`) re-evaluates when
    /// that field's type is refined late. Without these edges
    /// `inputs_of(Attribute)` lists only `[recv]`, and the reducer's
    /// `ClassField` read is a one-shot snapshot that misses later updates.
    field_to_classes: HashMap<InternedString, Vec<ClassId>>,
    /// Active `else`-branch narrowing context `(v, T)` while collecting the
    /// else arm of `… if isinstance(v, T) else …`. In that arm `v` is NOT a
    /// `T`, so the coercion ctor `T(v)` (the idiom
    /// `v = v if isinstance(v, T) else T(v)`) must NOT flow `v`'s
    /// (T-inclusive) type into `T`'s `__init__` param — doing so pollutes
    /// `T`'s own field with `T` (the path-insensitive solver can't compute
    /// `v minus T` monotonically). Suppressing that one hint lets the field
    /// keep the type from its non-coercion writes (e.g. `Float`).
    else_isinstance: Option<(VarId, ClassId)>,
}

impl<'a> Collector<'a> {
    pub fn new(
        solver: &'a mut Solver,
        module: &'a hir::Module,
        interner: &'a StringInterner,
    ) -> Self {
        Self {
            solver,
            module,
            interner,
            current_func: None,
            visited: IndexSet::new(),
            var_class_hints: HashMap::new(),
            method_owner: HashMap::new(),
            var_to_lambda: HashMap::new(),
            mutated_param_indices: HashMap::new(),
            ni_methods: std::collections::HashSet::new(),
            field_to_classes: HashMap::new(),
            else_isinstance: None,
        }
    }

    /// If `cond` is `isinstance(Var(v), ClassRef(T))`, return `(v, T)` — the
    /// narrowing applied to the THEN branch (`v : T`) and, negated, to the
    /// ELSE branch (`v : not T`). Used to suppress the coercion-ctor hint
    /// in the else arm (see [`Self::else_isinstance`]).
    fn isinstance_cond(&self, cond: ExprId) -> Option<(VarId, ClassId)> {
        let ExprKind::BuiltinCall { builtin, args, .. } = &self.module.exprs[cond].kind else {
            return None;
        };
        if !matches!(builtin, hir::Builtin::Isinstance) || args.len() < 2 {
            return None;
        }
        let ExprKind::Var(v) = &self.module.exprs[args[0]].kind else {
            return None;
        };
        let ExprKind::ClassRef(cid) = &self.module.exprs[args[1]].kind else {
            return None;
        };
        Some((*v, *cid))
    }

    /// Whether body-derived `Return` constraints should be SKIPPED for
    /// `fid`. True for generators (`return v` raises `StopIteration(v)`,
    /// handled via `FuncYield`) and for functions with a declared return
    /// annotation. For annotated functions the annotation is
    /// authoritative — this both matches the legacy planner and prevents
    /// an abstract method's implicit `return None` (a `...`/`pass` body)
    /// from widening a declared `-> str` to `str | None`.
    fn skip_body_return(&self, fid: FuncId) -> bool {
        self.module
            .func_defs
            .get(&fid)
            .map(|f| f.is_generator || f.return_type.is_some())
            .unwrap_or(false)
    }

    /// Record a static `Var → Class` hint. Idempotent; later writes with
    /// a different class id are ignored (the first observed binding
    /// wins).
    fn note_var_class(&mut self, var: VarId, ty: &Type) {
        if let Type::Class { class_id, .. } = ty {
            self.var_class_hints.entry(var).or_insert(*class_id);
        }
    }

    /// If `eid` is a constructor call `ClassName(...)`, return the
    /// constructed `ClassId`. Lets the `Bind` collector class-hint a local
    /// (`n = V(...)`) so a later `n.field = …` attr-store routes through a
    /// `FieldWrite` — without it, cross-instance field writes via non-`self`
    /// locals are dropped and the field type loses that branch.
    fn ctor_call_class(&self, eid: ExprId) -> Option<ClassId> {
        if let ExprKind::Call { func, .. } = &self.module.exprs[eid].kind {
            if let ExprKind::ClassRef(cid) = &self.module.exprs[*func].kind {
                return Some(*cid);
            }
        }
        None
    }

    /// Returns the static class hint for `obj` if `obj` is a `Var(v)`
    /// with a known class — used to route field reads/writes to the
    /// solver's ClassField key.
    fn obj_class_hint(&self, obj: ExprId) -> Option<ClassId> {
        match &self.module.exprs[obj].kind {
            ExprKind::Var(v) => self.var_class_hints.get(v).copied(),
            _ => None,
        }
    }

    /// Solver-native `refined_container_types`: back-propagate the
    /// element type of a mutator call into the receiver Var.
    ///
    /// Patterns handled:
    /// - `var.append(x)` / `var.add(x)`: emit single-element
    ///   `ContainerLiteral(List/Set, [x])` and flow into `Var(v)`. Once
    ///   `x` resolves to `T`, the JOIN refines `Var(v)` from
    ///   `list[Never]` (the empty-literal seed) to `list[T]`.
    /// - `var.insert(_, x)`: same as `append` — second arg is the element.
    /// - `var.extend(iter)` / `var.update(iter)`: take iter's element
    ///   type via an `IterElem` constraint, then wrap as list/set, then
    ///   flow into `Var(v)`.
    ///
    /// Method name is resolved via the interner (string compare); only
    /// `Var(v)` receivers participate (legacy also requires a static Var
    /// for the empty-container refinement).
    fn propagate_mutator_elem_refinement(
        &mut self,
        obj: ExprId,
        method: pyaot_utils::InternedString,
        args: &[ExprId],
    ) {
        // Receiver shapes that participate in empty-container refinement.
        // Peel any number of `Index` levels off the receiver to reach the
        // base `Var`, counting the depth:
        // - `var.append(x)`          → `var : list[x]`             (depth 0)
        // - `var[i].append(x)`       → `var : list[list[x]]`       (depth 1)
        // - `var[i][j].append(x)`    → `var : list[list[list[x]]]` (depth 2)
        // Container-of-container refinement: each indexed level is a
        // list-of-lists, the innermost holds the appended element. Drives
        // `_coc_gpt` (`keys[li].append(v)`) and `_coc_deep_fill`
        // (`grid[i][j].append(v)`) so chained `grid[0][0][0].data` reads
        // type-check.
        let (var, outer_wrap_levels) = {
            let mut cur = obj;
            let mut depth = 0usize;
            loop {
                match &self.module.exprs[cur].kind {
                    ExprKind::Var(v) => break (*v, depth),
                    ExprKind::Index { obj: inner, .. } => {
                        cur = *inner;
                        depth += 1;
                    }
                    _ => return,
                }
            }
        };
        // Own the resolved method name so the immutable borrow on
        // `self.interner` is released before we mutate `self.solver`.
        let method_name = self.interned_method_name(method).to_owned();
        let kind = match method_name.as_str() {
            "append" | "add" if args.len() == 1 => RefineKind::SingleElem(args[0]),
            "insert" if args.len() == 2 => RefineKind::SingleElem(args[1]),
            "extend" | "update" if args.len() == 1 => RefineKind::IterableArg(args[0]),
            _ => return,
        };
        let container_kind = match method_name.as_str() {
            "add" | "update" => ContainerKind::Set,
            _ => ContainerKind::List,
        };
        let elem_key = match kind {
            RefineKind::SingleElem(eid) => TypeKey::Expr(eid),
            RefineKind::IterableArg(eid) => {
                let elem_meta = self.solver.fresh_meta();
                self.solver.add(Constraint::IterElem {
                    result: elem_meta,
                    iter: TypeKey::Expr(eid),
                });
                elem_meta
            }
        };
        // Inner container holding the appended element: `list[elem]` /
        // `set[elem]` per the method.
        let mut wrap_meta = self.solver.fresh_meta();
        self.solver.add(Constraint::ContainerLiteral {
            result: wrap_meta,
            kind: container_kind,
            elems: vec![elem_key],
            kv: Vec::new(),
        });
        // For an indexed receiver (`var[idx].append`), wrap one more level:
        // the OUTER container is always list-shaped (it was indexed by an
        // integer position), holding the inner containers.
        for _ in 0..outer_wrap_levels {
            let outer_meta = self.solver.fresh_meta();
            self.solver.add(Constraint::ContainerLiteral {
                result: outer_meta,
                kind: ContainerKind::List,
                elems: vec![wrap_meta],
                kv: Vec::new(),
            });
            wrap_meta = outer_meta;
        }
        self.solver.add(Constraint::FlowsInto {
            src: wrap_meta,
            dst: TypeKey::Var(var),
        });
    }

    /// Resolve a method call `obj.method(…)` to a concrete user-defined
    /// `FuncId` when `obj`'s class is statically known: a `Var` with a
    /// `var_class_hints` entry (notably `self`) **or** a direct constructor
    /// call `V(...).method(…)`. Walks the single-inheritance chain and matches
    /// by the method's source name (the suffix after the `ClassName$`
    /// mangling). Returns `None` for unknown receivers or built-in /
    /// cross-module methods, which then fall back to the `MethodCall` ctx path.
    ///
    /// Routing the result through the resolved method's `FuncReturn` lets a
    /// `self.method()` call self-bootstrap through the solver env — exactly
    /// like a direct `Call(Func)` — instead of reading the stale Lowering
    /// return type via `ctx.method_return` (which misses solver-inferred
    /// returns such as `NotImplemented`). It also drives the arg→param hint
    /// flow (collect.rs `MethodCall` site): without resolving the receiver
    /// class, an unannotated method param stays `Any` and lowering picks the
    /// wrong runtime (`rt_deque_append` vs `rt_list_append`) → SIGSEGV.
    fn resolve_local_method(
        &self,
        obj: ExprId,
        method: pyaot_utils::InternedString,
    ) -> Option<FuncId> {
        let mut class_id = match &self.module.exprs[obj].kind {
            ExprKind::Var(v) => self.var_class_hints.get(v).copied()?,
            // `V(...).method(…)` — method called on a fresh instance. The
            // receiver is a Call expr, never a Var, so it never landed in
            // var_class_hints even though its class is statically obvious.
            ExprKind::Call { .. } => self.ctor_call_class(obj)?,
            _ => return None,
        };
        let want = self.interner.get(method)?;
        loop {
            let cdef = self.module.class_defs.get(&class_id)?;
            for &mfid in &cdef.methods {
                if let Some(f) = self.module.func_defs.get(&mfid) {
                    let fname = self.interner.get(f.name).unwrap_or("");
                    let suffix = fname.rsplit('$').next().unwrap_or(fname);
                    if suffix == want {
                        return Some(mfid);
                    }
                }
            }
            class_id = cdef.base_class?;
        }
    }

    /// Resolve an `InternedString` to a `&str` for method-name matching.
    /// Returns `""` if the index isn't bound in the local interner —
    /// happens in tests that pass an empty interner alongside an
    /// HIR module containing already-interned strings (the test's
    /// own interner is different from the one passed to `collect`).
    fn interned_method_name(&self, method: pyaot_utils::InternedString) -> &str {
        self.interner.get(method).unwrap_or("")
    }

    /// Emit `LambdaParamHint` constraints that route a higher-order
    /// builtin's iterable-element type to its callback's first param.
    ///
    /// Pattern table:
    /// - `map(f, xs)`, `filter(f, xs)`: callback is `args[0]`, iterable
    ///   is `args[1]`. Hint the callback's param 0 with `IterElem(xs)`.
    /// - `sorted(xs, key=f)` is keyword-driven; the keyword path is
    ///   handled in the regular `Call` collector (kwargs carry the
    ///   callback). This method covers positional HOFs only.
    ///
    /// Skips silently when the callback isn't an inline `Closure` —
    /// `map(named_func, xs)` would route through `CalleeRef::Func` at
    /// the regular Call collector once we lift the closure-as-value
    /// constraint to recover the func id.
    fn propagate_hof_iterable_hint(&mut self, builtin: hir::Builtin, args: &[hir::ExprId]) {
        // Positional-callback HOFs: `map`/`filter` deliver ONE element
        // param; `reduce` delivers TWO (accumulator + element), both of
        // the element type. All take the callback first, iterable second.
        let elem_param_count = match builtin {
            hir::Builtin::Map | hir::Builtin::Filter => 1,
            hir::Builtin::Reduce => 2,
            _ => return,
        };
        if args.len() < 2 {
            return;
        }
        let (callback_fid, cap_count) = match &self.module.exprs[args[0]].kind {
            ExprKind::Closure { func, captures } => (*func, captures.len()),
            ExprKind::FuncRef(fid) => (*fid, 0),
            _ => return,
        };
        let iterable_eid = args[1];
        let elem_meta = self.solver.fresh_meta();
        self.solver.add(Constraint::IterElem {
            result: elem_meta,
            iter: TypeKey::Expr(iterable_eid),
        });
        // The callback's element parameter(s) follow its capture params:
        // a lifted closure delivers captures as its LEADING positional
        // params, so `map(lambda x: x + off, xs)` (capturing `off`) has
        // params `[off, x]` and the element type must hint `x` at index
        // `cap_count`, NOT 0. Hinting index 0 would type the `off` capture
        // with the iterable element — the mis-slotting that made the first
        // attempt at this propagation regress and led to it being disabled.
        // `reduce(lambda acc, x: …, xs)` hints both `acc` and `x` with the
        // element type (the accumulator seeds from the first element).
        for i in 0..elem_param_count {
            self.solver.add(Constraint::LambdaParamHint {
                func: callback_fid,
                param_ix: cap_count + i,
                hint: elem_meta,
            });
        }
    }

    /// Emit a `LambdaParamHint` routing a `sorted`/`min`/`max` call's
    /// iterable-element type to its `key=` callback's user parameter.
    ///
    /// `sorted(xs, key=f)` / `min(xs, key=f)` / `max(xs, key=f)` apply `f`
    /// to each element of `xs` (the first positional arg). The key callback
    /// has exactly one user parameter; in the lifted-closure ABI it follows
    /// the callback's leading capture params, so the hint lands at index
    /// `cap_count`. Restricted to the single-iterable form (`args.len() == 1`)
    /// — the variadic `min(a, b, c, key=f)` form passes elements directly as
    /// positional args, where `args[0]` is an element rather than an iterable.
    fn propagate_hof_key_hint(
        &mut self,
        builtin: hir::Builtin,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
    ) {
        if !matches!(
            builtin,
            hir::Builtin::Sorted | hir::Builtin::Min | hir::Builtin::Max
        ) {
            return;
        }
        if args.len() != 1 {
            return;
        }
        let Some(key_kw) = kwargs
            .iter()
            .find(|kw| self.interner.get(kw.name) == Some("key"))
        else {
            return;
        };
        let (callback_fid, cap_count) = match &self.module.exprs[key_kw.value].kind {
            ExprKind::Closure { func, captures } => (*func, captures.len()),
            ExprKind::FuncRef(fid) => (*fid, 0),
            _ => return,
        };
        let elem_meta = self.solver.fresh_meta();
        self.solver.add(Constraint::IterElem {
            result: elem_meta,
            iter: TypeKey::Expr(args[0]),
        });
        self.solver.add(Constraint::LambdaParamHint {
            func: callback_fid,
            param_ix: cap_count,
            hint: elem_meta,
        });
    }

    /// Emit a `LambdaParamHint` routing a `list.sort(key=f)` receiver's
    /// element type to the key callback's user parameter — the in-place
    /// `.sort()` analogue of [`Self::propagate_hof_key_hint`]. The element
    /// type comes from the receiver list (`obj`), not a positional arg.
    fn propagate_method_key_hint(
        &mut self,
        obj: hir::ExprId,
        method: InternedString,
        kwargs: &[hir::KeywordArg],
    ) {
        if self.interner.get(method) != Some("sort") {
            return;
        }
        let Some(key_kw) = kwargs
            .iter()
            .find(|kw| self.interner.get(kw.name) == Some("key"))
        else {
            return;
        };
        let (callback_fid, cap_count) = match &self.module.exprs[key_kw.value].kind {
            ExprKind::Closure { func, captures } => (*func, captures.len()),
            ExprKind::FuncRef(fid) => (*fid, 0),
            _ => return,
        };
        let elem_meta = self.solver.fresh_meta();
        self.solver.add(Constraint::IterElem {
            result: elem_meta,
            iter: TypeKey::Expr(obj),
        });
        self.solver.add(Constraint::LambdaParamHint {
            func: callback_fid,
            param_ix: cap_count,
            hint: elem_meta,
        });
    }

    /// Walk every function in the module and emit constraints. Top-level
    /// entry point.
    pub fn collect_module(&mut self) {
        // Pre-pass: map each method / `__init__` / property accessor to its
        // owning class, so `collect_function` can class-hint `self`.
        for (cid, cdef) in self.module.class_defs.iter() {
            for &m in &cdef.methods {
                self.method_owner.insert(m, *cid);
            }
            if let Some(init) = cdef.init_method {
                self.method_owner.insert(init, *cid);
            }
            for prop in &cdef.properties {
                self.method_owner.insert(prop.getter, *cid);
                if let Some(setter) = prop.setter {
                    self.method_owner.insert(setter, *cid);
                }
            }
            for field in &cdef.fields {
                self.field_to_classes
                    .entry(field.name)
                    .or_default()
                    .push(*cid);
            }
        }

        // Pre-pass: map variables to a directly-assigned, capture-free
        // lambda / function reference so an indirect call `f(args)`
        // resolves to the concrete target (see `var_to_lambda` docs).
        // Walk every function's `Bind` statements; a `VarId` bound to more
        // than one distinct target is removed (ambiguous holders must not
        // mis-hint).
        let mut ambiguous: std::collections::HashSet<VarId> = std::collections::HashSet::new();
        for func in self.module.func_defs.values() {
            for block in func.blocks.values() {
                for &stmt_id in &block.stmts {
                    let hir::StmtKind::Bind { target, value, .. } =
                        &self.module.stmts[stmt_id].kind
                    else {
                        continue;
                    };
                    let hir::BindingTarget::Var(v) = target else {
                        continue;
                    };
                    let target_fid = match &self.module.exprs[*value].kind {
                        ExprKind::Closure { func, captures } if captures.is_empty() => Some(*func),
                        ExprKind::FuncRef(fid) => Some(*fid),
                        _ => None,
                    };
                    match target_fid {
                        Some(fid) => {
                            if ambiguous.contains(v) {
                                continue;
                            }
                            match self.var_to_lambda.get(v) {
                                Some(prev) if *prev != fid => {
                                    self.var_to_lambda.remove(v);
                                    ambiguous.insert(*v);
                                }
                                Some(_) => {}
                                None => {
                                    self.var_to_lambda.insert(*v, fid);
                                }
                            }
                        }
                        None => {
                            // Var rebound to a non-lambda value — its
                            // holder identity is no longer a single lambda.
                            if self.var_to_lambda.remove(v).is_some() {
                                ambiguous.insert(*v);
                            }
                        }
                    }
                }
            }
        }

        // Pre-pass: identify container-mutated params per function (for
        // inter-procedural mutation flow-back — see `mutated_param_indices`).
        for (fid, func) in self.module.func_defs.iter() {
            // Map each param's VarId to its positional index.
            let mut param_index: HashMap<VarId, usize> = HashMap::new();
            for (ix, p) in func.params.iter().enumerate() {
                param_index.insert(p.var, ix);
            }
            if param_index.is_empty() {
                continue;
            }
            let mut mutated: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for block in func.blocks.values() {
                for &stmt_id in &block.stmts {
                    // A mutator surfaces as `Expr(MethodCall)` (the call's
                    // value is discarded) or nested inside another stmt;
                    // the common in-place form is a bare expression stmt.
                    let expr_id = match &self.module.stmts[stmt_id].kind {
                        hir::StmtKind::Expr(e) => *e,
                        _ => continue,
                    };
                    let ExprKind::MethodCall {
                        obj, method, args, ..
                    } = &self.module.exprs[expr_id].kind
                    else {
                        continue;
                    };
                    let is_mutator = matches!(
                        self.interner.get(*method),
                        Some("append" | "add" | "insert" | "extend" | "update")
                    ) && !args.is_empty();
                    if !is_mutator {
                        continue;
                    }
                    // Peel `Index` levels off the receiver to reach the base
                    // Var; if it's a param, that param is mutated in place.
                    let mut cur = *obj;
                    let base_var = loop {
                        match &self.module.exprs[cur].kind {
                            ExprKind::Var(v) => break Some(*v),
                            ExprKind::Index { obj: inner, .. } => cur = *inner,
                            _ => break None,
                        }
                    };
                    if let Some(v) = base_var {
                        if let Some(&ix) = param_index.get(&v) {
                            mutated.insert(ix);
                        }
                    }
                }
            }
            if !mutated.is_empty() {
                self.mutated_param_indices.insert(*fid, mutated);
            }
        }

        // Pre-pass: functions that directly `return NotImplemented`
        // (terminator or statement). Used to narrowly route NI-returning
        // self-method calls through `FuncReturn` (see `ni_methods`).
        for (fid, func) in self.module.func_defs.iter() {
            let mut is_ni = false;
            'blocks: for block in func.blocks.values() {
                if let hir::HirTerminator::Return(Some(eid)) = &block.terminator {
                    if matches!(self.module.exprs[*eid].kind, ExprKind::NotImplemented) {
                        is_ni = true;
                        break 'blocks;
                    }
                }
                for &stmt_id in &block.stmts {
                    if let hir::StmtKind::Return(Some(eid)) = &self.module.stmts[stmt_id].kind {
                        if matches!(self.module.exprs[*eid].kind, ExprKind::NotImplemented) {
                            is_ni = true;
                            break 'blocks;
                        }
                    }
                }
            }
            if is_ni {
                self.ni_methods.insert(*fid);
            }
        }

        // IndexMap iteration is deterministic — important for
        // reproducible constraint ordering.
        let func_ids: Vec<FuncId> = self.module.func_defs.keys().copied().collect();
        for fid in func_ids {
            self.collect_function(fid);
        }
    }

    fn collect_function(&mut self, fid: FuncId) {
        let func = &self.module.func_defs[&fid];
        let prev_func = self.current_func.replace(fid);
        // Per-function var_class_hints scope. Stash the outer scope's
        // hints and start fresh — Python's lexical scoping means a
        // VarId-to-class binding in one function doesn't carry into
        // sibling functions. Nested functions inherit through the same
        // shared HashMap, which is acceptable because the same VarId
        // means the same variable in closure-captured cells.
        let saved_hints = std::mem::take(&mut self.var_class_hints);

        // Class-hint `self` (the receiver of an instance method / `__init__`
        // / property accessor) so `self.field = …` and `self.field` route
        // through `ClassField` keys via `obj_class_hint`. `self` is almost
        // always unannotated, so without this the FieldWrite for
        // `self._children = children` is never emitted and the field type
        // stays at its constructor-default seed. Mirrors the legacy
        // `method_self_types` seeding. (Static methods have no `self`;
        // classmethods take `cls`, which is not an instance — both excluded
        // by the `Instance` kind check.)
        if matches!(func.method_kind, hir::MethodKind::Instance) {
            if let (Some(&cid), Some(first)) = (self.method_owner.get(&fid), func.params.first()) {
                let self_var = first.var;
                self.var_class_hints.entry(self_var).or_insert(cid);
            }
        }

        // Seed annotated parameter types as Concrete constraints. Also
        // record any `Class`-typed param into var_class_hints so
        // `self.x = …` and `self.x` lookups inside the body route
        // through ClassField keys.
        for (ix, param) in func.params.iter().enumerate() {
            if let Some(ty) = &param.ty {
                self.solver
                    .add(Constraint::Concrete(TypeKey::Var(param.var), ty.clone()));
                self.note_var_class(param.var, ty);
            } else {
                // Unannotated param: connect it to LambdaParam(fid, ix)
                // so call-site hints flow into the local Var. Annotated
                // params skip this (their Concrete annotation already
                // pins the type — adding the flow would widen the
                // annotated type by every observed call-site argument).
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::LambdaParam(fid, ix),
                    dst: TypeKey::Var(param.var),
                });
            }
        }

        // Seed the function's `FuncReturn`:
        // - A declared return annotation is authoritative for BOTH regular
        //   functions and generators. The legacy planner used it verbatim —
        //   most importantly for abstract methods (`def speak(self) -> str:
        //   ...`) whose `...`/`pass` body has no explicit `return`, so body
        //   inference alone yields `None`.
        // - An UNANNOTATED generator derives `FuncReturn = Iterator(FuncYield)`
        //   during solving via `GeneratorReturn`, so a caller iterating the
        //   generator (e.g. a genexp over `range_gen(1, 5)`) sees the real
        //   element type instead of `Never`.
        if let Some(ret) = &func.return_type {
            self.solver
                .add(Constraint::Concrete(TypeKey::FuncReturn(fid), ret.clone()));
        } else if func.is_generator {
            self.solver.add(Constraint::GeneratorReturn { func: fid });
        }

        // Walk every block — IndexMap iteration is insertion-ordered.
        let block_ids: Vec<_> = func.blocks.keys().copied().collect();
        for bid in block_ids {
            let block = &self.module.func_defs[&fid].blocks[&bid];
            for &stmt_id in &block.stmts {
                self.collect_stmt(stmt_id);
            }
            // Each terminator may reference an ExprId we need to walk +
            // emit Return / Yield constraints from.
            self.collect_terminator(&block.terminator, fid);
        }

        self.var_class_hints = saved_hints;
        self.current_func = prev_func;
    }

    fn collect_terminator(&mut self, term: &HirTerminator, fid: FuncId) {
        // In Python, `return [value]` inside a generator raises
        // `StopIteration([value])` — it does NOT contribute to the
        // iterator element type. The generator's effective return
        // type is `Iterator(yield_element_type)`, established by the
        // apply step from `FuncYield(fid)`. Annotated functions take
        // their return from the declared annotation (seeded in
        // `collect_function`). In both cases skip body-derived Return
        // constraints — see `skip_body_return`.
        let skip_return = self.skip_body_return(fid);
        match term {
            HirTerminator::Return(Some(eid)) => {
                self.collect_expr(*eid);
                if !skip_return {
                    self.solver.add(Constraint::Return {
                        func: fid,
                        value: TypeKey::Expr(*eid),
                    });
                }
            }
            HirTerminator::Return(None) => {
                // `return` with no value → returns None. Skipped for
                // generators / annotated functions (see skip_body_return).
                if !skip_return {
                    self.solver
                        .add(Constraint::Concrete(TypeKey::FuncReturn(fid), Type::None));
                }
            }
            HirTerminator::Yield { value, .. } => {
                self.collect_expr(*value);
                self.solver.add(Constraint::Yield {
                    func: fid,
                    value: TypeKey::Expr(*value),
                });
            }
            HirTerminator::Branch { cond, .. } => {
                // Branch condition isn't itself a Return/Yield, but it IS
                // an expression we need to walk so its type gets bound.
                self.collect_expr(*cond);
            }
            HirTerminator::Raise { exc, cause } => {
                self.collect_expr(*exc);
                if let Some(c) = cause {
                    self.collect_expr(*c);
                }
            }
            HirTerminator::Jump(_) | HirTerminator::Reraise | HirTerminator::Unreachable => {}
        }
    }

    fn collect_stmt(&mut self, stmt_id: hir::StmtId) {
        let stmt = &self.module.stmts[stmt_id];
        match &stmt.kind {
            StmtKind::Bind {
                target,
                value,
                type_hint,
            } => {
                self.collect_expr(*value);
                // Class-hint a local bound to a constructor result so a
                // later `n.field = …` attr-store emits a `FieldWrite`
                // (capturing cross-instance field writes through non-`self`
                // locals). Statements are walked in order, so the hint is in
                // scope for subsequent stores to `n`.
                if let hir::BindingTarget::Var(n) = target {
                    if let Some(cid) = self.ctor_call_class(*value) {
                        self.var_class_hints.entry(*n).or_insert(cid);
                    }
                }
                self.collect_binding_target(target, *value, type_hint.as_ref());
            }
            // Legacy Return path — most returns now live in HirTerminator,
            // but a Stmt-form Return may still appear before CFG canonicalization.
            // Skipped inside generators (see HirTerminator::Return note).
            StmtKind::Return(Some(eid)) => {
                self.collect_expr(*eid);
                if let Some(fid) = self.current_func {
                    if !self.skip_body_return(fid) {
                        self.solver.add(Constraint::Return {
                            func: fid,
                            value: TypeKey::Expr(*eid),
                        });
                    }
                }
            }
            StmtKind::Return(None) => {
                if let Some(fid) = self.current_func {
                    if !self.skip_body_return(fid) {
                        self.solver
                            .add(Constraint::Concrete(TypeKey::FuncReturn(fid), Type::None));
                    }
                }
            }
            StmtKind::Expr(eid) => self.collect_expr(*eid),
            StmtKind::Assert { cond, msg } => {
                self.collect_expr(*cond);
                if let Some(m) = msg {
                    self.collect_expr(*m);
                }
            }
            StmtKind::Raise { exc, cause } => {
                if let Some(e) = exc {
                    self.collect_expr(*e);
                }
                if let Some(c) = cause {
                    self.collect_expr(*c);
                }
            }
            StmtKind::IndexDelete { obj, index } => {
                self.collect_expr(*obj);
                self.collect_expr(*index);
            }
            StmtKind::IterAdvance { iter, target } => {
                self.collect_expr(*iter);
                self.bind_iter_target(target, *iter);
            }
            StmtKind::IterSetup { iter } => self.collect_expr(*iter),
            StmtKind::Break | StmtKind::Continue | StmtKind::Pass => {}
        }
    }

    /// Bind the loop variable(s) of `for <target> in <iter>` to the
    /// iterable's element type. The `Var(v)` case routes through an
    /// `IterElem` reducer over the iterable expression: once `iter`
    /// resolves to a concrete container, `IterElem` yields its element
    /// type and flows it into the loop var. This is essential — without
    /// it every loop variable stays `Never`, so `for x in xs:
    /// out.append(f(x))` freezes `out` at `list[Never]`.
    ///
    /// `Var(v)` flows the iterable's element type straight in. Tuple
    /// targets (`for a, b in pairs`) destructure per-position via
    /// `TupleProject`: each leaf takes its slot of the element tuple, so
    /// `zip(...)` loops type `a`/`b` to the correct component instead of
    /// the whole element (the mis-typing that earlier whole-element
    /// approaches produced). `Starred` captures the remaining elements as
    /// a list of the element type.
    fn bind_iter_target(&mut self, target: &BindingTarget, iter: ExprId) {
        let elem = self.solver.fresh_meta();
        self.solver.add(Constraint::IterElem {
            result: elem,
            iter: TypeKey::Expr(iter),
        });
        self.flow_elem_into_target(target, elem);
    }

    /// Flow an already-computed element-type key `elem` into a binding
    /// target, destructuring tuples per-position.
    fn flow_elem_into_target(&mut self, target: &BindingTarget, elem: TypeKey) {
        match target {
            BindingTarget::Var(v) => {
                self.solver.add(Constraint::FlowsInto {
                    src: elem,
                    dst: TypeKey::Var(*v),
                });
            }
            BindingTarget::Tuple { elts, .. } => {
                for (i, elt) in elts.iter().enumerate() {
                    let proj = self.solver.fresh_meta();
                    self.solver.add(Constraint::TupleProject {
                        result: proj,
                        tuple: elem,
                        index: i,
                    });
                    self.flow_elem_into_target(elt, proj);
                }
            }
            BindingTarget::Starred { inner, .. } => {
                // `*rest` binds a list of the element type. Wrap `elem`
                // in a single-element list-literal meta, then recurse.
                let wrap = self.solver.fresh_meta();
                self.solver.add(Constraint::ContainerLiteral {
                    result: wrap,
                    kind: super::vocab::ContainerKind::List,
                    elems: vec![elem],
                    kv: Vec::new(),
                });
                self.flow_elem_into_target(inner, wrap);
            }
            BindingTarget::Attr { obj, field, .. } => {
                // Destructured store into `obj.field` — route through a
                // `FieldWrite` when `obj`'s class is known (e.g.
                // `self.a, self.b = pair`).
                self.collect_expr(*obj);
                if let Some(class_id) = self.obj_class_hint(*obj) {
                    self.solver.add(Constraint::FieldWrite {
                        class: class_id,
                        name: *field,
                        value: elem,
                    });
                }
            }
            BindingTarget::ClassAttr { class_id, attr, .. } => {
                self.solver.add(Constraint::FieldWrite {
                    class: *class_id,
                    name: *attr,
                    value: elem,
                });
            }
            BindingTarget::Index { obj, .. } => {
                // `obj[i], … = …` — visit the receiver; element-precise
                // index-store typing is out of scope for destructuring.
                self.collect_expr(*obj);
            }
        }
    }

    /// Emit `FlowsInto(Expr(value) → Var(v))` for `Var` leaves of the
    /// binding target. `Attr`/`Index`/`ClassAttr` targets are S3+ scope
    /// (need reducer support for field writes and indexed stores). Tuple
    /// patterns recurse but don't yet emit per-element constraints —
    /// that needs an `IterElem` or destructuring reducer, both S3+.
    fn collect_binding_target(
        &mut self,
        target: &BindingTarget,
        value: ExprId,
        type_hint: Option<&Type>,
    ) {
        match target {
            BindingTarget::Var(v) => {
                let var_key = TypeKey::Var(*v);
                // Annotation pins the variable's type — emitted as Concrete
                // so it's a permanent lower bound (the lattice JOIN will
                // widen it if heterogeneous assigns happen).
                if let Some(ty) = type_hint {
                    self.solver.add(Constraint::Concrete(var_key, ty.clone()));
                    // Capture Class hints so subsequent `var.x` reads/writes
                    // can route through ClassField keys.
                    self.note_var_class(*v, ty);
                }
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::Expr(value),
                    dst: var_key,
                });
            }
            BindingTarget::Tuple { .. } | BindingTarget::Starred { .. } => {
                // Per-position destructure of `a, b = rhs` (and `*rest`):
                // route through `flow_elem_into_target`, which projects each
                // tuple position via `TupleProject(Expr(value), i)`. Without
                // this, every leaf received the WHOLE tuple type — so
                // `_sb, _sd = SfBase(), SfDerived()` left both `_sb` and
                // `_sd` typed `tuple[SfBase, SfDerived]`, and `_sb * _sd`
                // (a class dunder dispatch) failed to resolve `.tag`.
                self.flow_elem_into_target(target, TypeKey::Expr(value));
            }
            BindingTarget::Attr { obj, field, .. } => {
                // Visit the object expression so its type gets bound.
                self.collect_expr(*obj);
                // If `obj` is a `Var(v)` with a known class, route the
                // store to `ClassField(class_id, field)` via the
                // `FieldWrite` reducer. This is the primary path for
                // `self.x = …` inside methods.
                if let Some(class_id) = self.obj_class_hint(*obj) {
                    self.solver.add(Constraint::FieldWrite {
                        class: class_id,
                        name: *field,
                        value: TypeKey::Expr(value),
                    });
                }
                // Dynamic-class case (obj is an expression / Var with
                // inferred-only Class type): no static FieldWrite emit
                // here — the production ctx in S5 wire-in will handle
                // the long tail via cross-instance field harvesting.
            }
            BindingTarget::Index { obj, index, .. } => {
                self.collect_expr(*obj);
                self.collect_expr(*index);
                // Dict-write refinement: `obj[k] = value` where `obj` is a
                // `Var(v)`. Emit `dict[key_ty, value_ty]` via a single-pair
                // ContainerLiteral and flow it into `Var(v)`. Lattice JOIN
                // refines `dict[Never, Never]` (from `v = {}`) into
                // `dict[K, V]`. Solver-native equivalent of the dict half
                // of legacy `find_dict_types_from_usage`.
                if let ExprKind::Var(var) = &self.module.exprs[*obj].kind {
                    let pair_meta = self.solver.fresh_meta();
                    self.solver.add(Constraint::ContainerLiteral {
                        result: pair_meta,
                        kind: ContainerKind::Dict,
                        elems: Vec::new(),
                        kv: vec![(TypeKey::Expr(*index), TypeKey::Expr(value))],
                    });
                    self.solver.add(Constraint::FlowsInto {
                        src: pair_meta,
                        dst: TypeKey::Var(*var),
                    });
                }
            }
            BindingTarget::ClassAttr { class_id, attr, .. } => {
                // `ClassName.attr = …` writes to a class-level attribute.
                // Treat the same as a `FieldWrite` for now — the
                // distinction between class-attr and instance-attr is
                // handled by the materialization layer in S5.
                self.solver.add(Constraint::FieldWrite {
                    class: *class_id,
                    name: *attr,
                    value: TypeKey::Expr(value),
                });
            }
        }
    }

    /// Walk an expression and emit its constraint. Idempotent: re-visiting
    /// a shared subexpression is a no-op.
    fn collect_expr(&mut self, eid: ExprId) {
        if !self.visited.insert(eid) {
            return;
        }
        let expr = &self.module.exprs[eid];
        let key = TypeKey::Expr(eid);
        match &expr.kind {
            ExprKind::Int(_) => {
                self.solver.add(Constraint::Concrete(key, Type::Int));
            }
            ExprKind::Float(_) => {
                self.solver.add(Constraint::Concrete(key, Type::Float));
            }
            ExprKind::Bool(_) => {
                self.solver.add(Constraint::Concrete(key, Type::Bool));
            }
            ExprKind::Str(_) => {
                self.solver.add(Constraint::Concrete(key, Type::Str));
            }
            ExprKind::Bytes(_) => {
                self.solver.add(Constraint::Concrete(key, Type::Bytes));
            }
            ExprKind::None => {
                self.solver.add(Constraint::Concrete(key, Type::None));
            }
            ExprKind::NotImplemented => {
                self.solver
                    .add(Constraint::Concrete(key, Type::NotImplementedT));
            }
            ExprKind::Var(v) => {
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::Var(*v),
                    dst: key,
                });
            }
            ExprKind::BinOp { op, left, right } => {
                self.collect_expr(*left);
                self.collect_expr(*right);
                self.solver.add(Constraint::BinOp {
                    result: key,
                    op: *op,
                    lhs: TypeKey::Expr(*left),
                    rhs: TypeKey::Expr(*right),
                });
            }
            ExprKind::UnOp { op, operand } => {
                self.collect_expr(*operand);
                self.solver.add(Constraint::UnaryOp {
                    result: key,
                    op: *op,
                    operand: TypeKey::Expr(*operand),
                });
            }
            ExprKind::Compare { left, right, .. } => {
                self.collect_expr(*left);
                self.collect_expr(*right);
                self.solver.add(Constraint::Concrete(key, Type::Bool));
            }
            ExprKind::LogicalOp { left, right, .. } => {
                self.collect_expr(*left);
                self.collect_expr(*right);
                // Logical and/or returns one of the operands — model as
                // two FlowsInto edges so the lattice joins them.
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::Expr(*left),
                    dst: key,
                });
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::Expr(*right),
                    dst: key,
                });
            }
            ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                self.collect_expr(*cond);
                self.collect_expr(*then_val);
                // While collecting the else arm, install the isinstance
                // negative-narrowing context so a coercion ctor `T(v)` in
                // `v if isinstance(v, T) else T(v)` doesn't pollute `T`'s
                // field with `T`.
                let saved = self.else_isinstance;
                self.else_isinstance = self.isinstance_cond(*cond);
                self.collect_expr(*else_val);
                self.else_isinstance = saved;
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::Expr(*then_val),
                    dst: key,
                });
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::Expr(*else_val),
                    dst: key,
                });
            }
            ExprKind::List(items) => {
                for &e in items {
                    self.collect_expr(e);
                }
                self.solver.add(Constraint::ContainerLiteral {
                    result: key,
                    kind: ContainerKind::List,
                    elems: items.iter().copied().map(TypeKey::Expr).collect(),
                    kv: Vec::new(),
                });
            }
            ExprKind::Tuple(items) => {
                for &e in items {
                    self.collect_expr(e);
                }
                self.solver.add(Constraint::ContainerLiteral {
                    result: key,
                    kind: ContainerKind::Tuple,
                    elems: items.iter().copied().map(TypeKey::Expr).collect(),
                    kv: Vec::new(),
                });
            }
            ExprKind::Set(items) => {
                for &e in items {
                    self.collect_expr(e);
                }
                self.solver.add(Constraint::ContainerLiteral {
                    result: key,
                    kind: ContainerKind::Set,
                    elems: items.iter().copied().map(TypeKey::Expr).collect(),
                    kv: Vec::new(),
                });
            }
            ExprKind::Dict(pairs) => {
                for &(k, v) in pairs {
                    self.collect_expr(k);
                    self.collect_expr(v);
                }
                self.solver.add(Constraint::ContainerLiteral {
                    result: key,
                    kind: ContainerKind::Dict,
                    elems: Vec::new(),
                    kv: pairs
                        .iter()
                        .map(|&(k, v)| (TypeKey::Expr(k), TypeKey::Expr(v)))
                        .collect(),
                });
            }
            ExprKind::Yield(Some(v)) => {
                self.collect_expr(*v);
                if let Some(fid) = self.current_func {
                    self.solver.add(Constraint::Yield {
                        func: fid,
                        value: TypeKey::Expr(*v),
                    });
                }
            }
            ExprKind::Yield(None) => {
                // `yield expr` evaluates to the value sent into the
                // generator — for S2 we leave the expression key
                // unbound (the existing planner already special-cases
                // this; the solver's S4 generator phase handles it).
            }
            ExprKind::TypeRef(ty) => {
                // `int` / `Foo` as a value — type-of is the referenced
                // type itself. The frontend pre-types this; preserve
                // by emitting Concrete.
                self.solver.add(Constraint::Concrete(key, ty.clone()));
            }
            ExprKind::ClassRef(class_id) => {
                // `Foo` as a value — its type is `Type::Class { id, name }`.
                // Class name comes from the module's class_defs table; if
                // the class is unknown (cross-module placeholder), leave
                // bottom and let materialization fall back.
                if let Some(class_def) = self.module.class_defs.get(class_id) {
                    self.solver.add(Constraint::Concrete(
                        key,
                        Type::Class {
                            class_id: *class_id,
                            name: class_def.name,
                        },
                    ));
                }
            }
            ExprKind::Index { obj, index } => {
                self.collect_expr(*obj);
                self.collect_expr(*index);
                self.solver.add(Constraint::Subscript {
                    result: key,
                    recv: TypeKey::Expr(*obj),
                    index: TypeKey::Expr(*index),
                });
            }
            ExprKind::Attribute { obj, attr } => {
                self.collect_expr(*obj);
                let attr_cid = self.solver.add(Constraint::Attribute {
                    result: key,
                    recv: TypeKey::Expr(*obj),
                    name: *attr,
                });
                // Static class-field path: if `obj` is a `Var(v)` with a
                // known class, add an explicit `FlowsInto` from
                // `ClassField(class_id, attr)` to `Expr(eid)`. This
                // registers the dependency edge so the worklist
                // re-evaluates this read when the field's type
                // updates — `eval_attribute`'s own ClassField
                // short-circuit handles the lookup, but the dependents
                // map only sees this `FlowsInto`.
                if let Some(class_id) = self.obj_class_hint(*obj) {
                    self.solver.add(Constraint::FlowsInto {
                        src: TypeKey::ClassField(class_id, *attr),
                        dst: key,
                    });
                } else if let Some(classes) = self.field_to_classes.get(attr).cloned() {
                    // Dynamic receiver (class resolved during solving, e.g.
                    // `keys[0][0].data`): register a dependency edge — NOT a
                    // type flow — on every class that defines `attr`, so the
                    // Attribute reducer (which reads `ClassField(resolved,
                    // attr)`) re-evaluates when that field's type is refined
                    // late. A type flow would wrongly JOIN every candidate
                    // class's field type into the result; the reducer already
                    // selects the actual resolved class's field.
                    for cid_class in classes {
                        self.solver
                            .add_dep(TypeKey::ClassField(cid_class, *attr), attr_cid);
                    }
                }
            }
            ExprKind::ClassAttrRef { class_id, attr } => {
                // `ClassName.attr` — read a class-level attribute. Route
                // through the same `ClassField` key the `FieldWrite`
                // reducer writes to.
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::ClassField(*class_id, *attr),
                    dst: key,
                });
            }
            ExprKind::MethodCall {
                obj,
                method,
                args,
                kwargs,
                ..
            } => {
                self.collect_expr(*obj);
                for &a in args {
                    self.collect_expr(a);
                }
                for kw in kwargs {
                    self.collect_expr(kw.value);
                }
                // Self-bootstrap a call to a statically-resolvable local
                // user method through its `FuncReturn` — but ONLY when the
                // method may return `NotImplemented`. The `MethodCall` ctx
                // path reads the stale Lowering return and reports `None`
                // for such methods, so the inferred caller return omits
                // `NotImplementedType` and the lowering return-check fails.
                // Routing through `FuncReturn` recovers it. Restricted to
                // NI-returners: routing *every* self-method call this way
                // regressed the verifier (precise `Raw` returns clashed
                // with `Tagged` call dests). We emit EITHER the FuncReturn
                // flow OR the MethodCall constraint — never both, so the
                // ctx's stale `None` can't re-pollute the result via JOIN.
                let ni_route = self
                    .resolve_local_method(*obj, *method)
                    .filter(|mfid| self.ni_methods.contains(mfid));
                if let Some(mfid) = ni_route {
                    self.solver.add(Constraint::FlowsInto {
                        src: TypeKey::FuncReturn(mfid),
                        dst: key,
                    });
                } else {
                    self.solver.add(Constraint::MethodCall {
                        result: key,
                        recv: TypeKey::Expr(*obj),
                        name: *method,
                        args: args.iter().copied().map(TypeKey::Expr).collect(),
                    });
                }
                // Flow the call args into the resolved method's params (skip
                // `self` at index 0) when the receiver's class is statically
                // known. An unannotated method param (`store` in
                // `def add(self, store, k): store.append(k)`) otherwise stays
                // `Any`, and lowering's method dispatch picks the wrong
                // runtime (`rt_deque_append` instead of `rt_list_append`) →
                // garbage / SIGSEGV. Mirrors the regular-Call / ctor
                // `LambdaParamHint` arg→param flow.
                if let Some(mfid) = self.resolve_local_method(*obj, *method) {
                    for (ix, &a) in args.iter().enumerate() {
                        self.solver.add(Constraint::LambdaParamHint {
                            func: mfid,
                            param_ix: ix + 1,
                            hint: TypeKey::Expr(a),
                        });
                    }
                }
                // `list.sort(key=f)` applies `f` to each element of the
                // receiver list; route the receiver's element type to the
                // key callback's user parameter (same SIGSEGV-avoidance
                // rationale as the `sorted`/`min`/`max` builtin path).
                self.propagate_method_key_hint(*obj, *method, kwargs);
                // Empty-container element-type refinement: when the
                // receiver is a `Var(v)` and the method is a known
                // mutator (`list.append`/`set.add` etc.), back-propagate
                // the argument's type into `Var(v)` via a single-element
                // ContainerLiteral. The lattice JOIN refines
                // `list[Never]` (from `v = []`) into `list[T]` once `T`
                // is observed. Solver-native replacement for the legacy
                // `refine_empty_container_types` pass.
                self.propagate_mutator_elem_refinement(*obj, *method, args);
            }
            ExprKind::Call {
                func, args, kwargs, ..
            } => {
                self.collect_expr(*func);
                // Resolve the callee kind by inspecting the func expression.
                // FuncRef → known function id. ClassRef → constructor.
                // Closure → directly dispatch to the closure's body via Func.
                // Anything else (Var, Attribute, etc.) → dynamic dispatch
                // through the callee's env type.
                let callee = match &self.module.exprs[*func].kind {
                    ExprKind::FuncRef(fid) => CalleeRef::Func(*fid),
                    ExprKind::ClassRef(cid) => CalleeRef::ClassCtor(*cid),
                    ExprKind::Closure { func: fid, .. } => CalleeRef::Func(*fid),
                    // Indirect call through a variable that holds a single
                    // capture-free lambda / function reference: resolve it
                    // to the concrete target so the call args hint its
                    // params and the result reads its `FuncReturn` (the
                    // `CalleeRef::Func` arms below). Lowering already lowers
                    // this to a `CallDirect`, so the typing now matches the
                    // direct-call ABI instead of leaving the params `Any`.
                    ExprKind::Var(v) if self.var_to_lambda.contains_key(v) => {
                        CalleeRef::Func(self.var_to_lambda[v])
                    }
                    _ => CalleeRef::Dynamic(TypeKey::Expr(*func)),
                };
                // Number of leading capture params on the resolved callee.
                // A lifted closure delivers its captures as the LEADING
                // positional params, so the call's positional args map to
                // params `[cap_count..]`, NOT `[0..]`. An immediately-invoked
                // capturing closure (`c = 1; (lambda x: x + c)(5)`) lowers to
                // a `Func` whose params are `[c, x]`; without this offset arg
                // `5` would hint the `c` capture slot and leave `x` untyped —
                // the same mis-slotting `propagate_hof_iterable_hint` guards
                // against with `cap_count + i`. `FuncRef` and the capture-free
                // `var_to_lambda` arm contribute 0.
                let callee_cap_count = match &self.module.exprs[*func].kind {
                    ExprKind::Closure { captures, .. } => captures.len(),
                    _ => 0,
                };
                // Walk positional args.
                let mut arg_keys = Vec::with_capacity(args.len());
                for arg in args {
                    match arg {
                        hir::CallArg::Regular(eid) | hir::CallArg::Starred(eid) => {
                            self.collect_expr(*eid);
                            arg_keys.push(TypeKey::Expr(*eid));
                        }
                    }
                }
                // Walk keyword args.
                let mut kw_keys = Vec::with_capacity(kwargs.len());
                for kw in kwargs {
                    self.collect_expr(kw.value);
                    kw_keys.push((kw.name, TypeKey::Expr(kw.value)));
                }
                // S4: emit LambdaParamHint for each positional arg when
                // the callee is a known function. This drives cross-
                // function inference for unannotated lambda/closure
                // params (which connect LambdaParam → Var inside
                // `collect_function`). Annotated params have a
                // `Concrete` lower bound and are insulated from these
                // hints by the absence of the LambdaParam → Var edge.
                if let CalleeRef::Func(target_fid) = callee {
                    for (ix, &arg_key) in arg_keys.iter().enumerate() {
                        self.solver.add(Constraint::LambdaParamHint {
                            func: target_fid,
                            param_ix: callee_cap_count + ix,
                            hint: arg_key,
                        });
                    }
                    // Inter-procedural mutation flow-back: if the callee
                    // mutates a container param in place (`keys[i].append`),
                    // that mutation also refines the caller's argument object
                    // (Python passes containers by reference). Flow the
                    // (mutation-refined) param `Var` back into a `Var`
                    // argument so `ks = [[] for _ in range(n)]; fill(ks)`
                    // sees `ks : list[list[T]]` after the call. Gated on the
                    // param being mutated, so a reassigned-only param never
                    // widens the caller's var. Param indices in `mutated` are
                    // lifted-ABI (capture-inclusive); the arg-relative `ix`
                    // therefore maps to lifted param `callee_cap_count + ix`.
                    if let Some(mutated) = self.mutated_param_indices.get(&target_fid).cloned() {
                        for (ix, arg) in args.iter().enumerate() {
                            let param_ix = callee_cap_count + ix;
                            if !mutated.contains(&param_ix) {
                                continue;
                            }
                            let (hir::CallArg::Regular(arg_eid) | hir::CallArg::Starred(arg_eid)) =
                                arg;
                            let ExprKind::Var(arg_v) = &self.module.exprs[*arg_eid].kind else {
                                continue;
                            };
                            let Some(param_var) = self
                                .module
                                .func_defs
                                .get(&target_fid)
                                .and_then(|f| f.params.get(param_ix))
                                .map(|p| p.var)
                            else {
                                continue;
                            };
                            self.solver.add(Constraint::FlowsInto {
                                src: TypeKey::Var(param_var),
                                dst: TypeKey::Var(*arg_v),
                            });
                        }
                    }
                }
                // Constructor call `C(a, b)` → hint `__init__`'s params.
                // `__init__`'s param 0 is `self`, so the call's positional
                // args map to params `[1..]`. Without this, an unannotated
                // `__init__(self, children=())` param stays at its default
                // type (`()` → empty tuple), so `self._children = children`
                // (a `FieldWrite`) types the field as empty and a later
                // `for child in self._children` loop yields `Never`. Mirrors
                // the legacy constructor-call harvester (`harvest_skip = 1`).
                if let CalleeRef::ClassCtor(cid) = callee {
                    if let Some(init_fid) =
                        self.module.class_defs.get(&cid).and_then(|c| c.init_method)
                    {
                        for (ix, (arg, &arg_key)) in args.iter().zip(arg_keys.iter()).enumerate() {
                            // Suppress the coercion-idiom self-pollution:
                            // `T(v)` in the else arm of `isinstance(v, T)`
                            // would flow `v`'s T-inclusive type into `T`'s
                            // `__init__` param (→ `T`'s field gains a spurious
                            // `T` branch). In that arm `v` is NOT a `T`.
                            let (hir::CallArg::Regular(arg_eid) | hir::CallArg::Starred(arg_eid)) =
                                arg;
                            let is_coercion_self = matches!(
                                self.else_isinstance,
                                Some((nv, ncid))
                                    if ncid == cid
                                        && matches!(
                                            &self.module.exprs[*arg_eid].kind,
                                            ExprKind::Var(v) if *v == nv
                                        )
                            );
                            if is_coercion_self {
                                continue;
                            }
                            self.solver.add(Constraint::LambdaParamHint {
                                func: init_fid,
                                param_ix: ix + 1,
                                hint: arg_key,
                            });
                        }
                    }
                }
                self.solver.add(Constraint::Call {
                    result: key,
                    callee,
                    args: arg_keys,
                    kwargs: kw_keys,
                });
            }
            ExprKind::Closure { func, captures } => {
                // Per-slot capture constraints so the materializer records
                // upvalue types for `closures.closure_capture_types`.
                for (slot, &cap_eid) in captures.iter().enumerate() {
                    self.collect_expr(cap_eid);
                    self.solver.add(Constraint::Capture {
                        func: *func,
                        slot,
                        src: TypeKey::Expr(cap_eid),
                    });
                    // Lifted-closure ABI: capture slot `i` is delivered to
                    // the callee as its leading positional param `i` (this
                    // is what `precompute_closure_capture_types` relies on
                    // when it seeds `func.params.get(i)` from
                    // `closure_capture_types[i]`). Flow the capture type
                    // into that param's `Var` so the callee body — and any
                    // DEEPER re-capture — sees the concrete captured type.
                    // Without this edge a transitive capture
                    // (`outer.x → mid → inner`) leaves `mid`'s internal
                    // capture-param `Var` untyped, so `inner`'s re-capture
                    // of it resolves to `Any`; the resulting `Any` closure
                    // capture then forces a Tagged return-ABI on the callee
                    // while the caller's dest stays `Raw(I64)` →
                    // `CallDirect: return … Tagged not assignable to dest
                    // Raw(I64)` at final-pre-codegen. Capture params carry
                    // no call-site `LambdaParamHint` (captures are implicit,
                    // not in `Call.args`), so this is the sole type source
                    // for them and cannot be polluted by an `Any` hint.
                    if let Some(param) = self
                        .module
                        .func_defs
                        .get(func)
                        .and_then(|f| f.params.get(slot))
                    {
                        let pvar = param.var;
                        self.solver.add(Constraint::FlowsInto {
                            src: TypeKey::Capture(*func, slot),
                            dst: TypeKey::Var(pvar),
                        });
                    }
                    // Mirror the capture type into the leading
                    // `LambdaParam` slot so `lambda_param_type_hints` carries
                    // `[capture_0, …, capture_{n-1}, <call-site element>]`
                    // for the callee — matching the legacy
                    // `register_lambda_hints_from_iterable`, which prepends
                    // capture types to the hint vector. Lowering's prologue
                    // unbox decision for a HOF callback reads
                    // `lambda_param_type_hints`; without the capture slots
                    // filled, a captured comparison operand
                    // (`lambda x: x > lo and x < hi` capturing `lo`/`hi`)
                    // stays `Any` in the hints, the prologue skips its
                    // unbox, and the body operates on the still-tagged cell
                    // value → `BinOp operand Tagged is not Raw`.
                    self.solver.add(Constraint::LambdaParamHint {
                        func: *func,
                        param_ix: slot,
                        hint: TypeKey::Capture(*func, slot),
                    });
                }
                // A GENERATOR closure used as a value IS the generator
                // object — its type is `Iterator[yield]`, i.e. the
                // generator's `FuncReturn` (which `GeneratorReturn` derives
                // as `Iterator(FuncYield)`). This is exactly the genexp case
                // `sum(x for x in …)`: the genexp desugars to a generator
                // closure passed to `sum`; without this the closure value is
                // `Any`, `sum`'s element resolves to `Any`, and the result
                // pollutes the consumer (`list[Any]` instead of `list[V]`).
                // Non-generator (lambda) closures stay `Any` — direct lambda
                // calls bypass this path via `CalleeRef::Func`.
                let is_gen = self
                    .module
                    .func_defs
                    .get(func)
                    .map(|f| f.is_generator)
                    .unwrap_or(false);
                if is_gen {
                    self.solver.add(Constraint::FlowsInto {
                        src: TypeKey::FuncReturn(*func),
                        dst: key,
                    });
                } else {
                    self.solver.add(Constraint::Concrete(key, Type::Any));
                }
            }
            ExprKind::BuiltinCall {
                builtin,
                args,
                kwargs,
                ..
            } => {
                // Walk args first so their types are available when
                // `eval_call` reduces this constraint.
                let mut arg_keys = Vec::with_capacity(args.len());
                for &eid in args {
                    self.collect_expr(eid);
                    arg_keys.push(TypeKey::Expr(eid));
                }
                // Collect keyword-arg values too — notably the `key=`
                // callback of `sorted`/`min`/`max`, whose closure body and
                // captures must be wired into the solver.
                for kw in kwargs {
                    self.collect_expr(kw.value);
                }
                // HOF iterable→callback hint propagation: type a
                // `map`/`filter` callback's element parameter from the
                // iterable's element type. The earlier attempt regressed
                // because it hinted param index 0 (a capture slot) instead
                // of the post-capture element slot; `propagate_hof_iterable_hint`
                // now offsets by the callback's capture count, so the hint
                // lands on the element param and captures keep their own
                // (Capture-derived) types.
                self.propagate_hof_iterable_hint(*builtin, args);
                // `sorted`/`min`/`max` accept a `key=` callable applied to
                // each element; route the iterable's element type to the
                // key callback's user parameter so its body types precisely
                // (without this the key lambda's param stays `Any`/`Tagged`
                // and the HOF runtime passes a raw element into a
                // tagged-expecting parameter → SIGSEGV).
                self.propagate_hof_key_hint(*builtin, args, kwargs);
                // `map(callback, …)` result is `Iterator[callback_return]`.
                // The generic builtin reducer only sees the callback's
                // *value* type (`Any` — closures are `Any`-valued), so it
                // can't recover the element type; derive it directly from
                // the callback's `FuncReturn` instead. Without this the map
                // result is `Iterator[Any]`, and a consumer
                // (`filter(p, map(f, xs))` / a `for` loop) infers `Any`
                // elements, defeating the callback-param hints downstream.
                let map_callback = if matches!(*builtin, hir::Builtin::Map) {
                    match args.first().map(|a| &self.module.exprs[*a].kind) {
                        Some(ExprKind::Closure { func, .. } | ExprKind::FuncRef(func)) => {
                            Some(*func)
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(cb) = map_callback {
                    self.solver.add(Constraint::WrapIterator {
                        result: key,
                        elem: TypeKey::FuncReturn(cb),
                    });
                } else {
                    self.solver.add(Constraint::Call {
                        result: key,
                        callee: CalleeRef::Builtin(super::vocab::BuiltinId(*builtin)),
                        args: arg_keys,
                        kwargs: Vec::new(),
                    });
                }
            }
            ExprKind::FuncRef(_) => {
                // A function reference used as a value (e.g. `f = some_fn`).
                // No precise type; `Type::Any` so downstream calls through
                // a Var holding a func-ref dispatch via `Call(Dynamic)`
                // and fall back to `Any` — same precision as legacy.
                self.solver.add(Constraint::Concrete(key, Type::Any));
            }
            ExprKind::Slice {
                obj,
                start,
                end,
                step,
            } => {
                self.collect_expr(*obj);
                if let Some(e) = start {
                    self.collect_expr(*e);
                }
                if let Some(e) = end {
                    self.collect_expr(*e);
                }
                if let Some(e) = step {
                    self.collect_expr(*e);
                }
                // A slice preserves the container type (`xs[a:b]` has the
                // same shape/element type as `xs`). Flow `obj → slice` so
                // `q = xs[0:2]; q[j].attr` resolves `q`'s element type.
                self.solver.add(Constraint::FlowsInto {
                    src: TypeKey::Expr(*obj),
                    dst: key,
                });
            }
            ExprKind::IterHasNext(inner) => {
                self.collect_expr(*inner);
                // Always Bool — pure predicate.
                self.solver.add(Constraint::Concrete(key, Type::Bool));
            }
            ExprKind::MatchPattern { subject, .. } => {
                self.collect_expr(*subject);
                self.solver.add(Constraint::Concrete(key, Type::Bool));
            }
            ExprKind::FormatSpec { value, .. } => {
                // `f"{value:spec}"` — recurse into the formatted value so a
                // nested call (notably `sorted(xs, key=lambda …)` embedded
                // in an f-string) is collected and its key callback gets
                // hinted. Without this the inner HOF callback's param stays
                // `Any`/Tagged while the runtime delivers raw scalars.
                self.collect_expr(*value);
                self.solver.add(Constraint::Concrete(key, Type::Str));
            }
            // Deferred to follow-up S6 passes: SuperCall, ImportedRef,
            // ModuleAttr, BuiltinRef, StdlibAttr/Call/Const,
            // ExcCurrentValue, GeneratorIntrinsic.
            //
            // These leave `Expr(eid)` at the lattice bottom in env.
            // Materialization falls back to the expression's existing
            // `expr.ty` (often pre-typed by the frontend) for these
            // until the corresponding stage lands.
            _ => {}
        }
    }
}

/// Top-level entry point: collect every constraint from `module` into
/// `solver`. Call before `Solver::run`.
///
/// `interner` is needed for method-name dispatch in
/// `propagate_mutator_elem_refinement`. Tests that don't exercise that
/// path can pass an empty interner via [`collect_with_empty_interner`].
pub fn collect(solver: &mut Solver, module: &hir::Module, interner: &StringInterner) {
    Collector::new(solver, module, interner).collect_module();
}

/// Test convenience: build a fresh empty interner and call [`collect`].
/// Real production code routes through [`collect`] with the
/// `Lowering`'s live interner so method-name lookups resolve correctly.
#[cfg(test)]
pub fn collect_with_empty_interner(solver: &mut Solver, module: &hir::Module) {
    let interner = StringInterner::new();
    collect(solver, module, &interner);
}

#[cfg(test)]
mod tests {
    use super::super::solve::PermissiveCtx;
    use super::*;
    use pyaot_hir as hir;
    use pyaot_types::Type;
    use pyaot_utils::{ClassId, FuncId, HirBlockId, InternedString, Span, StringInterner, VarId};

    fn expr(kind: hir::ExprKind) -> hir::Expr {
        hir::Expr {
            kind,
            ty: None,
            span: Span::dummy(),
        }
    }

    fn stmt(kind: hir::StmtKind) -> hir::Stmt {
        hir::Stmt {
            kind,
            span: Span::dummy(),
        }
    }

    /// Build a minimal `hir::Function` with one entry block. Returns the
    /// FuncId so the caller can insert into the module.
    fn make_function(
        name: InternedString,
        fid: FuncId,
        params: Vec<hir::Param>,
        return_type: Option<Type>,
        entry_block: HirBlockId,
        block: hir::HirBlock,
    ) -> hir::Function {
        let mut blocks = indexmap::IndexMap::new();
        blocks.insert(entry_block, block);
        hir::Function {
            id: fid,
            name,
            params,
            return_type,
            span: Span::dummy(),
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: false,
            method_kind: hir::MethodKind::Static,
            is_abstract: false,
            blocks,
            entry_block,
            try_scopes: Vec::new(),
        }
    }

    fn make_param(name: InternedString, var: VarId, ty: Option<Type>) -> hir::Param {
        hir::Param {
            name,
            var,
            ty,
            default: None,
            kind: hir::ParamKind::Regular,
            span: Span::dummy(),
        }
    }

    fn make_block(bid: HirBlockId, stmts: Vec<hir::StmtId>, term: HirTerminator) -> hir::HirBlock {
        hir::HirBlock {
            id: bid,
            stmts,
            terminator: term,
            loop_depth: 0,
            handler_depth: 0,
        }
    }

    /// `def f(x: int) -> int: y = x + 1; return y`
    #[test]
    fn collect_simple_function_int_passthrough() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let x_name = interner.intern("x");

        let mut m = hir::Module::new(interner.intern("test"));
        let x_var = VarId::new(0);
        let y_var = VarId::new(1);

        // Allocate exprs: Var(x), Int(1), x+1, Var(y).
        let e_x = m.exprs.alloc(expr(hir::ExprKind::Var(x_var)));
        let e_one = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_add = m.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left: e_x,
            right: e_one,
        }));
        let e_y = m.exprs.alloc(expr(hir::ExprKind::Var(y_var)));

        // Allocate stmts: y = x + 1.
        let s_bind = m.stmts.alloc(stmt(hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(y_var),
            value: e_add,
            type_hint: None,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![s_bind], HirTerminator::Return(Some(e_y)));

        let fid = FuncId::new(0);
        let func = make_function(
            f_name,
            fid,
            vec![make_param(x_name, x_var, Some(Type::Int))],
            Some(Type::Int),
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        // Run solver.
        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // Expected results.
        assert_eq!(solver.env().get(TypeKey::Var(x_var)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::Expr(e_x)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::Expr(e_one)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::Expr(e_add)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::Var(y_var)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::Expr(e_y)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Int);
    }

    /// `def g(): return 1 + 2.0`  →  FuncReturn(g) = Float.
    #[test]
    fn collect_numeric_tower_widens_return() {
        let mut interner = StringInterner::new();
        let g_name = interner.intern("g");
        let mut m = hir::Module::new(interner.intern("test"));

        let e_one = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_two = m.exprs.alloc(expr(hir::ExprKind::Float(2.0)));
        let e_add = m.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left: e_one,
            right: e_two,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_add)));
        let fid = FuncId::new(1);
        let func = make_function(g_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Float);
    }

    /// `def h(): return [1, 2, 3]`  →  FuncReturn(h) = list[Int].
    #[test]
    fn collect_list_literal_return() {
        let mut interner = StringInterner::new();
        let h_name = interner.intern("h");
        let mut m = hir::Module::new(interner.intern("test"));

        let e1 = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e2 = m.exprs.alloc(expr(hir::ExprKind::Int(2)));
        let e3 = m.exprs.alloc(expr(hir::ExprKind::Int(3)));
        let e_list = m.exprs.alloc(expr(hir::ExprKind::List(vec![e1, e2, e3])));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_list)));
        let fid = FuncId::new(2);
        let func = make_function(h_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(
            solver.env().get(TypeKey::FuncReturn(fid)),
            Type::list_of(Type::Int)
        );
    }

    /// Tuple preserves per-position types: `(1, "a", 3.0)`.
    #[test]
    fn collect_tuple_preserves_fixed_shape() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        let s = interner.intern("a");
        let e1 = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e2 = m.exprs.alloc(expr(hir::ExprKind::Str(s)));
        let e3 = m.exprs.alloc(expr(hir::ExprKind::Float(3.0)));
        let e_tup = m.exprs.alloc(expr(hir::ExprKind::Tuple(vec![e1, e2, e3])));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_tup)));
        let fid = FuncId::new(3);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(
            solver.env().get(TypeKey::FuncReturn(fid)),
            Type::tuple_of(vec![Type::Int, Type::Str, Type::Float])
        );
    }

    /// `def f(): return 1 + 2.0  vs  return 0`  → FuncReturn widens to Float.
    #[test]
    fn collect_two_return_sites_widen_to_join() {
        let mut interner = StringInterner::new();
        let name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        // Two return blocks, no real conditional — both are entry-reachable
        // for this test (we don't model unreachability).
        let e_int = m.exprs.alloc(expr(hir::ExprKind::Int(0)));
        let e_one = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_two = m.exprs.alloc(expr(hir::ExprKind::Float(2.0)));
        let e_add = m.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left: e_one,
            right: e_two,
        }));

        // For simplicity put both returns into the entry block back-to-back —
        // semantically only the first runs, but the solver is purely
        // syntactic and joins both anyway.
        let entry = HirBlockId::new(0);
        let mid = HirBlockId::new(1);
        let mut blocks = indexmap::IndexMap::new();
        blocks.insert(
            entry,
            make_block(entry, vec![], HirTerminator::Return(Some(e_int))),
        );
        blocks.insert(
            mid,
            make_block(mid, vec![], HirTerminator::Return(Some(e_add))),
        );
        let fid = FuncId::new(4);
        let func = hir::Function {
            id: fid,
            name,
            params: vec![],
            return_type: None,
            span: Span::dummy(),
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: false,
            method_kind: hir::MethodKind::Static,
            is_abstract: false,
            blocks,
            entry_block: entry,
            try_scopes: Vec::new(),
        };
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // Int (from return 0) join Float (from 1 + 2.0) = Float.
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Float);
    }

    /// Annotated `Bind { type_hint: Some(Int) }` pins the variable type.
    #[test]
    fn collect_bind_with_annotation_pins_var_type() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));
        let v = VarId::new(0);

        let e_one = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let s_bind = m.stmts.alloc(stmt(hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(v),
            value: e_one,
            type_hint: Some(Type::Int),
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![s_bind], HirTerminator::Return(None));
        let fid = FuncId::new(5);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(solver.env().get(TypeKey::Var(v)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::None);
    }

    /// Compare expressions are always Bool.
    #[test]
    fn collect_compare_is_bool() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        let e1 = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e2 = m.exprs.alloc(expr(hir::ExprKind::Int(2)));
        let e_cmp = m.exprs.alloc(expr(hir::ExprKind::Compare {
            left: e1,
            op: hir::CmpOp::Lt,
            right: e2,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_cmp)));
        let fid = FuncId::new(6);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(solver.env().get(TypeKey::Expr(e_cmp)), Type::Bool);
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Bool);
    }

    /// IfExpr joins both branches.
    #[test]
    fn collect_ifexpr_joins_both_branches() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        let s = interner.intern("a");
        let e_cond = m.exprs.alloc(expr(hir::ExprKind::Bool(true)));
        let e_then = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_else = m.exprs.alloc(expr(hir::ExprKind::Str(s)));
        let e_if = m.exprs.alloc(expr(hir::ExprKind::IfExpr {
            cond: e_cond,
            then_val: e_then,
            else_val: e_else,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_if)));
        let fid = FuncId::new(7);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // Int and Str are incomparable → Union.
        match solver.env().get(TypeKey::FuncReturn(fid)) {
            Type::Union(members) => {
                assert_eq!(members.len(), 2);
                assert!(members.contains(&Type::Int));
                assert!(members.contains(&Type::Str));
            }
            other => panic!("expected Union[Int, Str], got {other:?}"),
        }
    }

    /// `def f() -> int: return` (bare return) → FuncReturn = None.
    #[test]
    fn collect_bare_return_is_none() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(None));
        let fid = FuncId::new(8);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::None);
    }

    /// Unsupported ExprKind (e.g. ClassRef) is silently skipped — Expr key
    /// stays at the lattice bottom, and the function gets no Return
    /// constraint resolved from it.
    #[test]
    fn collect_unsupported_expr_kind_leaves_key_at_bottom() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let mut m = hir::Module::new(interner.intern("test"));

        // ClassRef is in the S3+ category — collector should skip it.
        let class_id = ClassId::new(0);
        let e_cls = m.exprs.alloc(expr(hir::ExprKind::ClassRef(class_id)));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_cls)));
        let fid = FuncId::new(9);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // Expr(e_cls) has no Concrete or reducer producing it →
        // stays Never (bottom). FuncReturn picks up the bottom.
        // (ClassRef IS handled by the collector, but only if the class
        //  is registered in module.class_defs. This test uses an
        //  unregistered class id so the variant is skipped.)
        assert_eq!(solver.env().get(TypeKey::Expr(e_cls)), Type::Never);
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Never);
    }

    // -----------------------------------------------------------------
    // S3: end-to-end collector tests for the new compound-reducer paths.
    // -----------------------------------------------------------------

    /// `def f(xs: list[int]) -> int: return xs[0]`.
    /// Verifies the `Index` ExprKind → `Subscript` constraint path and
    /// that the list-element inline reducer fires.
    #[test]
    fn collect_index_expr_returns_list_elem() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let xs_name = interner.intern("xs");
        let mut m = hir::Module::new(interner.intern("test"));

        let xs_var = VarId::new(0);
        let e_xs = m.exprs.alloc(expr(hir::ExprKind::Var(xs_var)));
        let e_zero = m.exprs.alloc(expr(hir::ExprKind::Int(0)));
        let e_idx = m.exprs.alloc(expr(hir::ExprKind::Index {
            obj: e_xs,
            index: e_zero,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_idx)));
        let fid = FuncId::new(10);
        let func = make_function(
            f_name,
            fid,
            vec![hir::Param {
                name: xs_name,
                var: xs_var,
                ty: Some(Type::list_of(Type::Int)),
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            Some(Type::Int),
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(solver.env().get(TypeKey::Expr(e_idx)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Int);
    }

    /// `def f(d: dict[str, float]) -> float: return d["k"]`.
    /// Subscript on dict returns the value type.
    #[test]
    fn collect_index_expr_on_dict_returns_value_type() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let d_name = interner.intern("d");
        let key_str = interner.intern("k");
        let mut m = hir::Module::new(interner.intern("test"));

        let d_var = VarId::new(0);
        let e_d = m.exprs.alloc(expr(hir::ExprKind::Var(d_var)));
        let e_key = m.exprs.alloc(expr(hir::ExprKind::Str(key_str)));
        let e_idx = m.exprs.alloc(expr(hir::ExprKind::Index {
            obj: e_d,
            index: e_key,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_idx)));
        let fid = FuncId::new(11);
        let func = make_function(
            f_name,
            fid,
            vec![hir::Param {
                name: d_name,
                var: d_var,
                ty: Some(Type::dict_of(Type::Str, Type::Float)),
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            Some(Type::Float),
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Float);
    }

    /// `def caller() -> int: return f()` where `def f() -> int: return 42`.
    /// Verifies cross-function `Call` constraint with `CalleeRef::Func` —
    /// the solver bootstraps callee's FuncReturn before propagating to
    /// caller.
    #[test]
    fn collect_call_func_propagates_callee_return_type() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let caller_name = interner.intern("caller");
        let mut m = hir::Module::new(interner.intern("test"));

        // def f(): return 42
        let e_42 = m.exprs.alloc(expr(hir::ExprKind::Int(42)));
        let entry_f = HirBlockId::new(0);
        let block_f = make_block(entry_f, vec![], HirTerminator::Return(Some(e_42)));
        let fid_f = FuncId::new(20);
        let func_f = make_function(f_name, fid_f, vec![], None, entry_f, block_f);
        m.functions.push(fid_f);
        m.func_defs.insert(fid_f, func_f);

        // def caller(): return f()
        let e_funcref = m.exprs.alloc(expr(hir::ExprKind::FuncRef(fid_f)));
        let e_call = m.exprs.alloc(expr(hir::ExprKind::Call {
            func: e_funcref,
            args: Vec::new(),
            kwargs: Vec::new(),
            kwargs_unpack: None,
        }));
        let entry_c = HirBlockId::new(0);
        let block_c = make_block(entry_c, vec![], HirTerminator::Return(Some(e_call)));
        let fid_c = FuncId::new(21);
        let func_c = make_function(caller_name, fid_c, vec![], None, entry_c, block_c);
        m.functions.push(fid_c);
        m.func_defs.insert(fid_c, func_c);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid_f)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::Expr(e_call)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid_c)), Type::Int);
    }

    /// `def f() -> Foo: return Foo` where `class Foo: ...`.
    /// Verifies ClassRef emission as `Type::Class`.
    #[test]
    fn collect_classref_emits_class_type() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let foo_name = interner.intern("Foo");
        let mut m = hir::Module::new(interner.intern("test"));

        let class_id = ClassId::new(0);
        m.class_defs.insert(
            class_id,
            hir::ClassDef {
                id: class_id,
                name: foo_name,
                base_class: None,
                fields: Vec::new(),
                class_attrs: Vec::new(),
                methods: Vec::new(),
                init_method: None,
                properties: Vec::new(),
                abstract_methods: indexmap::IndexSet::new(),
                span: Span::dummy(),
                is_exception_class: false,
                is_protocol: false,
                base_exception_type: None,
                type_params: Vec::new(),
            },
        );

        let e_cls = m.exprs.alloc(expr(hir::ExprKind::ClassRef(class_id)));
        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_cls)));
        let fid = FuncId::new(22);
        let func = make_function(f_name, fid, vec![], None, entry, block);
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        let expected = Type::Class {
            class_id,
            name: foo_name,
        };
        assert_eq!(solver.env().get(TypeKey::Expr(e_cls)), expected);
    }

    /// `def f() -> Any: return obj.attr` where `obj` is some Var.
    /// Verifies Attribute constraint is emitted. With PermissiveCtx
    /// (`attribute_return → None`), the result falls back to `Any`.
    #[test]
    fn collect_attribute_emits_constraint_falls_back_to_any() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let obj_name = interner.intern("obj");
        let attr_name = interner.intern("attr");
        let mut m = hir::Module::new(interner.intern("test"));

        let obj_var = VarId::new(0);
        let e_obj = m.exprs.alloc(expr(hir::ExprKind::Var(obj_var)));
        let e_attr = m.exprs.alloc(expr(hir::ExprKind::Attribute {
            obj: e_obj,
            attr: attr_name,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_attr)));
        let fid = FuncId::new(23);
        let func = make_function(
            f_name,
            fid,
            vec![hir::Param {
                name: obj_name,
                var: obj_var,
                ty: Some(Type::Int), // dummy concrete type
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            None,
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // PermissiveCtx returns None for attribute_return → falls back to Any.
        assert_eq!(solver.env().get(TypeKey::Expr(e_attr)), Type::Any);
    }

    /// `def f(s: str) -> Any: return s.upper()`.
    /// Verifies MethodCall constraint is emitted. With PermissiveCtx
    /// → Any fallback.
    #[test]
    fn collect_method_call_emits_constraint() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let s_name = interner.intern("s");
        let method = interner.intern("upper");
        let mut m = hir::Module::new(interner.intern("test"));

        let s_var = VarId::new(0);
        let e_s = m.exprs.alloc(expr(hir::ExprKind::Var(s_var)));
        let e_call = m.exprs.alloc(expr(hir::ExprKind::MethodCall {
            obj: e_s,
            method,
            args: Vec::new(),
            kwargs: Vec::new(),
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![], HirTerminator::Return(Some(e_call)));
        let fid = FuncId::new(24);
        let func = make_function(
            f_name,
            fid,
            vec![hir::Param {
                name: s_name,
                var: s_var,
                ty: Some(Type::Str),
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            None,
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // PermissiveCtx → None → Any fallback. The constraint was emitted
        // (we know because the dispatch path ran); ctx just didn't resolve it.
        assert_eq!(solver.env().get(TypeKey::Expr(e_call)), Type::Any);
    }

    // -----------------------------------------------------------------
    // S4: end-to-end tests for class fields, captures, lambda hints,
    // and generator yields.
    // -----------------------------------------------------------------

    fn make_class_def(class_id: ClassId, name: pyaot_utils::InternedString) -> hir::ClassDef {
        hir::ClassDef {
            id: class_id,
            name,
            base_class: None,
            fields: Vec::new(),
            class_attrs: Vec::new(),
            methods: Vec::new(),
            init_method: None,
            properties: Vec::new(),
            abstract_methods: indexmap::IndexSet::new(),
            span: Span::dummy(),
            is_exception_class: false,
            is_protocol: false,
            base_exception_type: None,
            type_params: Vec::new(),
        }
    }

    /// `class Point: def m(self): self.x = 1; return self.x`.
    /// Verifies self.x = N → FieldWrite, self.x read → FlowsInto from
    /// ClassField. Round-trip: write Int, read Int.
    #[test]
    fn collect_class_field_write_then_read_round_trip() {
        let mut interner = StringInterner::new();
        let m_name = interner.intern("m");
        let self_name = interner.intern("self");
        let x_attr = interner.intern("x");
        let class_id = ClassId::new(0);
        let class_name = interner.intern("Point");
        let mut m = hir::Module::new(interner.intern("test"));
        m.class_defs
            .insert(class_id, make_class_def(class_id, class_name));

        let self_var = VarId::new(0);

        // Statement 1: `self.x = 1`
        let e_self_w = m.exprs.alloc(expr(hir::ExprKind::Var(self_var)));
        let e_one = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let s_write = m.stmts.alloc(stmt(hir::StmtKind::Bind {
            target: hir::BindingTarget::Attr {
                obj: e_self_w,
                field: x_attr,
                span: Span::dummy(),
            },
            value: e_one,
            type_hint: None,
        }));
        // Expression: `self.x` read
        let e_self_r = m.exprs.alloc(expr(hir::ExprKind::Var(self_var)));
        let e_read = m.exprs.alloc(expr(hir::ExprKind::Attribute {
            obj: e_self_r,
            attr: x_attr,
        }));

        let entry = HirBlockId::new(0);
        let block = make_block(entry, vec![s_write], HirTerminator::Return(Some(e_read)));
        let fid = FuncId::new(30);
        let func = make_function(
            m_name,
            fid,
            vec![hir::Param {
                name: self_name,
                var: self_var,
                ty: Some(Type::Class {
                    class_id,
                    name: class_name,
                }),
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            None,
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // ClassField(Point, "x") = Int (from `self.x = 1`).
        assert_eq!(
            solver.env().get(TypeKey::ClassField(class_id, x_attr)),
            Type::Int
        );
        // The read `self.x` propagates ClassField → Int.
        assert_eq!(solver.env().get(TypeKey::Expr(e_read)), Type::Int);
        // FuncReturn(m) = Int.
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid)), Type::Int);
    }

    /// Two methods write self.x with Int and Float → ClassField widens
    /// via numeric tower to Float (cross-instance refinement).
    #[test]
    fn collect_class_field_widens_across_two_method_writes() {
        let mut interner = StringInterner::new();
        let class_id = ClassId::new(0);
        let class_name = interner.intern("C");
        let x_attr = interner.intern("x");
        let self_name = interner.intern("self");
        let mut m = hir::Module::new(interner.intern("test"));
        m.class_defs
            .insert(class_id, make_class_def(class_id, class_name));

        // Method 1: writes Int.
        let build_writer = |interner: &mut StringInterner,
                            m: &mut hir::Module,
                            fid_val: u32,
                            fn_name: &str,
                            value_expr_kind: hir::ExprKind|
         -> FuncId {
            let f_name = interner.intern(fn_name);
            let self_var = VarId::new(fid_val);
            let e_self = m.exprs.alloc(expr(hir::ExprKind::Var(self_var)));
            let e_val = m.exprs.alloc(expr(value_expr_kind));
            let s_write = m.stmts.alloc(stmt(hir::StmtKind::Bind {
                target: hir::BindingTarget::Attr {
                    obj: e_self,
                    field: x_attr,
                    span: Span::dummy(),
                },
                value: e_val,
                type_hint: None,
            }));
            let entry = HirBlockId::new(0);
            let block = make_block(entry, vec![s_write], HirTerminator::Return(None));
            let fid = FuncId::new(fid_val);
            let func = make_function(
                f_name,
                fid,
                vec![hir::Param {
                    name: self_name,
                    var: self_var,
                    ty: Some(Type::Class {
                        class_id,
                        name: class_name,
                    }),
                    default: None,
                    kind: hir::ParamKind::Regular,
                    span: Span::dummy(),
                }],
                None,
                entry,
                block,
            );
            m.functions.push(fid);
            m.func_defs.insert(fid, func);
            fid
        };

        let _f1 = build_writer(&mut interner, &mut m, 40, "set_int", hir::ExprKind::Int(0));
        let _f2 = build_writer(
            &mut interner,
            &mut m,
            41,
            "set_float",
            hir::ExprKind::Float(0.5),
        );

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // Int join Float = Float.
        assert_eq!(
            solver.env().get(TypeKey::ClassField(class_id, x_attr)),
            Type::Float,
            "ClassField widens via numeric tower across cross-instance writes"
        );
    }

    /// `def f(x): return x + 1` called as `f(5)`. Unannotated `x`
    /// receives `Int` via `LambdaParamHint` → `FlowsInto LambdaParam → Var`.
    #[test]
    fn collect_lambda_param_hint_propagates_arg_type() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let caller_name = interner.intern("caller");
        let x_name = interner.intern("x");
        let mut m = hir::Module::new(interner.intern("test"));

        // def f(x):  return x + 1
        let x_var = VarId::new(0);
        let e_x = m.exprs.alloc(expr(hir::ExprKind::Var(x_var)));
        let e_one_f = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_add = m.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left: e_x,
            right: e_one_f,
        }));
        let entry_f = HirBlockId::new(0);
        let block_f = make_block(entry_f, vec![], HirTerminator::Return(Some(e_add)));
        let fid_f = FuncId::new(50);
        let func_f = make_function(
            f_name,
            fid_f,
            vec![hir::Param {
                name: x_name,
                var: x_var,
                ty: None, // UNANNOTATED — relies on LambdaParamHint.
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            None,
            entry_f,
            block_f,
        );
        m.functions.push(fid_f);
        m.func_defs.insert(fid_f, func_f);

        // def caller(): return f(5)
        let e_funcref = m.exprs.alloc(expr(hir::ExprKind::FuncRef(fid_f)));
        let e_arg = m.exprs.alloc(expr(hir::ExprKind::Int(5)));
        let e_call = m.exprs.alloc(expr(hir::ExprKind::Call {
            func: e_funcref,
            args: vec![hir::CallArg::Regular(e_arg)],
            kwargs: Vec::new(),
            kwargs_unpack: None,
        }));
        let entry_c = HirBlockId::new(0);
        let block_c = make_block(entry_c, vec![], HirTerminator::Return(Some(e_call)));
        let fid_c = FuncId::new(51);
        let func_c = make_function(caller_name, fid_c, vec![], None, entry_c, block_c);
        m.functions.push(fid_c);
        m.func_defs.insert(fid_c, func_c);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // LambdaParam(f, 0) ← Int.
        assert_eq!(
            solver.env().get(TypeKey::LambdaParam(fid_f, 0)),
            Type::Int,
            "LambdaParamHint must join call-site arg type"
        );
        // Var(x) ← LambdaParam via FlowsInto for unannotated param.
        assert_eq!(solver.env().get(TypeKey::Var(x_var)), Type::Int);
        // The body `x + 1` resolves to Int.
        assert_eq!(solver.env().get(TypeKey::Expr(e_add)), Type::Int);
        // FuncReturn(f) = Int.
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid_f)), Type::Int);
        // Caller's call expr = Int.
        assert_eq!(solver.env().get(TypeKey::Expr(e_call)), Type::Int);
    }

    /// Immediately-invoked capturing closure: `(lambda x: ...)(5)` where the
    /// lambda captures one upvalue. The lifted closure's params are
    /// `[capture, x]`, so the call's positional arg `5` must hint the USER
    /// param at index `cap_count` (1), NOT index 0 (the capture slot).
    /// Regression test for the missing capture offset in the direct-Call
    /// `CalleeRef::Func` hint loop.
    #[test]
    fn collect_immediate_closure_call_hints_param_after_captures() {
        let mut interner = StringInterner::new();
        let g_name = interner.intern("g");
        let caller_name = interner.intern("caller");
        let cap_name = interner.intern("c");
        let x_name = interner.intern("x");
        let mut m = hir::Module::new(interner.intern("test"));

        // Lifted closure body `def g(c, x): return x` — params [capture c, user x].
        let cap_var = VarId::new(0);
        let x_var = VarId::new(1);
        let e_x = m.exprs.alloc(expr(hir::ExprKind::Var(x_var)));
        let entry_g = HirBlockId::new(0);
        let block_g = make_block(entry_g, vec![], HirTerminator::Return(Some(e_x)));
        let fid_g = FuncId::new(60);
        let func_g = make_function(
            g_name,
            fid_g,
            vec![
                hir::Param {
                    name: cap_name,
                    var: cap_var,
                    ty: None,
                    default: None,
                    kind: hir::ParamKind::Regular,
                    span: Span::dummy(),
                },
                hir::Param {
                    name: x_name,
                    var: x_var,
                    ty: None, // UNANNOTATED — relies on LambdaParamHint.
                    default: None,
                    kind: hir::ParamKind::Regular,
                    span: Span::dummy(),
                },
            ],
            None,
            entry_g,
            block_g,
        );
        m.functions.push(fid_g);
        m.func_defs.insert(fid_g, func_g);

        // def caller(): return (lambda x: x)(5)  — closure captures one upvalue.
        let e_cap = m.exprs.alloc(expr(hir::ExprKind::Int(99)));
        let e_closure = m.exprs.alloc(expr(hir::ExprKind::Closure {
            func: fid_g,
            captures: vec![e_cap],
        }));
        let e_arg = m.exprs.alloc(expr(hir::ExprKind::Int(5)));
        let e_call = m.exprs.alloc(expr(hir::ExprKind::Call {
            func: e_closure,
            args: vec![hir::CallArg::Regular(e_arg)],
            kwargs: Vec::new(),
            kwargs_unpack: None,
        }));
        let entry_c = HirBlockId::new(0);
        let block_c = make_block(entry_c, vec![], HirTerminator::Return(Some(e_call)));
        let fid_c = FuncId::new(61);
        let func_c = make_function(caller_name, fid_c, vec![], None, entry_c, block_c);
        m.functions.push(fid_c);
        m.func_defs.insert(fid_c, func_c);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // The arg `5` must hint the USER param at index 1 (after the single
        // capture), so the body `return x` resolves to Int.
        assert_eq!(
            solver.env().get(TypeKey::LambdaParam(fid_g, 1)),
            Type::Int,
            "call arg must hint the user param at index cap_count, not 0"
        );
        assert_eq!(solver.env().get(TypeKey::Var(x_var)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid_g)), Type::Int);
        assert_eq!(solver.env().get(TypeKey::Expr(e_call)), Type::Int);
    }

    /// Method call on a fresh instance: `Box().add(5)` where
    /// `def add(self, store): return store` has an UNANNOTATED `store`.
    /// The receiver is a constructor call (not a Var), so it never lands in
    /// `var_class_hints`; `resolve_local_method` must still resolve the
    /// method via `ctor_call_class` and hint `store` (param index 1) with
    /// the arg type `Int`. Regression test for the method-arg→param hint
    /// narrowing (the `rt_deque_append` vs `rt_list_append` hazard).
    #[test]
    fn collect_method_call_on_ctor_receiver_hints_param() {
        let mut interner = StringInterner::new();
        let box_name = interner.intern("Box");
        let add_name = interner.intern("add");
        let self_name = interner.intern("self");
        let store_name = interner.intern("store");
        let caller_name = interner.intern("caller");
        let class_id = ClassId::new(0);
        let mut m = hir::Module::new(interner.intern("test"));

        // def add(self, store): return store   (store UNANNOTATED)
        let self_var = VarId::new(0);
        let store_var = VarId::new(1);
        let e_store = m.exprs.alloc(expr(hir::ExprKind::Var(store_var)));
        let entry_add = HirBlockId::new(0);
        let block_add = make_block(entry_add, vec![], HirTerminator::Return(Some(e_store)));
        let fid_add = FuncId::new(70);
        let func_add = make_function(
            add_name,
            fid_add,
            vec![
                hir::Param {
                    name: self_name,
                    var: self_var,
                    ty: Some(Type::Class {
                        class_id,
                        name: box_name,
                    }),
                    default: None,
                    kind: hir::ParamKind::Regular,
                    span: Span::dummy(),
                },
                hir::Param {
                    name: store_name,
                    var: store_var,
                    ty: None, // UNANNOTATED — relies on the method arg hint.
                    default: None,
                    kind: hir::ParamKind::Regular,
                    span: Span::dummy(),
                },
            ],
            None,
            entry_add,
            block_add,
        );
        m.functions.push(fid_add);
        m.func_defs.insert(fid_add, func_add);

        let mut class_def = make_class_def(class_id, box_name);
        class_def.methods.push(fid_add);
        m.class_defs.insert(class_id, class_def);

        // def caller(): return Box().add(5)
        let e_classref = m.exprs.alloc(expr(hir::ExprKind::ClassRef(class_id)));
        let e_ctor = m.exprs.alloc(expr(hir::ExprKind::Call {
            func: e_classref,
            args: Vec::new(),
            kwargs: Vec::new(),
            kwargs_unpack: None,
        }));
        let e_arg = m.exprs.alloc(expr(hir::ExprKind::Int(5)));
        let e_call = m.exprs.alloc(expr(hir::ExprKind::MethodCall {
            obj: e_ctor,
            method: add_name,
            args: vec![e_arg],
            kwargs: Vec::new(),
        }));
        let entry_c = HirBlockId::new(0);
        let block_c = make_block(entry_c, vec![], HirTerminator::Return(Some(e_call)));
        let fid_c = FuncId::new(71);
        let func_c = make_function(caller_name, fid_c, vec![], None, entry_c, block_c);
        m.functions.push(fid_c);
        m.func_defs.insert(fid_c, func_c);

        let mut solver = Solver::new();
        // Pass the real interner — `resolve_local_method` resolves the
        // method name to a `&str` to match against the class's methods.
        collect(&mut solver, &m, &interner);
        solver.run(&PermissiveCtx);

        // The arg `5` must hint `store` (param index 1, after `self`),
        // resolving the method via the ctor-call receiver.
        assert_eq!(
            solver.env().get(TypeKey::LambdaParam(fid_add, 1)),
            Type::Int,
            "ctor-receiver method call must hint the unannotated param"
        );
        assert_eq!(solver.env().get(TypeKey::Var(store_var)), Type::Int);
    }

    /// Annotated parameter is insulated from call-site hints.
    #[test]
    fn collect_annotated_param_not_widened_by_call_site_hints() {
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let caller_name = interner.intern("caller");
        let x_name = interner.intern("x");
        let s_name = interner.intern("hello");
        let mut m = hir::Module::new(interner.intern("test"));

        // def f(x: int): return x  (annotated as Int)
        let x_var = VarId::new(0);
        let e_x = m.exprs.alloc(expr(hir::ExprKind::Var(x_var)));
        let entry_f = HirBlockId::new(0);
        let block_f = make_block(entry_f, vec![], HirTerminator::Return(Some(e_x)));
        let fid_f = FuncId::new(60);
        let func_f = make_function(
            f_name,
            fid_f,
            vec![hir::Param {
                name: x_name,
                var: x_var,
                ty: Some(Type::Int),
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            None,
            entry_f,
            block_f,
        );
        m.functions.push(fid_f);
        m.func_defs.insert(fid_f, func_f);

        // def caller(): return f("hello")  // type-incorrect on purpose
        let e_funcref = m.exprs.alloc(expr(hir::ExprKind::FuncRef(fid_f)));
        let e_arg = m.exprs.alloc(expr(hir::ExprKind::Str(s_name)));
        let e_call = m.exprs.alloc(expr(hir::ExprKind::Call {
            func: e_funcref,
            args: vec![hir::CallArg::Regular(e_arg)],
            kwargs: Vec::new(),
            kwargs_unpack: None,
        }));
        let entry_c = HirBlockId::new(0);
        let block_c = make_block(entry_c, vec![], HirTerminator::Return(Some(e_call)));
        let fid_c = FuncId::new(61);
        let func_c = make_function(caller_name, fid_c, vec![], None, entry_c, block_c);
        m.functions.push(fid_c);
        m.func_defs.insert(fid_c, func_c);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // The hint propagates into LambdaParam, but no FlowsInto edge
        // connects LambdaParam → Var for annotated params.
        assert_eq!(solver.env().get(TypeKey::LambdaParam(fid_f, 0)), Type::Str);
        // Var(x) stays Int — annotation insulated.
        assert_eq!(
            solver.env().get(TypeKey::Var(x_var)),
            Type::Int,
            "annotated param must not widen via call-site hints"
        );
        assert_eq!(solver.env().get(TypeKey::FuncReturn(fid_f)), Type::Int);
    }

    /// Closure capture: outer `x = 5` is captured as slot 0 of inner.
    /// Verifies `closure_capture_types[(inner, 0)] = Int` via materialize.
    #[test]
    fn collect_closure_capture_records_slot_type() {
        use super::super::materialize::materialize;
        let mut interner = StringInterner::new();
        let outer_name = interner.intern("outer");
        let inner_name = interner.intern("__lambda_inner");
        let x_name = interner.intern("x");
        let mut m = hir::Module::new(interner.intern("test"));

        // Inner closure (just a stub function for the capture target).
        let inner_fid = FuncId::new(71);
        let entry_inner = HirBlockId::new(0);
        let block_inner = make_block(entry_inner, vec![], HirTerminator::Return(None));
        let func_inner = make_function(
            inner_name,
            inner_fid,
            vec![],
            None,
            entry_inner,
            block_inner,
        );
        m.functions.push(inner_fid);
        m.func_defs.insert(inner_fid, func_inner);

        // Outer: x = 5; lambda_value = Closure { inner, captures: [x] }
        let x_var = VarId::new(0);
        let e_five = m.exprs.alloc(expr(hir::ExprKind::Int(5)));
        let s_bind_x = m.stmts.alloc(stmt(hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(x_var),
            value: e_five,
            type_hint: None,
        }));
        let e_xread = m.exprs.alloc(expr(hir::ExprKind::Var(x_var)));
        let e_closure = m.exprs.alloc(expr(hir::ExprKind::Closure {
            func: inner_fid,
            captures: vec![e_xread],
        }));
        let entry_outer = HirBlockId::new(0);
        let block_outer = make_block(
            entry_outer,
            vec![s_bind_x],
            HirTerminator::Return(Some(e_closure)),
        );
        let outer_fid = FuncId::new(70);
        let func_outer = make_function(
            outer_name,
            outer_fid,
            vec![],
            None,
            entry_outer,
            block_outer,
        );
        m.functions.push(outer_fid);
        m.func_defs.insert(outer_fid, func_outer);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // Capture(inner, 0) = Int (via the captured x's value).
        assert_eq!(solver.env().get(TypeKey::Capture(inner_fid, 0)), Type::Int);
        // Verify materialization surfaces this.
        let out = materialize(solver.env());
        assert_eq!(
            out.closure_capture_types.get(&(inner_fid, 0)),
            Some(&Type::Int)
        );
        // Suppress unused interner warning.
        let _ = x_name;
    }

    /// Generator with two yields of differing types → FuncYield widens.
    #[test]
    fn collect_generator_yield_widens_to_join() {
        use super::super::materialize::materialize;
        let mut interner = StringInterner::new();
        let gen_name = interner.intern("gen");
        let mut m = hir::Module::new(interner.intern("test"));

        let e_one = m.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let e_half = m.exprs.alloc(expr(hir::ExprKind::Float(2.5)));

        // Two blocks ending in Yield terminators (sequentially in
        // sequence — both reachable from entry for the test).
        let b0 = HirBlockId::new(0);
        let b1 = HirBlockId::new(1);
        let b2 = HirBlockId::new(2);
        let mut blocks = indexmap::IndexMap::new();
        blocks.insert(
            b0,
            make_block(
                b0,
                vec![],
                HirTerminator::Yield {
                    value: e_one,
                    resume_bb: b1,
                },
            ),
        );
        blocks.insert(
            b1,
            make_block(
                b1,
                vec![],
                HirTerminator::Yield {
                    value: e_half,
                    resume_bb: b2,
                },
            ),
        );
        blocks.insert(b2, make_block(b2, vec![], HirTerminator::Return(None)));

        let fid = FuncId::new(80);
        let func = hir::Function {
            id: fid,
            name: gen_name,
            params: vec![],
            return_type: None,
            span: Span::dummy(),
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: true,
            method_kind: hir::MethodKind::Static,
            is_abstract: false,
            blocks,
            entry_block: b0,
            try_scopes: Vec::new(),
        };
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        // FuncYield = Int join Float = Float.
        assert_eq!(solver.env().get(TypeKey::FuncYield(fid)), Type::Float);

        let out = materialize(solver.env());
        assert_eq!(out.func_yield_types.get(&fid), Some(&Type::Float));
    }

    /// Single-shot integration test: a small program exercising every
    /// new contract output (base_var_types, lambda_param_type_hints,
    /// closure_capture_types, refined_class_field_types, func_yield_types).
    #[test]
    fn materialize_exposes_all_s4_contract_outputs() {
        use super::super::materialize::materialize;
        let mut interner = StringInterner::new();
        let f_name = interner.intern("f");
        let self_name = interner.intern("self");
        let x_attr = interner.intern("data");
        let mut m = hir::Module::new(interner.intern("test"));

        let class_id = ClassId::new(0);
        let class_name = interner.intern("Box");
        m.class_defs
            .insert(class_id, make_class_def(class_id, class_name));

        // def f(self: Box) -> int:
        //     self.data = 7      # FieldWrite
        //     y = self.data      # field read via Attribute / ClassField FlowsInto
        //     return y
        let self_var = VarId::new(0);
        let y_var = VarId::new(1);
        let e_self_w = m.exprs.alloc(expr(hir::ExprKind::Var(self_var)));
        let e_seven = m.exprs.alloc(expr(hir::ExprKind::Int(7)));
        let s_write = m.stmts.alloc(stmt(hir::StmtKind::Bind {
            target: hir::BindingTarget::Attr {
                obj: e_self_w,
                field: x_attr,
                span: Span::dummy(),
            },
            value: e_seven,
            type_hint: None,
        }));
        let e_self_r = m.exprs.alloc(expr(hir::ExprKind::Var(self_var)));
        let e_attr_r = m.exprs.alloc(expr(hir::ExprKind::Attribute {
            obj: e_self_r,
            attr: x_attr,
        }));
        let s_bind_y = m.stmts.alloc(stmt(hir::StmtKind::Bind {
            target: hir::BindingTarget::Var(y_var),
            value: e_attr_r,
            type_hint: None,
        }));
        let e_y = m.exprs.alloc(expr(hir::ExprKind::Var(y_var)));

        let entry = HirBlockId::new(0);
        let block = make_block(
            entry,
            vec![s_write, s_bind_y],
            HirTerminator::Return(Some(e_y)),
        );
        let fid = FuncId::new(90);
        let func = make_function(
            f_name,
            fid,
            vec![hir::Param {
                name: self_name,
                var: self_var,
                ty: Some(Type::Class {
                    class_id,
                    name: class_name,
                }),
                default: None,
                kind: hir::ParamKind::Regular,
                span: Span::dummy(),
            }],
            Some(Type::Int),
            entry,
            block,
        );
        m.functions.push(fid);
        m.func_defs.insert(fid, func);

        let mut solver = Solver::new();
        collect_with_empty_interner(&mut solver, &m);
        solver.run(&PermissiveCtx);

        let out = materialize(solver.env());
        // base_var_types contains both self and y.
        assert_eq!(
            out.base_var_types.get(&self_var),
            Some(&Type::Class {
                class_id,
                name: class_name,
            })
        );
        assert_eq!(out.base_var_types.get(&y_var), Some(&Type::Int));
        // refined_class_field_types tracks the field write.
        assert_eq!(
            out.refined_class_field_types.get(&(class_id, x_attr)),
            Some(&Type::Int)
        );
        // func_return_types[f] = Int.
        assert_eq!(out.func_return_types.get(&fid), Some(&Type::Int));
        // expr_types covers the read path (cache gate: Int is concrete).
        assert_eq!(out.expr_types.get(&e_y), Some(&Type::Int));
    }
}
