use super::*;

/// Collect every nested `class` reachable in the module `body`, in source order
/// (so `class_id` assignment is deterministic). Direct module-level classes are
/// NOT collected (the existing top-level loop owns them), but their bodies are
/// descended for `class`-in-`class`. This is a single forward, read-only
/// pre-scan (A3) that extends the existing top-level class collection.
pub(super) fn collect_nested_classdefs<'a>(body: &'a [Stmt], out: &mut Vec<NestedClass<'a>>) {
    let empty = HashSet::new();
    for s in body {
        match s {
            // A top-level class is not itself nested; its body may hold one.
            Stmt::ClassDef(c) => collect_nested_in_body(&c.body, &empty, out),
            // A top-level def's locals form the first enclosing scope.
            Stmt::FunctionDef(f) => {
                let enclosing = freevars::analyze_def(f).bound;
                collect_nested_in_body(&f.body, &enclosing, out);
            }
            // Module-level control flow: a class in here IS nested (module scope,
            // so no enclosing function locals).
            Stmt::If(_)
            | Stmt::While(_)
            | Stmt::For(_)
            | Stmt::Try(_)
            | Stmt::With(_)
            | Stmt::Match(_) => collect_nested_in_controlflow(s, &empty, out),
            _ => {}
        }
    }
}

/// Collect nested classes appearing in `stmts`, where a `ClassDef` at this level
/// IS nested. `enclosing` is the accumulated set of enclosing-function locals.
pub(super) fn collect_nested_in_body<'a>(
    stmts: &'a [Stmt],
    enclosing: &HashSet<String>,
    out: &mut Vec<NestedClass<'a>>,
) {
    for s in stmts {
        match s {
            Stmt::ClassDef(c) => {
                out.push(NestedClass {
                    def: c,
                    enclosing_locals: enclosing.clone(),
                });
                // class-in-class: inner classes are nested under the same scope.
                collect_nested_in_body(&c.body, enclosing, out);
            }
            Stmt::FunctionDef(f) => {
                let mut e = enclosing.clone();
                e.extend(freevars::analyze_def(f).bound);
                collect_nested_in_body(&f.body, &e, out);
            }
            Stmt::If(_)
            | Stmt::While(_)
            | Stmt::For(_)
            | Stmt::Try(_)
            | Stmt::With(_)
            | Stmt::Match(_) => collect_nested_in_controlflow(s, enclosing, out),
            _ => {}
        }
    }
}

/// Descend the sub-bodies of a control-flow statement (which do not introduce a
/// new function scope), collecting nested classes within them.
pub(super) fn collect_nested_in_controlflow<'a>(
    s: &'a Stmt,
    enclosing: &HashSet<String>,
    out: &mut Vec<NestedClass<'a>>,
) {
    match s {
        Stmt::If(s) => {
            collect_nested_in_body(&s.body, enclosing, out);
            collect_nested_in_body(&s.orelse, enclosing, out);
        }
        Stmt::While(s) => {
            collect_nested_in_body(&s.body, enclosing, out);
            collect_nested_in_body(&s.orelse, enclosing, out);
        }
        Stmt::For(s) => {
            collect_nested_in_body(&s.body, enclosing, out);
            collect_nested_in_body(&s.orelse, enclosing, out);
        }
        Stmt::Try(t) => {
            collect_nested_in_body(&t.body, enclosing, out);
            for h in &t.handlers {
                let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = h;
                collect_nested_in_body(&h.body, enclosing, out);
            }
            collect_nested_in_body(&t.orelse, enclosing, out);
            collect_nested_in_body(&t.finalbody, enclosing, out);
        }
        Stmt::With(w) => collect_nested_in_body(&w.body, enclosing, out),
        Stmt::Match(m) => {
            for case in &m.cases {
                collect_nested_in_body(&case.body, enclosing, out);
            }
        }
        _ => {}
    }
}

/// Lower a `class` definition: lower each method into `functions` (recording its
/// `FuncId`) and collect base names + class-level field annotations. The resolved
/// layout (MRO, slots, inherited members) is computed later in `semantics`.
/// Collect every method-call name `x.NAME(...)` reachable in `body`, recursing
/// into all nested function / method / class bodies. The Phase-B gate for
/// gradual-completeness method dispatch: an instance method whose name appears
/// here gets a uniform thunk so `rt_obj_method` can invoke it on a `Dyn`
/// receiver. Over-approximate by design — the frontend runs pre-typeck, so
/// whether a given receiver is `Dyn` is not yet known; method-call *syntax* is
/// the soundest available proxy, and building an unused thunk is harmless.
pub(super) fn collect_method_call_names(body: &[Stmt], out: &mut HashSet<String>) {
    for s in body {
        collect_calls_stmt(s, out);
    }
}

