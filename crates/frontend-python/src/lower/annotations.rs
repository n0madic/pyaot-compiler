use super::*;

/// If `stmt` is `Name = TypeVar(...)` (or `ParamSpec`/`TypeVarTuple`), return the
/// target name — a module-level type variable (Phase 5E).
pub(super) fn type_var_assign_name(stmt: &Stmt) -> Option<String> {
    let Stmt::Assign(a) = stmt else { return None };
    if a.targets.len() != 1 {
        return None;
    }
    let Expr::Name(target) = &a.targets[0] else {
        return None;
    };
    let Expr::Call(call) = a.value.as_ref() else {
        return None;
    };
    let Expr::Name(f) = call.func.as_ref() else {
        return None;
    };
    matches!(f.id.as_str(), "TypeVar" | "ParamSpec" | "TypeVarTuple")
        .then(|| target.id.as_str().to_string())
}

/// The `SemTy` type arguments in a `Cls[args]` subscript slice (Phase 5E).
pub(super) fn subscript_type_args(slice: &Expr, ctx: &AnnCtx) -> Vec<SemTy> {
    match slice {
        Expr::Tuple(t) => t.elts.iter().map(|e| annotation_to_semty(e, ctx)).collect(),
        single => vec![annotation_to_semty(single, ctx)],
    }
}


pub(super) fn binop_from_ast(op: &PyOperator) -> BinOp {
    match op {
        PyOperator::Add => BinOp::Add,
        PyOperator::Sub => BinOp::Sub,
        PyOperator::Mult => BinOp::Mul,
        PyOperator::Div => BinOp::Div,
        PyOperator::FloorDiv => BinOp::FloorDiv,
        PyOperator::Mod => BinOp::Mod,
        PyOperator::Pow => BinOp::Pow,
        PyOperator::LShift => BinOp::Shl,
        PyOperator::RShift => BinOp::Shr,
        PyOperator::BitOr => BinOp::BitOr,
        PyOperator::BitXor => BinOp::BitXor,
        PyOperator::BitAnd => BinOp::BitAnd,
        // `a @ b` (PEP 465): no built-in numeric `@`, so it dispatches the
        // `__matmul__`/`__rmatmul__` dunder at runtime (like `+`/`*`).
        PyOperator::MatMult => BinOp::MatMul,
    }
}

/// Map a type annotation to a `SemTy` (primitives and built-in containers drive
/// `Repr`; everything else is `Dyn`). A bare container name (`list`) defaults its
/// element types to `Dyn`; a subscripted one (`list[int]`, `dict[str, int]`,
/// `tuple[int, ...]`) carries them — this is what lets the empty-literal bootstrap
/// seed `x: list[int] = []` (PITFALLS B4).
pub(super) fn annotation_to_semty(ann: &Expr, ctx: &AnnCtx) -> SemTy {
    match ann {
        Expr::Name(n) => named_annotation(n.id.as_str(), ctx),
        // A qualified class annotation through an `import M` alias (Phase 8):
        // `math_utils.Point` resolves via the `"M.Cls"` key in `class_map`;
        // a stdlib class (`time.struct_time`) via the stdlib bindings (8B).
        Expr::Attribute(a) => {
            if let Expr::Name(m) = a.value.as_ref() {
                if ctx.aliases.contains(m.id.as_str()) {
                    let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                    if let Some((class_id, name)) = ctx.class_map.get(&qual) {
                        return SemTy::Class {
                            class_id: *class_id,
                            name: *name,
                        };
                    }
                }
                if ctx.stdlib.aliases.contains(m.id.as_str()) {
                    let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                    if let Some(ty) = ctx.stdlib.classes.get(&qual) {
                        return ty.clone();
                    }
                    // `import collections; x: collections.Counter` — the qualified
                    // construction function names the runtime-object type.
                    if let Some(def) = ctx.stdlib.funcs.get(&qual) {
                        if is_counter_def(def) {
                            return SemTy::RuntimeObject(pyaot_core_defs::TypeTagKind::Counter);
                        }
                    }
                }
            }
            SemTy::Dyn
        }
        Expr::Subscript(s) => annotation_subscript(s.value.as_ref(), s.slice.as_ref(), ctx),
        Expr::Constant(c) => match &c.value {
            Constant::None => SemTy::NoneTy,
            // A string annotation is a PEP-484 forward reference: resolve the
            // quoted name exactly like a bare one (`-> "CM"` ≡ `-> CM`).
            Constant::Str(s) => named_annotation(s, ctx),
            _ => SemTy::Dyn,
        },
        _ => SemTy::Dyn,
    }
}

/// Resolve a (possibly forward-referenced) annotation NAME to a `SemTy`.
pub(super) fn named_annotation(name: &str, ctx: &AnnCtx) -> SemTy {
    match name {
        "int" => SemTy::Int,
        "float" => SemTy::Float,
        "bool" => SemTy::Bool,
        "str" => SemTy::Str,
        "bytes" => SemTy::Bytes,
        "None" | "NoneType" => SemTy::NoneTy,
        "list" | "List" => SemTy::list_of(SemTy::Dyn),
        "dict" | "Dict" => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
        "set" | "Set" | "frozenset" => SemTy::set_of(SemTy::Dyn),
        "tuple" | "Tuple" => SemTy::tuple_var_of(SemTy::Dyn),
        other => {
            // An in-scope type variable (Phase 5E) → `SemTy::Var`.
            if let Some(id) = ctx.type_vars.get(other) {
                return SemTy::Var(*id);
            }
            // A module type alias (`type X = T` / `X: TypeAlias = T`, PLAN §3 B/C)
            // resolves to its body. Checked before `class_map` so an alias name
            // never collides with a class; aliases never shadow a type var above.
            if let Some(sty) = ctx.type_aliases.get(other) {
                return sty.clone();
            }
            // A user-defined class name annotates an instance of that class.
            if let Some((class_id, name)) = ctx.class_map.get(other) {
                // A `Protocol` annotation (PLAN §3 G) erases to `Dyn` (Tagged
                // baseline): method dispatch rides the gradual `rt_obj_method`
                // path, never a per-protocol ABI. `isinstance` resolves the
                // protocol class by NAME (independent of this), so structural
                // checks still see the protocol's class id.
                if ctx.proto_ids.contains(class_id) {
                    return SemTy::Dyn;
                }
                return SemTy::Class {
                    class_id: *class_id,
                    name: *name,
                };
            }
            // A from-imported stdlib class (`from time import struct_time`).
            if let Some(ty) = ctx.stdlib.classes.get(other) {
                return ty.clone();
            }
            // `collections.Counter` is bound as a construction FUNCTION (not a
            // class), so `from collections import Counter` puts it in `funcs`.
            // Used as an annotation (`x: Counter`) it names the runtime-object
            // type. Import-gated (an un-imported name stays `Dyn`) and checked
            // after user classes (a user `class Counter` wins, above).
            if let Some(def) = ctx.stdlib.funcs.get(other) {
                if is_counter_def(def) {
                    return SemTy::RuntimeObject(pyaot_core_defs::TypeTagKind::Counter);
                }
            }
            SemTy::Dyn
        }
    }
}