pub(super) fn collect_calls_stmt(s: &Stmt, out: &mut HashSet<String>) {
    let e = |x: &Expr, out: &mut HashSet<String>| collect_calls_expr(x, out);
    match s {
        Stmt::Expr(x) => e(&x.value, out),
        Stmt::Assign(a) => {
            e(&a.value, out);
            for t in &a.targets {
                e(t, out);
            }
        }
        Stmt::AugAssign(a) => {
            e(&a.value, out);
            e(&a.target, out);
        }
        Stmt::AnnAssign(a) => {
            if let Some(v) = &a.value {
                e(v, out);
            }
            e(&a.target, out);
        }
        Stmt::If(s) => {
            e(&s.test, out);
            collect_calls_body(&s.body, out);
            collect_calls_body(&s.orelse, out);
        }
        Stmt::While(s) => {
            e(&s.test, out);
            collect_calls_body(&s.body, out);
            collect_calls_body(&s.orelse, out);
        }
        Stmt::For(s) => {
            e(&s.iter, out);
            e(&s.target, out);
            collect_calls_body(&s.body, out);
            collect_calls_body(&s.orelse, out);
        }
        Stmt::Assert(s) => {
            e(&s.test, out);
            if let Some(m) = &s.msg {
                e(m, out);
            }
        }
        Stmt::Return(r) => {
            if let Some(v) = &r.value {
                e(v, out);
            }
        }
        // Descend into nested defs / methods (where `self.m()` calls live), plus
        // decorators and parameter defaults (this-scope expressions).
        Stmt::FunctionDef(d) => {
            for deco in &d.decorator_list {
                e(deco, out);
            }
            for awd in d
                .args
                .posonlyargs
                .iter()
                .chain(&d.args.args)
                .chain(&d.args.kwonlyargs)
            {
                if let Some(dflt) = &awd.default {
                    e(dflt, out);
                }
            }
            collect_calls_body(&d.body, out);
        }
        Stmt::ClassDef(c) => {
            for deco in &c.decorator_list {
                e(deco, out);
            }
            for b in &c.bases {
                e(b, out);
            }
            collect_calls_body(&c.body, out);
        }
        Stmt::Try(t) => {
            collect_calls_body(&t.body, out);
            for h in &t.handlers {
                let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = h;
                if let Some(ty) = &h.type_ {
                    e(ty, out);
                }
                collect_calls_body(&h.body, out);
            }
            collect_calls_body(&t.orelse, out);
            collect_calls_body(&t.finalbody, out);
        }
        Stmt::Raise(r) => {
            if let Some(x) = &r.exc {
                e(x, out);
            }
            if let Some(c) = &r.cause {
                e(c, out);
            }
        }
        Stmt::With(w) => {
            // A `with` desugars to synthetic `mgr.__enter__()` / `mgr.__exit__(...)`
            // method calls (exceptions.rs). When `mgr` is a `Dyn` receiver — most
            // notably a generator gen-slot, where it can never be narrowed — those
            // calls route through gradual dispatch, so the context manager's
            // `__enter__`/`__exit__` need uniform thunks. They are not in the AST,
            // so register them here.
            out.insert("__enter__".to_string());
            out.insert("__exit__".to_string());
            for item in &w.items {
                e(&item.context_expr, out);
                if let Some(t) = &item.optional_vars {
                    e(t, out);
                }
            }
            collect_calls_body(&w.body, out);
        }
        Stmt::Match(m) => {
            e(&m.subject, out);
            for case in &m.cases {
                if let Some(g) = &case.guard {
                    e(g, out);
                }
                collect_calls_body(&case.body, out);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_calls_body(body: &[Stmt], out: &mut HashSet<String>) {
    for s in body {
        collect_calls_stmt(s, out);
    }
}

pub(super) fn collect_calls_expr(x: &Expr, out: &mut HashSet<String>) {
    match x {
        Expr::Call(c) => {
            // `x.NAME(...)` → record `NAME` (the method-call gate).
            if let Expr::Attribute(a) = c.func.as_ref() {
                out.insert(a.attr.as_str().to_string());
            }
            collect_calls_expr(&c.func, out);
            for a in &c.args {
                collect_calls_expr(a, out);
            }
            for k in &c.keywords {
                collect_calls_expr(&k.value, out);
            }
        }
        Expr::Attribute(a) => collect_calls_expr(&a.value, out),
        Expr::UnaryOp(u) => collect_calls_expr(&u.operand, out),
        Expr::BinOp(b) => {
            collect_calls_expr(&b.left, out);
            collect_calls_expr(&b.right, out);
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                collect_calls_expr(v, out);
            }
        }
        Expr::Compare(c) => {
            collect_calls_expr(&c.left, out);
            for v in &c.comparators {
                collect_calls_expr(v, out);
            }
        }
        Expr::IfExp(t) => {
            collect_calls_expr(&t.test, out);
            collect_calls_expr(&t.body, out);
            collect_calls_expr(&t.orelse, out);
        }
        Expr::Subscript(s) => {
            collect_calls_expr(&s.value, out);
            collect_calls_expr(&s.slice, out);
        }
        Expr::List(l) => {
            for v in &l.elts {
                collect_calls_expr(v, out);
            }
        }
        Expr::Tuple(t) => {
            for v in &t.elts {
                collect_calls_expr(v, out);
            }
        }
        Expr::Set(s) => {
            for v in &s.elts {
                collect_calls_expr(v, out);
            }
        }
        Expr::Dict(d) => {
            for k in d.keys.iter().flatten() {
                collect_calls_expr(k, out);
            }
            for v in &d.values {
                collect_calls_expr(v, out);
            }
        }
        Expr::Starred(s) => collect_calls_expr(&s.value, out),
        Expr::Slice(s) => {
            for part in [&s.lower, &s.upper, &s.step].into_iter().flatten() {
                collect_calls_expr(part, out);
            }
        }
        Expr::ListComp(c) => {
            collect_calls_comp(&c.generators, out);
            collect_calls_expr(&c.elt, out);
        }
        Expr::SetComp(c) => {
            collect_calls_comp(&c.generators, out);
            collect_calls_expr(&c.elt, out);
        }
        Expr::GeneratorExp(g) => {
            collect_calls_comp(&g.generators, out);
            collect_calls_expr(&g.elt, out);
        }
        Expr::DictComp(c) => {
            collect_calls_comp(&c.generators, out);
            collect_calls_expr(&c.key, out);
            collect_calls_expr(&c.value, out);
        }
        Expr::Lambda(l) => {
            for awd in l
                .args
                .posonlyargs
                .iter()
                .chain(&l.args.args)
                .chain(&l.args.kwonlyargs)
            {
                if let Some(dflt) = &awd.default {
                    collect_calls_expr(dflt, out);
                }
            }
            collect_calls_expr(&l.body, out);
        }
        Expr::Yield(y) => {
            if let Some(v) = &y.value {
                collect_calls_expr(v, out);
            }
        }
        Expr::YieldFrom(y) => collect_calls_expr(&y.value, out),
        Expr::NamedExpr(n) => {
            collect_calls_expr(&n.value, out);
            collect_calls_expr(&n.target, out);
        }
        _ => {}
    }
}

pub(super) fn collect_calls_comp(generators: &[Comprehension], out: &mut HashSet<String>) {
    for g in generators {
        collect_calls_expr(&g.iter, out);
        collect_calls_expr(&g.target, out);
        for c in &g.ifs {
            collect_calls_expr(c, out);
        }
    }
}

/// Build the per-method **uniform thunk** `M.m.<uniform>(self, __args__,
/// __kwargs__) → Value` (gradual-completeness method dispatch, Phase B) — the
/// method analogue of [`FnLowerer::uniform_thunk_over_nested`]. `self` is
/// forwarded as the method's leading positional (`pass_env = true`); the
/// non-`self` params bind from `__args__` at run time (defaults, `*args`, the
/// checked float/bool unbox) exactly as a value call, then ONE direct call is
/// made to the native method (its synthetic name resolves to its `FuncId`, so
/// an inherited override resolves correctly). Returns the thunk's `FuncId`.
///
/// The fixed/keyword-only split is read straight off the method's AST arg
/// counts (NOT re-parsed), so no parameter default is re-resolved — re-running
/// `parse_params` would duplicate a mutable default's synthetic global slot.
pub(super) fn build_method_uniform_thunk(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    args: &rustpython_parser::ast::Arguments,
    method_fid: FuncId,
) -> Result<FuncId> {
    // Non-`self` positional params, then keyword-only params. An instance method
    // always has `self` as its first positional, so the subtraction is safe.
    let n_positional = args.posonlyargs.len() + args.args.len();
    let n_fixed = n_positional.saturating_sub(1);
    let n_kwonly = args.kwonlyargs.len();
    let target = {
        let f = shared.funcs[method_fid.index()]
            .as_ref()
            .expect("method is filled before its uniform thunk is built");
        // params: [self, fixed.., kwonly.., *args?, **kwargs?] — skip self (1).
        let base = 1;
        let fixed = f.params[base..base + n_fixed]
            .iter()
            .map(ThunkParam::from_hir_param)
            .collect();
        let kwonly = f.params[base + n_fixed..base + n_fixed + n_kwonly]
            .iter()
            .map(ThunkParam::from_hir_param)
            .collect();
        UniformTarget {
            name: f.name,
            ret: f.ret_ty.clone(),
            pass_env: true,
            fixed,
            kwonly,
            varargs: f.varargs,
            kwargs: f.kwargs,
            // Method dispatch may pass a positional-or-keyword param by keyword
            // (`obj.m(a, scale=2)`), so bind fixed params from `__kwargs__` too.
            kw_bindable: true,
        }
    };
    build_uniform_thunk(interner, ctx, shared, &target)
}

/// Build the per-class **iternext thunk** `Cls.<iternext>(self: Cls) → Value`
/// (lazy user-class iterator protocol) ≡
///   `try: return self.__next__() except StopIteration: return UNBOUND`.
///
/// The runtime's `iter_next_instance` calls this to drive `for x in instance` /
/// `iter()` / `next()`: it translates a raised `StopIteration` into the
/// `Value::UNBOUND` sentinel (the runtime's `exhausted`-flag protocol). The
/// synthetic `try/except StopIteration` is real HIR, so codegen routes
/// `self.__next__()` (a PROTECTED call under `cur_handler`) through its
/// `CallConv::Tail` trampolines exactly like a `with`/`try` body — PITFALLS
/// B3/B17 hold for free (the same path the corpus already pins). `self` is
/// typed `Class{cid}` so `self.__next__()` devirtualizes precisely; inheritance
/// reuses the base's thunk (keyed by the base's `__next__` FuncId). Returns the
/// thunk's `FuncId`. Modeled on [`FnLowerer::lower_try_except`] + the `with`
/// dunder-`MethodCall` desugar.
pub(super) fn build_iternext_thunk(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    class_id: ClassId,
    class_name: InternedString,
) -> Result<FuncId> {
    let span = Span::dummy();
    let fid = shared.reserve();
    let base = interner.resolve(class_name).to_string();
    let tname = interner.intern(&format!("{base}.<iternext>"));
    let mut fl = FnLowerer::new(
        interner,
        ctx,
        shared,
        tname,
        &base,
        SemTy::Dyn,
        Some(class_id),
    );
    let self_name = fl.intern("self");
    let class_ty = SemTy::Class {
        class_id,
        name: class_name,
    };
    fl.add_param(self_name, class_ty);
    let self_lid = LocalId::new(0);

    let next_name = fl.intern("__next__");
    let try_b = fl.new_block();
    let h_test = fl.new_block();
    fl.seal(HirTerminator::Jump(try_b));

    // ── try: result = self.__next__(); return result ── (protected call) ──
    fl.switch(try_b);
    let outer = fl.cur_handler; // None
    fl.cur_handler = Some(h_test);
    let self_ref = fl.local_ref(self_lid, span);
    let call = fl.alloc(
        HirExprKind::MethodCall {
            recv: self_ref,
            method_name: next_name,
            args: vec![],
            kwargs: vec![],
        },
        SemTy::Dyn,
        span,
    );
    let result = fl.fresh_local(SemTy::Dyn);
    fl.push_stmt(HirStmt::Assign {
        target: result,
        value: call,
    });
    // Leave the protected region (the return runs under the OUTER handler), then
    // return the `__next__()` result — the boxed Tagged value the runtime reads.
    fl.exit_protected(outer);
    let rref = fl.local_ref(result, span);
    fl.seal(HirTerminator::Return(Some(rref)));
    fl.cur_handler = outer;

    // ── handler chain (under the OUTER handler): StopIteration → UNBOUND ──
    fl.switch(h_test);
    let tag = pyaot_core_defs::BuiltinExceptionKind::StopIteration.tag();
    let q = fl.alloc(
        HirExprKind::ExcQuery(ExcQuery::MatchesBuiltin(tag)),
        SemTy::Bool,
        span,
    );
    let body_b = fl.new_block();
    let nomatch_b = fl.new_block();
    fl.seal(HirTerminator::Branch {
        cond: q,
        then: body_b,
        else_: nomatch_b,
    });

    // matched StopIteration: park + clear the exception, return the sentinel.
    fl.switch(body_b);
    fl.push_stmt(HirStmt::ExcOp(ExcOp::StartHandling));
    fl.push_stmt(HirStmt::ExcOp(ExcOp::EndHandling));
    // `Value::UNBOUND` (a distinct tagged immediate, GC-ignored); only
    // `iter_next_instance` reads it via `is_unbound()`. Typed `Dyn` (NOT `Never`)
    // so the thunk's return join stays clean.
    let unbound = fl.alloc(HirExprKind::Unbound, SemTy::Dyn, span);
    fl.seal(HirTerminator::Return(Some(unbound)));

    // any other exception: propagate outward unchanged.
    fl.switch(nomatch_b);
    fl.push_stmt(HirStmt::Raise(HirRaise::Reraise));
    fl.seal(HirTerminator::Unreachable);

    let f = fl.finish(HirTerminator::Return(None));
    shared.fill(fid, f);
    Ok(fid)
}

/// Build the per-class **copy thunk** `Cls.<dunder>(self: Cls) → Value` for
/// `__copy__` / `__deepcopy__`, consulted by `copy.copy` / `copy.deepcopy`:
///   `__copy__`     ≡ `return self.__copy__()`
///   `__deepcopy__` ≡ `return self.__deepcopy__({})`  (fresh memo dict)
///
/// The runtime (`rt_copy_copy` / `rt_copy_deepcopy`) calls the registered thunk
/// as `extern "C" fn(i64 /*self*/) -> *mut Obj` with NO memo argument, so the
/// `__deepcopy__` thunk supplies the memo itself. `self` is typed `Class{cid}`
/// so `self.<dunder>()` devirtualizes precisely; inheritance reuses the base's
/// thunk (keyed by the base's dunder FuncId in `build_mir_classes`). Modeled on
/// [`build_iternext_thunk`], minus the `try/except` (no `StopIteration` here).
///
/// NOTE (memo caveat): the `__deepcopy__` memo is a fresh empty dict, NOT the
/// runtime's cycle tracker. A user `__deepcopy__` that relies on `memo` to break
/// reference cycles across nested `copy.deepcopy()` calls is unsupported; the
/// common "rebuild from deep-copied fields" pattern works.
pub(super) fn build_copy_dunder_thunk(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    class_id: ClassId,
    class_name: InternedString,
    dunder: &str,
    pass_memo: bool,
) -> Result<FuncId> {
    let span = Span::dummy();
    let fid = shared.reserve();
    let base = interner.resolve(class_name).to_string();
    let tname = interner.intern(&format!("{base}.<{dunder}>"));
    let mut fl = FnLowerer::new(
        interner,
        ctx,
        shared,
        tname,
        &base,
        SemTy::Dyn,
        Some(class_id),
    );
    let self_name = fl.intern("self");
    let class_ty = SemTy::Class {
        class_id,
        name: class_name,
    };
    fl.add_param(self_name, class_ty);
    let self_lid = LocalId::new(0);

    let dunder_name = fl.intern(dunder);
    let body_b = fl.new_block();
    fl.seal(HirTerminator::Jump(body_b));
    fl.switch(body_b);

    let self_ref = fl.local_ref(self_lid, span);
    // `__deepcopy__(self, memo)` needs a memo dict; `__copy__(self)` takes none.
    let args = if pass_memo {
        vec![fl.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span)]
    } else {
        vec![]
    };
    let call = fl.alloc(
        HirExprKind::MethodCall {
            recv: self_ref,
            method_name: dunder_name,
            args,
            kwargs: vec![],
        },
        SemTy::Dyn,
        span,
    );
    let result = fl.fresh_local(SemTy::Dyn);
    fl.push_stmt(HirStmt::Assign {
        target: result,
        value: call,
    });
    let rref = fl.local_ref(result, span);
    fl.seal(HirTerminator::Return(Some(rref)));

    let f = fl.finish(HirTerminator::Return(None));
    shared.fill(fid, f);
    Ok(fid)
}

pub(super) fn lower_class(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    cdef: &StmtClassDef,
    class_id: ClassId,
    shared: &mut Shared,
) -> Result<HirClass> {
    let span = to_span(cdef.range());
    // Class decorators (§5) do NOT change the class definition itself — they are
    // applied at module-init (`emit_class_decorators`) over the class-id int, for
    // their side effects; the class name stays bound to its class id via the
    // static `class_map`. So lowering the class body here ignores them.
    if !cdef.keywords.is_empty() {
        return Err(parse_error(
            "class keyword arguments (e.g. `metaclass=`) are out of scope",
            span,
        ));
    }

    // ── Type parameters (Phase 5E): PEP 695 `class C[T]` + `Generic[T]` base ──
    let mut type_params: Vec<InternedString> = Vec::new();
    let mut type_param_names: Vec<String> = Vec::new();
    for tp in &cdef.type_params {
        let name = type_param_name(tp);
        type_param_names.push(name.clone());
        type_params.push(interner.intern(&name));
    }

    // Base classes: bare names (`class Dog(Animal)`); `Generic[T]` / `Protocol[T]`
    // contribute type params (not a runtime base). A bare `Protocol` / `Generic`
    // base is a typing marker — not a runtime base — so it is NOT pushed to
    // `base_names` (which would make `semantics` reject it as "unknown base
    // class"); `Protocol` additionally flags the class as a structural protocol.
    let mut base_names = Vec::new();
    let mut is_protocol = false;
    for base in &cdef.bases {
        match base {
            Expr::Name(n) if matches!(n.id.as_str(), "Protocol" | "Generic") => {
                if n.id.as_str() == "Protocol" {
                    is_protocol = true;
                }
            }
            Expr::Name(n) => base_names.push(interner.intern(n.id.as_str())),
            // `Generic[T]` / `Generic[T1, T2]` → record the type params.
            Expr::Subscript(s) => {
                let Expr::Name(b) = s.value.as_ref() else {
                    return Err(parse_error(
                        "unsupported subscripted base class",
                        to_span(base.range()),
                    ));
                };
                if matches!(b.id.as_str(), "Generic" | "Protocol") {
                    if b.id.as_str() == "Protocol" {
                        is_protocol = true;
                    }
                    for tp in subscript_type_param_names(s.slice.as_ref()) {
                        if !type_param_names.contains(&tp) {
                            type_params.push(interner.intern(&tp));
                            type_param_names.push(tp);
                        }
                    }
                } else {
                    return Err(parse_error(
                        "subscripted base classes other than Generic/Protocol are out of scope",
                        to_span(base.range()),
                    ));
                }
            }
            _ => {
                return Err(parse_error(
                    "unsupported base-class expression",
                    to_span(base.range()),
                ))
            }
        }
    }

    // Per-class annotation context: module type vars + this class's params.
    let mut merged_tv: TypeVarSet = ctx.type_vars.clone();
    for (n, id) in type_param_names.iter().zip(&type_params) {
        merged_tv.insert(n.clone(), *id);
    }
    let cctx = AnnCtx {
        class_map: ctx.class_map,
        type_vars: &merged_tv,
        top_defs: ctx.top_defs,
        promoted: ctx.promoted,
        decorated: ctx.decorated,
        aliases: ctx.aliases,
        alias_vars: ctx.alias_vars,
        stdlib: ctx.stdlib,
        // Methods resolve a non-literal default (`__init__(self, x=[])`, §6) to
        // its synthetic global slot via the inherited map (def-time once-eval,
        // CPython shared-mutable-default aliasing); a literal default still folds
        // to a `Const`. A nested def inside a method re-clones with `None` (a
        // process-global slot can't hold a per-closure default).
        default_slots: ctx.default_slots,
        type_aliases: ctx.type_aliases,
        proto_ids: ctx.proto_ids,
    };

    let name = interner.intern(cdef.name.as_str());
    // CPython renders a top-level class instance as `<__main__.Cls object at …>`.
    let qualname = interner.intern(&format!("__main__.{}", cdef.name.as_str()));
    let class_ty = SemTy::Class { class_id, name };
    let mut methods: Vec<(InternedString, FuncId)> = Vec::new();
    let mut static_methods: Vec<(InternedString, FuncId)> = Vec::new();
    let mut class_methods: Vec<(InternedString, FuncId)> = Vec::new();
    let mut properties: Vec<HirProperty> = Vec::new();
    let mut class_attrs: Vec<HirClassAttr> = Vec::new();
    let mut field_annotations: Vec<(InternedString, SemTy)> = Vec::new();

    // Lower a method body into the shared table, returning its FuncId.
    let lower_method = |interner: &mut StringInterner,
                        shared: &mut Shared,
                        m: &StmtFunctionDef,
                        suffix: &str,
                        first: FirstParam,
                        enclosing: Option<ClassId>|
     -> Result<(FuncId, SemTy)> {
        let synthetic = format!("{}.{}{}", cdef.name.as_str(), m.name.as_str(), suffix);
        let fname = interner.intern(&synthetic);
        let func_id = lower_callable(
            interner, &cctx, shared, m, &synthetic, fname, first, enclosing, true, None,
        )?;
        let ret = shared.funcs[func_id.index()]
            .as_ref()
            .expect("method just filled")
            .ret_ty
            .clone();
        Ok((func_id, ret))
    };

    for stmt in &cdef.body {
        match stmt {
            Stmt::FunctionDef(m) => {
                let method_name = interner.intern(m.name.as_str());
                // `__new__` (§3): the allocator hook. Lowered like a
                // `@staticmethod` (`FirstParam::Plain`) — its first param `cls`
                // is a real int (the class-id, per the `def __new__(cls: int)`
                // convention), NOT a `self` instance. Stored in `static_methods`
                // so `lower_construct` finds it and calls it (passing the
                // class-id int) instead of emitting `MakeInstance`. The runtime
                // allocation happens inside via `object.__new__(cls)`.
                if m.name.as_str() == "__new__" && m.decorator_list.is_empty() {
                    let (fid, _) = lower_method(interner, shared, m, "", FirstParam::Plain, None)?;
                    static_methods.push((method_name, fid));
                    continue;
                }
                match classify_method_decorator(m)? {
                    MethodDecor::Instance => {
                        let (fid, _) = lower_method(
                            interner,
                            shared,
                            m,
                            "",
                            FirstParam::Method(class_ty.clone()),
                            Some(class_id),
                        )?;
                        methods.push((method_name, fid));
                        // Gradual completeness (Phase B): if this method's name is
                        // ever invoked as a method call, build its uniform thunk so
                        // `rt_obj_method` can dispatch it on a `Dyn` receiver. Keyed
                        // by the method's own FuncId — an inherited `ClassInfo.methods`
                        // entry reuses it, so the subclass registers this same thunk.
                        if shared.dyn_method_names.contains(&method_name) {
                            let thunk =
                                build_method_uniform_thunk(interner, &cctx, shared, &m.args, fid)?;
                            shared.method_uniform_thunks.insert(fid, thunk);
                        }
                        // Lazy user-class iterator protocol: a class with an own
                        // `__next__` gets an `<iternext>` thunk so the runtime can
                        // drive `for x in inst` / `iter()` / `next()`. Keyed by
                        // `__next__`'s own FuncId — an inherited `__next__`
                        // (ClassInfo entry reuses the base's FuncId) resolves the
                        // base's thunk in `build_mir_classes`.
                        if m.name.as_str() == "__next__" {
                            let thunk =
                                build_iternext_thunk(interner, &cctx, shared, class_id, name)?;
                            shared.iternext_thunks.insert(fid, thunk);
                        }
                        // `__copy__` / `__deepcopy__`: build a thunk so the
                        // runtime's `copy.copy` / `copy.deepcopy` dispatch to the
                        // user method (registered into COPY/DEEPCOPY_FUNC_REGISTRY
                        // in `__pyaot_classinit`). Keyed by the dunder's own
                        // FuncId — an inherited dunder reuses the base's thunk.
                        if m.name.as_str() == "__copy__" {
                            let thunk = build_copy_dunder_thunk(
                                interner, &cctx, shared, class_id, name, "__copy__", false,
                            )?;
                            shared.copy_thunks.insert(fid, thunk);
                        } else if m.name.as_str() == "__deepcopy__" {
                            let thunk = build_copy_dunder_thunk(
                                interner, &cctx, shared, class_id, name, "__deepcopy__", true,
                            )?;
                            shared.copy_thunks.insert(fid, thunk);
                        }
                    }
                    MethodDecor::Static => {
                        let (fid, _) =
                            lower_method(interner, shared, m, "", FirstParam::Plain, None)?;
                        static_methods.push((method_name, fid));
                    }
                    MethodDecor::Class => {
                        // Bind `cls` as a compile-time alias of this class (a
                        // classmethod body resolves `cls` / `cls(...)` against it).
                        // `enclosing` stays `None` — a classmethod has no `self`,
                        // so `super()` must keep cleanly rejecting.
                        let (fid, _) = lower_method(
                            interner,
                            shared,
                            m,
                            "",
                            FirstParam::ClsMethod { class_id, name },
                            None,
                        )?;
                        class_methods.push((method_name, fid));
                    }
                    MethodDecor::Property => {
                        let (fid, ty) = lower_method(
                            interner,
                            shared,
                            m,
                            ".get",
                            FirstParam::Method(class_ty.clone()),
                            Some(class_id),
                        )?;
                        properties.push(HirProperty {
                            name: method_name,
                            getter: fid,
                            setter: None,
                            ty,
                        });
                    }
                    MethodDecor::Setter(prop) => {
                        let pname = interner.intern(&prop);
                        let (fid, _) = lower_method(
                            interner,
                            shared,
                            m,
                            ".set",
                            FirstParam::Method(class_ty.clone()),
                            Some(class_id),
                        )?;
                        match properties.iter_mut().find(|p| p.name == pname) {
                            Some(p) => p.setter = Some(fid),
                            None => {
                                return Err(parse_error(
                                    format!("@{prop}.setter has no matching @property"),
                                    to_span(m.range()),
                                ))
                            }
                        }
                    }
                }
            }
            // `name: T = value` (annotated, *with* a value) is a class attribute;
            // `name: T` (no value) is an instance-field type hint.
            Stmt::AnnAssign(a) => {
                let Expr::Name(n) = a.target.as_ref() else {
                    return Err(parse_error(
                        "class-level annotated target must be a name",
                        to_span(a.range()),
                    ));
                };
                let fname = interner.intern(n.id.as_str());
                let ty = annotation_to_semty(a.annotation.as_ref(), &cctx);
                match &a.value {
                    Some(v) => {
                        let init = class_attr_init(interner, v.as_ref())?;
                        reject_tuple_class_attr(&init, to_span(a.range()))?;
                        class_attrs.push(HirClassAttr {
                            name: fname,
                            ty,
                            init,
                        });
                    }
                    None => field_annotations.push((fname, ty)),
                }
            }
            // Class-level `name = value` value assignment → a class attribute.
            Stmt::Assign(a) => {
                if a.targets.len() != 1 {
                    return Err(parse_error(
                        "chained class-attribute assignment is not supported",
                        to_span(a.range()),
                    ));
                }
                let Expr::Name(n) = &a.targets[0] else {
                    return Err(parse_error(
                        "class-level assignment target must be a name",
                        to_span(a.range()),
                    ));
                };
                // `__slots__` is a CPython memory optimization with no observable
                // semantics in our uniform-tagged object model — silently ignore
                // it (Phase 8E).
                if n.id.as_str() == "__slots__" {
                    continue;
                }
                let fname = interner.intern(n.id.as_str());
                let init = class_attr_init(interner, a.value.as_ref())?;
                reject_tuple_class_attr(&init, to_span(a.range()))?;
                let ty = class_attr_init_ty(&init);
                class_attrs.push(HirClassAttr {
                    name: fname,
                    ty,
                    init,
                });
            }
            // A docstring (a bare string-constant expression) is ignored.
            Stmt::Expr(e) if matches!(e.value.as_ref(), Expr::Constant(c) if matches!(c.value, Constant::Str(_))) =>
                {}
            // A bare `...` (Ellipsis) class body — a Protocol's `class P(Protocol):
            // ...` stub — is a no-op like `pass`.
            Stmt::Expr(e) if matches!(e.value.as_ref(), Expr::Constant(c) if matches!(c.value, Constant::Ellipsis)) =>
                {}
            Stmt::Pass(_) => {}
            other => {
                return Err(parse_error(
                    "unsupported statement in class body",
                    to_span(other.range()),
                ))
            }
        }
    }

    // Honor in-method instance-field annotations as field-type contracts: a
    // `self.<name>: T = v` (or a bare `self.<name>: T`) inside any instance method
    // declares the field's type exactly like a class-level `name: T`. CPython does
    // not record these in `__annotations__`, but this compiler treats a typed slot
    // annotation as a contract (e.g. the §8 numeric-tower int→float field store
    // needs the field type known before `typeck`/lowering). Class-level
    // annotations were collected above and WIN on a name clash; among methods the
    // first occurrence wins (`scan_self_field_annotations` skips already-present
    // names). Only `self`-receiver methods are scanned — a `@staticmethod`/
    // `classmethod` (`cls`/other first param, incl. `__new__`) has no instance
    // fields — mirroring `discover_fields`'s self-write scan.
    for stmt in &cdef.body {
        let Stmt::FunctionDef(m) = stmt else { continue };
        let first_param = m
            .args
            .posonlyargs
            .first()
            .or_else(|| m.args.args.first())
            .map(|awd| awd.def.arg.as_str());
        if first_param != Some("self") {
            continue;
        }
        scan_self_field_annotations(&m.body, &cctx, interner, &mut field_annotations);
    }

    Ok(HirClass {
        name,
        qualname,
        class_id,
        base_names,
        methods,
        static_methods,
        class_methods,
        properties,
        class_attrs,
        field_annotations,
        type_params,
        is_protocol,
    })
}

/// Collect `self.<name>: T` field-type annotations from a method body into
/// `out` (in-method field declarations). Recurses into nested
/// blocks (a field may be annotated inside an `if`/`for`/`with`/`try`), since the
/// annotation is a declaration regardless of control flow. A name already present
/// (a class-level annotation, or an earlier method's) is left untouched, so
/// class-level declarations stay authoritative and the first method wins. `Dyn`
/// annotations are skipped (they add no contract — the field-discovery scan in
/// `semantics` would infer the same or better from the writes).
pub(super) fn scan_self_field_annotations(
    body: &[Stmt],
    cctx: &AnnCtx,
    interner: &mut StringInterner,
    out: &mut Vec<(InternedString, SemTy)>,
) {
    for stmt in body {
        match stmt {
            Stmt::AnnAssign(a) => {
                // `self.<name>: T` — a single-dot attribute on the `self` receiver.
                let Expr::Attribute(attr) = a.target.as_ref() else {
                    continue;
                };
                let Expr::Name(recv) = attr.value.as_ref() else {
                    continue;
                };
                if recv.id.as_str() != "self" {
                    continue;
                }
                let ty = annotation_to_semty(a.annotation.as_ref(), cctx);
                if ty == SemTy::Dyn {
                    continue;
                }
                let fname = interner.intern(attr.attr.as_str());
                if !out.iter().any(|(n, _)| *n == fname) {
                    out.push((fname, ty));
                }
            }
            // Recurse into nested blocks (a `self.x: T` may be guarded).
            Stmt::If(s) => {
                scan_self_field_annotations(&s.body, cctx, interner, out);
                scan_self_field_annotations(&s.orelse, cctx, interner, out);
            }
            Stmt::For(s) => {
                scan_self_field_annotations(&s.body, cctx, interner, out);
                scan_self_field_annotations(&s.orelse, cctx, interner, out);
            }
            Stmt::While(s) => {
                scan_self_field_annotations(&s.body, cctx, interner, out);
                scan_self_field_annotations(&s.orelse, cctx, interner, out);
            }
            Stmt::With(s) => scan_self_field_annotations(&s.body, cctx, interner, out),
            Stmt::Try(s) => {
                scan_self_field_annotations(&s.body, cctx, interner, out);
                for h in &s.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = h;
                    scan_self_field_annotations(&h.body, cctx, interner, out);
                }
                scan_self_field_annotations(&s.orelse, cctx, interner, out);
                scan_self_field_annotations(&s.finalbody, cctx, interner, out);
            }
            _ => {}
        }
    }
}

/// The name of a PEP 695 type parameter (`T`, `*Ts`, `**P`). Only the simple
/// `TypeVar` form is meaningful for our erase-to-Tagged model.
pub(super) fn type_param_name(tp: &rustpython_parser::ast::TypeParam) -> String {
    use rustpython_parser::ast::TypeParam;
    match tp {
        TypeParam::TypeVar(t) => t.name.as_str().to_string(),
        TypeParam::ParamSpec(t) => t.name.as_str().to_string(),
        TypeParam::TypeVarTuple(t) => t.name.as_str().to_string(),
    }
}

/// True iff a class declares a `Protocol` base — bare (`class P(Protocol)`) or
/// subscripted (`class P(Protocol[T])`) — i.e. a structural-typing protocol.
/// Mirrors the `is_protocol` detection in `lower_class`'s base loop,
/// but runs in the pre-lowering class scan (to seed `proto_ids`).
pub(super) fn class_def_is_protocol(cdef: &StmtClassDef) -> bool {
    cdef.bases.iter().any(|base| match base {
        Expr::Name(n) => n.id.as_str() == "Protocol",
        Expr::Subscript(s) => {
            matches!(s.value.as_ref(), Expr::Name(b) if b.id.as_str() == "Protocol")
        }
        _ => false,
    })
}

/// The type-parameter names in a `Generic[...]` subscript slice.
pub(super) fn subscript_type_param_names(slice: &Expr) -> Vec<String> {
    match slice {
        Expr::Name(n) => vec![n.id.as_str().to_string()],
        Expr::Tuple(t) => t
            .elts
            .iter()
            .filter_map(|e| match e {
                Expr::Name(n) => Some(n.id.as_str().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}


/// The synthetic lowered-name suffix for a method, mirroring `lower_class`'s
/// `lower_method` calls: a `@property` getter is `.get`, an `@x.setter` is
/// `.set`, everything else (instance / `@staticmethod` / `@classmethod` /
/// `__new__`) has no suffix. Used to key mutable-default slots (§6) to the same
/// synthetic name `resolve_param_default` sees.
pub(super) fn method_synthetic_suffix(m: &StmtFunctionDef) -> &'static str {
    match classify_method_decorator(m) {
        Ok(MethodDecor::Property) => ".get",
        Ok(MethodDecor::Setter(_)) => ".set",
        _ => "",
    }
}

/// Classify a method's (at most one) decorator. Bare instance methods carry none.
pub(super) fn classify_method_decorator(m: &StmtFunctionDef) -> Result<MethodDecor> {
    let span = to_span(m.range());
    match m.decorator_list.as_slice() {
        [] => Ok(MethodDecor::Instance),
        [deco] => match deco {
            Expr::Name(n) => match n.id.as_str() {
                "staticmethod" => Ok(MethodDecor::Static),
                "classmethod" => Ok(MethodDecor::Class),
                "property" => Ok(MethodDecor::Property),
                // `@abstractmethod` is a runtime no-op here: the file only ever
                // instantiates *concrete* subclasses, and with no `ABCMeta`
                // metaclass (out of scope) even CPython permits instantiation —
                // so lowering an `@abstractmethod` method exactly as an ordinary
                // instance method is byte-exact. (Stacked
                // `@classmethod`+`@abstractmethod` stays rejected below — a
                // future strip-then-classify follow-up.)
                "abstractmethod" => Ok(MethodDecor::Instance),
                other => Err(parse_error(
                    format!("unsupported decorator @{other} (general decorators are Phase 6)"),
                    span,
                )),
            },
            // `@x.setter` → Attribute{value: Name("x"), attr: "setter"}.
            Expr::Attribute(a) if a.attr.as_str() == "setter" => match a.value.as_ref() {
                Expr::Name(n) => Ok(MethodDecor::Setter(n.id.as_str().to_string())),
                _ => Err(parse_error("malformed @x.setter decorator", span)),
            },
            _ => Err(parse_error(
                "unsupported decorator (general decorators are Phase 6)",
                span,
            )),
        },
        _ => Err(parse_error("stacked decorators are out of scope", span)),
    }
}

/// The single literal accept-set, shared by class attributes (`class_attr_init`)
/// and parameter defaults (the allocation pass + `resolve_param_default`).
/// `Some(init)` = a recognized constant literal; `None` = a valid but
/// non-literal expression (e.g. `[]`, `5 + 5` — a mutable/computed default
/// candidate). Every literal kind here is accepted (`bytes` literals intern as
/// raw byte blobs, so even non-UTF-8 `b"\xff"` is a recognized literal).
pub(super) fn try_literal_default(interner: &mut StringInterner, value: &Expr) -> Option<ClassAttrInit> {
    // Fold a unary +/- over a numeric literal first.
    if let Expr::UnaryOp(u) = value {
        if matches!(u.op, PyUnaryOp::USub | PyUnaryOp::UAdd) {
            if let Expr::Constant(c) = u.operand.as_ref() {
                let neg = matches!(u.op, PyUnaryOp::USub);
                return match &c.value {
                    Constant::Int(b) => Some(int_attr_init(interner, &b.to_string(), neg)),
                    Constant::Float(f) => Some(ClassAttrInit::Float(if neg { -*f } else { *f })),
                    // `-x`, `+obj`, … → non-literal (computed-default candidate).
                    _ => None,
                };
            }
            // `-(expr)` → non-literal.
            return None;
        }
    }
    match value {
        Expr::Constant(c) => match &c.value {
            Constant::Int(b) => Some(int_attr_init(interner, &b.to_string(), false)),
            Constant::Float(f) => Some(ClassAttrInit::Float(*f)),
            Constant::Bool(b) => Some(ClassAttrInit::Bool(*b)),
            Constant::Str(s) => Some(ClassAttrInit::Str(interner.intern(s))),
            Constant::None => Some(ClassAttrInit::None),
            // Raw bytes (the interner stores byte blobs), so a non-UTF-8 class
            // attribute default `b"\xff"` round-trips intact.
            Constant::Bytes(b) => Some(ClassAttrInit::Bytes(interner.intern_bytes(b))),
            // Complex / ellipsis / tuple constant → non-literal here.
            _ => None,
        },
        // The empty tuple `()` — accepted only as a parameter default (Phase 8E,
        // e.g. `children=()`); materialized as a fresh empty tuple at each call
        // site. A non-empty tuple default stays out of scope.
        Expr::Tuple(t) if t.elts.is_empty() => Some(ClassAttrInit::EmptyTuple),
        // Any other expression is non-literal (a slot candidate for a top-level
        // parameter default; rejected as a class-attribute initializer).
        _ => None,
    }
}

/// Lower a class-attribute initializer; only constant literals are supported (5D).
pub(super) fn class_attr_init(interner: &mut StringInterner, value: &Expr) -> Result<ClassAttrInit> {
    let span = to_span(value.range());
    match try_literal_default(interner, value) {
        Some(init) => Ok(init),
        None => Err(parse_error(
            "class-attribute initializers must be constant literals (Phase 5D)",
            span,
        )),
    }
}

/// Resolve a parameter default expression to a [`ParamDefault`]. A literal
/// becomes a per-call-materialized `Const`; a non-literal (mutable/computed)
/// default of a **top-level** function (where `ctx.default_slots` is `Some` and
/// holds a slot for `(fname, pname)`) becomes a once-evaluated global `Slot`.
/// Everywhere else (nested defs, methods, decorated defs, generators) a
/// non-literal default is a clean parse error.
pub(super) fn resolve_param_default(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    fname: InternedString,
    pname: InternedString,
    value: &Expr,
) -> Result<ParamDefault> {
    let span = to_span(value.range());
    match try_literal_default(interner, value) {
        Some(init) => Ok(ParamDefault::Const(init)),
        None => {
            if let Some(slots) = ctx.default_slots {
                if let Some(&var_id) = slots.get(&(fname, pname)) {
                    return Ok(ParamDefault::Slot(var_id));
                }
            }
            Err(parse_error(
                "a mutable/computed default is only supported on a top-level \
                 function parameter; otherwise defaults must be constant literals",
                span,
            ))
        }
    }
}

/// Collect the simple `Name` identifiers bound by a `for`-target (Phase 8E),
/// descending into tuple / list unpacking. Used to shadow comprehension loop
/// variables so they do not leak into the enclosing scope.
/// Count how many times `name` is bound in one SCOPE's statement list (Phase
/// 8H, #9). Descends into control-flow bodies (same scope) but NOT into
/// `def`/`class` (new scopes). A `global`/`nonlocal` declaration disqualifies
/// (returns 2+) — the name is not a plain single-bound local.
pub(super) fn count_scope_bindings(stmts: &[Stmt], name: &str) -> usize {
    let mut count = 0usize;
    for stmt in stmts {
        match stmt {
            Stmt::Assign(a) => {
                for t in &a.targets {
                    let mut names = Vec::new();
                    collect_target_names(t, &mut names);
                    count += names.iter().filter(|n| **n == name).count();
                }
            }
            Stmt::AugAssign(a) => {
                let mut names = Vec::new();
                collect_target_names(a.target.as_ref(), &mut names);
                count += names.iter().filter(|n| **n == name).count();
            }
            Stmt::AnnAssign(a) => {
                let mut names = Vec::new();
                collect_target_names(a.target.as_ref(), &mut names);
                count += names.iter().filter(|n| **n == name).count();
            }
            Stmt::For(f) => {
                let mut names = Vec::new();
                collect_target_names(f.target.as_ref(), &mut names);
                count += names.iter().filter(|n| **n == name).count();
                count += count_scope_bindings(&f.body, name);
                count += count_scope_bindings(&f.orelse, name);
            }
            Stmt::While(w) => {
                count += count_scope_bindings(&w.body, name);
                count += count_scope_bindings(&w.orelse, name);
            }
            Stmt::If(i) => {
                count += count_scope_bindings(&i.body, name);
                count += count_scope_bindings(&i.orelse, name);
            }
            Stmt::With(w) => {
                for item in &w.items {
                    if let Some(v) = &item.optional_vars {
                        let mut names = Vec::new();
                        collect_target_names(v.as_ref(), &mut names);
                        count += names.iter().filter(|n| **n == name).count();
                    }
                }
                count += count_scope_bindings(&w.body, name);
            }
            Stmt::Try(t) => {
                count += count_scope_bindings(&t.body, name);
                for h in &t.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = h;
                    if h.name.as_ref().is_some_and(|n| n.as_str() == name) {
                        count += 1;
                    }
                    count += count_scope_bindings(&h.body, name);
                }
                count += count_scope_bindings(&t.orelse, name);
                count += count_scope_bindings(&t.finalbody, name);
            }
            Stmt::FunctionDef(d) => {
                if d.name.as_str() == name {
                    count += 1;
                }
            }
            Stmt::ClassDef(c) => {
                if c.name.as_str() == name {
                    count += 1;
                }
            }
            Stmt::Import(im) => {
                for a in &im.names {
                    let bound = a.asname.as_ref().unwrap_or(&a.name);
                    if bound.as_str() == name {
                        count += 1;
                    }
                }
            }
            Stmt::ImportFrom(im) => {
                for a in &im.names {
                    let bound = a.asname.as_ref().unwrap_or(&a.name);
                    if bound.as_str() == name {
                        count += 1;
                    }
                }
            }
            Stmt::Global(g) => {
                if g.names.iter().any(|n| n.as_str() == name) {
                    return 2; // disqualify outright
                }
            }
            Stmt::Nonlocal(g) => {
                if g.names.iter().any(|n| n.as_str() == name) {
                    return 2; // disqualify outright
                }
            }
            _ => {}
        }
    }
    count
}

/// Reject the empty-tuple initializer as a *class attribute* (it is only valid
/// as a parameter default, where it materializes as a fresh `TupleLit`). A class
/// attribute lowers to a MIR `Const`, which has no empty-tuple form.
pub(super) fn reject_tuple_class_attr(init: &ClassAttrInit, span: Span) -> Result<()> {
    if matches!(init, ClassAttrInit::EmptyTuple) {
        return Err(parse_error(
            "a tuple `()` class attribute is out of scope (only valid as a parameter default)",
            span,
        ));
    }
    Ok(())
}

/// Build an int/bignum class-attr initializer from decimal text + sign.
pub(super) fn int_attr_init(interner: &mut StringInterner, decimal: &str, negative: bool) -> ClassAttrInit {
    match decimal.parse::<i64>() {
        Ok(mag) if pyaot_core_defs::int_fits(if negative { -mag } else { mag }) => {
            ClassAttrInit::Int(if negative { -mag } else { mag })
        }
        _ => {
            let text = if negative {
                format!("-{decimal}")
            } else {
                decimal.to_string()
            };
            ClassAttrInit::BigInt(interner.intern(&text))
        }
    }
}

/// The best-effort `SemTy` of a class-attribute initializer.
pub(super) fn class_attr_init_ty(init: &ClassAttrInit) -> SemTy {
    match init {
        ClassAttrInit::Int(_) | ClassAttrInit::BigInt(_) => SemTy::Int,
        ClassAttrInit::Float(_) => SemTy::Float,
        ClassAttrInit::Bool(_) => SemTy::Bool,
        ClassAttrInit::Str(_) => SemTy::Str,
        ClassAttrInit::Bytes(_) => SemTy::Bytes,
        ClassAttrInit::None => SemTy::NoneTy,
        ClassAttrInit::EmptyTuple => SemTy::Dyn,
    }
}