/// Map a subscripted generic annotation (`list[int]`, `dict[K, V]`, …) to a
/// `SemTy`. Unknown bases fall back to `Dyn`.
pub(super) fn annotation_subscript(base: &Expr, slice: &Expr, ctx: &AnnCtx) -> SemTy {
    let Expr::Name(n) = base else {
        return SemTy::Dyn;
    };
    match n.id.as_str() {
        "list" | "List" => SemTy::list_of(annotation_to_semty(slice, ctx)),
        "set" | "Set" | "frozenset" => SemTy::set_of(annotation_to_semty(slice, ctx)),
        "dict" | "Dict" => match slice {
            Expr::Tuple(t) if t.elts.len() == 2 => SemTy::dict_of(
                annotation_to_semty(&t.elts[0], ctx),
                annotation_to_semty(&t.elts[1], ctx),
            ),
            _ => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
        },
        "tuple" | "Tuple" => match slice {
            // `tuple[T, ...]` is the homogeneous variable-length tuple.
            Expr::Tuple(t) if t.elts.len() == 2 && is_ellipsis(&t.elts[1]) => {
                SemTy::tuple_var_of(annotation_to_semty(&t.elts[0], ctx))
            }
            Expr::Tuple(t) => {
                SemTy::tuple_of(t.elts.iter().map(|e| annotation_to_semty(e, ctx)).collect())
            }
            single => SemTy::tuple_of(vec![annotation_to_semty(single, ctx)]),
        },
        "Optional" => SemTy::optional(annotation_to_semty(slice, ctx)),
        // `Callable[[T…], R]` / `Callable[..., R]` (Phase 6A). The ellipsis form
        // is the `(*args, **kwargs)` signature — exactly one tuple param + one
        // dict param (Phase 6C ABI).
        "Callable" => callable_annotation(slice, ctx),
        // A user generic class annotation `Stack[int]` → `Generic{base, [int]}` (5E).
        // A subscripted `Protocol[T]` annotation (PLAN §3 G) erases to `Dyn` like
        // the bare-name protocol case in `named_annotation`.
        other => match ctx.class_map.get(other) {
            Some((class_id, _)) if ctx.proto_ids.contains(class_id) => SemTy::Dyn,
            Some((class_id, _)) => SemTy::Generic {
                base: *class_id,
                args: subscript_type_args(slice, ctx),
            },
            None => SemTy::Dyn,
        },
    }
}

/// True iff `e` is the `...` (Ellipsis) literal — the `tuple[T, ...]` marker.
pub(super) fn is_ellipsis(e: &Expr) -> bool {
    matches!(e, Expr::Constant(c) if matches!(c.value, Constant::Ellipsis))
}

/// Map a `Callable[...]` annotation slice to a `SemTy::Callable`. Unknown
/// shapes fall back to `Dyn` (→ `Tagged`, the correct baseline — calling such a
/// value then gets the loud Dyn-callee diagnostic).
pub(super) fn callable_annotation(slice: &Expr, ctx: &AnnCtx) -> SemTy {
    let Expr::Tuple(t) = slice else {
        return SemTy::Dyn;
    };
    if t.elts.len() != 2 {
        return SemTy::Dyn;
    }
    let ret = annotation_to_semty(&t.elts[1], ctx);
    match &t.elts[0] {
        Expr::List(l) => SemTy::Callable(Box::new(Sig::fixed(
            l.elts.iter().map(|e| annotation_to_semty(e, ctx)).collect(),
            ret,
        ))),
        e if is_ellipsis(e) => SemTy::Callable(Box::new(Sig {
            params: vec![
                SemTy::tuple_var_of(SemTy::Dyn),
                SemTy::dict_of(SemTy::Str, SemTy::Dyn),
            ],
            ret,
            varargs: true,
            kwargs: true,
        })),
        _ => SemTy::Dyn,
    }
}


/// Shared `def`/method/nested-def lowering. `name` is the function's (possibly
/// synthetic) interned name; `name_str` the raw base for child synthetics;
/// `first` controls the first parameter; `enclosing` is the class for `super()`;
/// `allow_decorators` permits the already-classified Phase-5D decorators (the
/// caller has validated them). `nested` is `Some((captures, facts))` for a
/// nested def: the function gets `__env__: Dyn` as explicit param 0 and a
/// capture-unpacking prologue. Reserves and fills the function's `FuncId`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_callable(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    def: &StmtFunctionDef,
    name_str: &str,
    name: InternedString,
    first: FirstParam,
    enclosing: Option<ClassId>,
    allow_decorators: bool,
    nested: Option<(&[(String, SemTy)], &ScopeFacts)>,
) -> Result<FuncId> {
    let span = to_span(def.range());
    if !allow_decorators && !def.decorator_list.is_empty() {
        return Err(parse_error(
            "decorators are out of scope for this milestone",
            span,
        ));
    }
    let ret_ty = match &def.returns {
        Some(e) => annotation_to_semty(e.as_ref(), ctx),
        None => SemTy::Dyn,
    };
    let parsed = parse_params(interner, ctx, def.args.as_ref(), &first, name)?;
    // The function's own scoping facts (computed by the caller for nested defs,
    // fresh here for top-level ones — same analysis either way).
    let own_facts;
    let facts = match nested {
        Some((_, f)) => f,
        None => {
            own_facts = freevars::analyze_def(def);
            &own_facts
        }
    };
    // `nonlocal x` requires an enclosing function binding for `x` — i.e. it must
    // be among this function's captures (the CPython SyntaxError otherwise).
    for n in &facts.nonlocals {
        let captured = matches!(nested, Some((caps, _)) if caps.iter().any(|(c, _)| c == n));
        if !captured {
            return Err(parse_error(
                format!("no binding for nonlocal '{n}' found"),
                span,
            ));
        }
    }

    let fid = shared.reserve();
    let varargs = parsed.varargs.is_some();
    let kwargs = parsed.kwargs.is_some();

    // A `def` containing `yield` is a generator (Phase 6E): build the wrapper
    // (into `fid`) + a resume state machine instead of a plain body. Captures /
    // *args / **kwargs in a generator are out of scope.
    if body_has_yield(&def.body) {
        if let Some((caps, _)) = nested {
            if !caps.is_empty() {
                let mut names: Vec<&str> = caps.iter().map(|(c, _)| c.as_str()).collect();
                names.sort_unstable();
                return Err(parse_error(
                    format!(
                        "a nested generator that captures an enclosing local is out of \
                         scope (captures: {}); a capture-free nested generator is supported",
                        names.join(", ")
                    ),
                    span,
                ));
            }
        }
        if varargs || kwargs {
            return Err(parse_error(
                "a generator with *args/**kwargs is out of scope (Phase 6E)",
                span,
            ));
        }
        lower_generator_def(
            interner,
            ctx,
            shared,
            &def.body,
            name_str,
            name,
            fid,
            &parsed,
            ret_ty,
            enclosing,
            nested.is_some(),
        )?;
        return Ok(fid);
    }

    let mut fl = FnLowerer::new(interner, ctx, shared, name, name_str, ret_ty, enclosing);
    fl.set_scope_facts(facts);
    if nested.is_some() {
        let env_name = fl.intern("__env__");
        fl.add_param(env_name, SemTy::Dyn);
    }
    fl.install_params(&parsed);
    if let Some((captures, f)) = nested {
        fl.install_captures(captures, f, span);
        // Self-recursion: the def's own name among its captures.
        if captures.iter().any(|(c, _)| c == def.name.as_str()) {
            let iname = fl.intern(def.name.as_str());
            if let Some(Binding::Cell(lid)) = fl.scope.get(&iname).copied() {
                fl.self_capture = Some((lid, name));
            }
        }
    }
    fl.init_cells();
    fl.lower_body(&def.body)?;
    let mut func = fl.finish(HirTerminator::Return(None));
    func.varargs = varargs;
    func.kwargs = kwargs;
    shared.fill(fid, func);
    Ok(fid)
}

/// Parse a callable's parameter list into the call-facing [`ParsedParams`]
/// shape (Phase 6C). The first fixed param is typed by `first` for instance
/// methods; a classmethod's `cls` is dropped. Defaults must be constant
/// literals (`x=[]` is rejected loudly, the `ClassAttrInit` shape).
pub(super) fn parse_params(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    args: &rustpython_parser::ast::Arguments,
    first: &FirstParam,
    fname: InternedString,
) -> Result<ParsedParams> {
    let skip = matches!(first, FirstParam::SkipCls) as usize;
    let mut fixed = Vec::new();
    for (i, awd) in args
        .posonlyargs
        .iter()
        .chain(args.args.iter())
        .skip(skip)
        .enumerate()
    {
        let ty = match (i, first) {
            (0, FirstParam::Method(t)) => t.clone(),
            _ => match &awd.def.annotation {
                Some(a) => annotation_to_semty(a.as_ref(), ctx),
                None => SemTy::Dyn,
            },
        };
        let pname = interner.intern(awd.def.arg.as_str());
        let default = match &awd.default {
            Some(e) => Some(resolve_param_default(interner, ctx, fname, pname, e)?),
            None => None,
        };
        fixed.push(ParamInfo {
            name: pname,
            ty,
            default,
        });
    }
    let mut kwonly = Vec::new();
    for awd in &args.kwonlyargs {
        let ty = match &awd.def.annotation {
            Some(a) => annotation_to_semty(a.as_ref(), ctx),
            None => SemTy::Dyn,
        };
        let pname = interner.intern(awd.def.arg.as_str());
        let default = match &awd.default {
            Some(e) => Some(resolve_param_default(interner, ctx, fname, pname, e)?),
            None => None,
        };
        kwonly.push(ParamInfo {
            name: pname,
            ty,
            default,
        });
    }
    let varargs = args
        .vararg
        .as_ref()
        .map(|a| interner.intern(a.arg.as_str()));
    let kwargs = args.kwarg.as_ref().map(|a| interner.intern(a.arg.as_str()));
    Ok(ParsedParams {
        fixed,
        kwonly,
        varargs,
        kwargs,
    })
}


/// Rewrite module-level `name = lambda ... (with DEFAULTS)` into a synthetic
/// `def name(...)` (Phase 8H, #9) — the def machinery provides the default
/// materialization and known-callee keyword adaptation that the closure path
/// rejects. Applies only when `name` is bound EXACTLY once at module scope
/// (rebinding keeps CPython's late-binding closure semantics) and the lambda
/// has no *args/**kwargs. Lambdas without defaults keep the closure path; a
/// lambda with defaults anywhere else still gets the loud rejection.
pub(crate) fn desugar_module_lambda_defs(body: &mut [Stmt]) {
    for i in 0..body.len() {
        let replacement = {
            let Stmt::Assign(a) = &body[i] else { continue };
            if a.targets.len() != 1 {
                continue;
            }
            let Expr::Name(n) = &a.targets[0] else {
                continue;
            };
            let Expr::Lambda(l) = a.value.as_ref() else {
                continue;
            };
            let args = l.args.as_ref();
            let has_defaults = args
                .posonlyargs
                .iter()
                .chain(args.args.iter())
                .any(|x| x.default.is_some());
            if !has_defaults
                || args.vararg.is_some()
                || args.kwarg.is_some()
                || !args.kwonlyargs.is_empty()
            {
                continue;
            }
            if count_scope_bindings(body, n.id.as_str()) != 1 {
                continue;
            }
            Stmt::FunctionDef(StmtFunctionDef {
                range: a.range,
                name: n.id.clone(),
                args: l.args.clone(),
                body: vec![Stmt::Return(rustpython_parser::ast::StmtReturn {
                    range: l.range,
                    value: Some(l.body.clone()),
                })],
                decorator_list: vec![],
                returns: None,
                type_comment: None,
                type_params: vec![],
            })
        };
        body[i] = replacement;
    }
}

pub(super) fn collect_target_names<'a>(target: &'a Expr, out: &mut Vec<&'a str>) {
    match target {
        Expr::Name(n) => out.push(n.id.as_str()),
        Expr::Tuple(t) => {
            for e in &t.elts {
                collect_target_names(e, out);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                collect_target_names(e, out);
            }
        }
        _ => {}
    }
}

pub(super) fn to_span(range: TextRange) -> Span {
    Span::new(range.start().to_u32(), range.end().to_u32())
}

pub(super) fn parse_error(msg: impl Into<String>, span: Span) -> CompilerError {
    CompilerError::parse_error(msg.into(), span)
}

/// The canonical `SemTy` target for a builtin-type `isinstance` element name
/// (`isinstance(x, str)`, or a tuple element `isinstance(x, (str, int))`).
/// Container builtins carry a canonical (Dyn-element) target; the static fold
/// in `lowering::lower_isinstance_builtin` matches by KIND (isinstance ignores
/// element types), so the concrete element types are irrelevant. `None` for a
/// name with no canonical mapping (e.g. `frozenset`) — a loud error upstream.
pub(super) fn isinstance_builtin_target(name: &str) -> Option<SemTy> {
    match name {
        "str" => Some(SemTy::Str),
        "int" => Some(SemTy::Int),
        "float" => Some(SemTy::Float),
        "bool" => Some(SemTy::Bool),
        "bytes" => Some(SemTy::Bytes),
        "list" => Some(SemTy::list_of(SemTy::Dyn)),
        "dict" => Some(SemTy::dict_of(SemTy::Dyn, SemTy::Dyn)),
        "set" => Some(SemTy::set_of(SemTy::Dyn)),
        "tuple" => Some(SemTy::tuple_var_of(SemTy::Dyn)),
        _ => None,
    }
}


