use super::*;

impl<'a> FnLowerer<'a> {
    /// Adapt a call to a known top-level function (Phase 6C): reorder keyword
    /// args, fill constant defaults, and pack `*args` / `**kwargs` — producing
    /// the positional argument vector matching the callee's MIR parameter order
    /// (fixed → keyword-only → `*args` tuple → `**kwargs` dict).
    /// Lower the builtin `open(file, mode="r", encoding=None)` (Phase 8C)
    /// through the shared stdlib-call adapter, against a synthetic descriptor
    /// targeting `rt_file_open`. The result's `binary`-ness is derived in
    /// typeck from the (constant) mode literal.
    pub(super) fn lower_open_builtin(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        self.lower_stdlib_call(&OPEN_DEF, c, span)
    }

    /// Adapt a Python-level stdlib call against its declarative descriptor and
    /// emit [`HirExprKind::CallRuntime`] (Phase 8B). Positional args fill param
    /// slots in order; keywords match by `ParamDef.name`; an absent optional
    /// param takes its `ConstValue` default as a literal, or stays an empty
    /// slot (the null-pointer sentinel) when it has none. The user-written arg
    /// count is recorded for `pass_arg_count` descriptors.
    pub(super) fn lower_stdlib_call(
        &mut self,
        def: &'static pyaot_stdlib_defs::StdlibFunctionDef,
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        if c.args.iter().any(|a| matches!(a, Expr::Starred(_))) {
            return Err(parse_error(
                "`*` spreading into a stdlib call is out of scope",
                span,
            ));
        }
        // A `variadic_to_list` descriptor (`os.path.join(*paths)`) collects all
        // positional args into one list passed as the single runtime arg
        // (Phase 8D). These descriptors are pure-variadic (no leading fixed
        // params) and take no keywords.
        if def.hints.variadic_to_list {
            if !c.keywords.is_empty() {
                return Err(parse_error(
                    format!("`{}()` does not take keyword arguments", def.name),
                    span,
                ));
            }
            let provided = c.args.len();
            if provided < def.min_args {
                return Err(parse_error(
                    format!(
                        "`{}()` takes at least {} argument(s)",
                        def.name, def.min_args
                    ),
                    span,
                ));
            }
            let elem_spec = &def.params[0].ty;
            let mut elems = Vec::with_capacity(c.args.len());
            for a in &c.args {
                elems.push(self.lower_stdlib_arg(a, elem_spec)?);
            }
            let list = self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span);
            return Ok(self.alloc(
                HirExprKind::CallRuntime {
                    target: pyaot_hir::RuntimeCallTarget::Func(def),
                    args: vec![Some(list)],
                    provided: provided as u32,
                },
                SemTy::Dyn,
                span,
            ));
        }
        let provided = c.args.len() + c.keywords.len();
        if provided < def.min_args || (def.max_args != usize::MAX && provided > def.max_args) {
            return Err(parse_error(
                format!(
                    "`{}()` takes {}..={} argument(s) but {provided} were given",
                    def.name, def.min_args, def.max_args,
                ),
                span,
            ));
        }
        // With keywords present, slot matching reorders arguments — stage every
        // (non-literal) value in WRITTEN order first (CPython evaluation order).
        let staging = !c.keywords.is_empty();
        let mut positionals: Vec<ArgSrc> = Vec::with_capacity(c.args.len());
        for a in &c.args {
            positionals.push(if staging {
                self.stage_arg_src(a)?
            } else {
                ArgSrc::Plain(a)
            });
        }
        let mut keywords: Vec<(String, ArgSrc, bool)> = Vec::new();
        for kw in &c.keywords {
            let Some(name) = &kw.arg else {
                return Err(parse_error(
                    "`**kwargs` spreading is out of scope here",
                    span,
                ));
            };
            let src = self.stage_arg_src(&kw.value)?;
            keywords.push((name.as_str().to_string(), src, false));
        }

        let mut slots: Vec<Option<Idx<HirExpr>>> = Vec::with_capacity(def.params.len());
        for (i, p) in def.params.iter().enumerate() {
            let v = if i < positionals.len() {
                self.stdlib_arg_slot(positionals[i], &p.ty, p.optional, span)?
            } else if let Some(kv) = take_keyword(&mut keywords, p.name) {
                self.stdlib_arg_slot(kv, &p.ty, p.optional, span)?
            } else if let Some(cv) = &p.default {
                Some(self.lower_stdlib_const(cv, span))
            } else if p.optional {
                None
            } else {
                return Err(parse_error(
                    format!("`{}()` missing required argument `{}`", def.name, p.name),
                    span,
                ));
            };
            slots.push(v);
        }
        if let Some((k, _, _)) = keywords.iter().find(|(_, _, used)| !used) {
            return Err(parse_error(
                format!("`{}()` got an unexpected keyword argument `{k}`", def.name),
                span,
            ));
        }
        Ok(self.alloc(
            HirExprKind::CallRuntime {
                target: pyaot_hir::RuntimeCallTarget::Func(def),
                args: slots,
                provided: provided as u32,
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// Fill one stdlib-call argument slot. An explicit `None` passed to an
    /// optional OBJECT param (`urlopen(url, None, …)`, `Request(url, data=None)`)
    /// becomes an absent slot → the null-pointer sentinel the runtime expects
    /// (`Value::NONE` would be `unwrap_ptr`-ed into a wild pointer). Raw
    /// primitive params (`Float`/`Int`/`Bool`) keep the literal. Phase 8D.
    pub(super) fn stdlib_arg_slot(
        &mut self,
        arg: ArgSrc<'_>,
        spec: &pyaot_stdlib_defs::TypeSpec,
        optional: bool,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        use pyaot_stdlib_defs::TypeSpec;
        let arg = match arg {
            ArgSrc::Plain(e) => e,
            // Already staged — never a literal (see `stage_arg_src`), so the
            // None-sentinel / int→float folds below cannot apply.
            ArgSrc::Staged(l) => return Ok(Some(self.local_ref(l, span))),
        };
        let is_object = !matches!(spec, TypeSpec::Float | TypeSpec::Int | TypeSpec::Bool);
        if optional && is_object && is_none_lit(arg) {
            return Ok(None);
        }
        Ok(Some(self.lower_stdlib_arg(arg, spec)?))
    }

    /// Lower one stdlib-call argument. An integer literal headed for a `Float`
    /// param becomes a float literal (CPython's int→float coercion, performed
    /// at the only place it is statically certain — implicit conversion of a
    /// runtime int at a raw-ABI boundary stays a typeck error).
    pub(super) fn lower_stdlib_arg(
        &mut self,
        e: &Expr,
        spec: &pyaot_stdlib_defs::TypeSpec,
    ) -> Result<Idx<HirExpr>> {
        if matches!(spec, pyaot_stdlib_defs::TypeSpec::Float) {
            let int_lit = |expr: &Expr| -> Option<f64> {
                if let Expr::Constant(k) = expr {
                    if let Constant::Int(i) = &k.value {
                        return i.to_string().parse::<f64>().ok();
                    }
                }
                None
            };
            let span = to_span(e.range());
            if let Some(f) = int_lit(e) {
                return Ok(self.alloc(HirExprKind::FloatLit(f), SemTy::Float, span));
            }
            // `-5` parses as USub(Constant) — fold it too.
            if let Expr::UnaryOp(u) = e {
                if matches!(u.op, PyUnaryOp::USub) {
                    if let Some(f) = int_lit(u.operand.as_ref()) {
                        return Ok(self.alloc(HirExprKind::FloatLit(-f), SemTy::Float, span));
                    }
                }
            }
        }
        self.lower_expr(e)
    }

    /// Materialize a descriptor's `ConstValue` default as a literal expr.
    pub(super) fn lower_stdlib_const(
        &mut self,
        cv: &pyaot_stdlib_defs::ConstValue,
        span: Span,
    ) -> Idx<HirExpr> {
        use pyaot_stdlib_defs::ConstValue;
        match cv {
            ConstValue::Int(i) => self.alloc(HirExprKind::IntLit(*i), SemTy::Int, span),
            ConstValue::Float(f) => self.alloc(HirExprKind::FloatLit(*f), SemTy::Float, span),
            ConstValue::Bool(b) => self.alloc(HirExprKind::BoolLit(*b), SemTy::Bool, span),
            ConstValue::Str(s) => {
                let id = self.intern(s);
                self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
            }
        }
    }

}

/// Reject keyword args and `*`/`**` spreads for a call form that does not
/// support them (generic construction, method calls, the desugared builtins).
/// True for the `functools.reduce` stdlib descriptor — identified by its unique
/// runtime symbol, so the call is rerouted to the HOF desugar
/// ([`Lowerer::lower_reduce`]) instead of the raw-ABI `rt_reduce` callback path.
/// The descriptor exists only for `from functools import reduce` recognition.
pub(super) fn is_reduce_def(def: &pyaot_stdlib_defs::StdlibFunctionDef) -> bool {
    def.runtime_name == "rt_reduce"
}

/// `collections.Counter` — recognized by the `COUNTER_NEW` import binding's
/// sentinel runtime name. The frontend intercepts construction (see
/// [`Lowerer::lower_counter_construct`]) to pick the empty vs from-iterable
/// runtime symbol and type the result `RuntimeObject(Counter)`.
pub(super) fn is_counter_def(def: &pyaot_stdlib_defs::StdlibFunctionDef) -> bool {
    def.runtime_name == "rt_make_counter"
}

/// `collections.deque` — recognized by the `DEQUE_NEW` import binding's sentinel
/// runtime name. The frontend intercepts construction (see
/// [`Lowerer::lower_deque_construct`]) to pick the empty vs from-iterable runtime
/// symbol and type the result `RuntimeObject(Deque)`.
pub(super) fn is_deque_def(def: &pyaot_stdlib_defs::StdlibFunctionDef) -> bool {
    def.runtime_name == "rt_make_deque"
}

/// `collections.defaultdict` — recognized by the `DEFAULTDICT_NEW` import
/// binding's sentinel runtime name. The frontend intercepts construction (see
/// [`Lowerer::lower_defaultdict_construct`]) so the factory argument (`int`,
/// `list`, …) is mapped to a raw tag instead of being lowered as a value (which
/// would fail with `undefined name 'set'` for a bare type Name).
pub(super) fn is_defaultdict_def(def: &pyaot_stdlib_defs::StdlibFunctionDef) -> bool {
    def.runtime_name == "rt_make_defaultdict"
}

/// Map a `defaultdict(...)` factory type name to its `(runtime tag, value SemTy)`.
/// The tag is the runtime `FACTORY_*` constant packed into the `DictObj` header
/// (`defaultdict.rs`); the value SemTy types the auto-inserted default so a
/// typed-`V` read dispatches the right method (`list` → `list.append`, …).
/// Returns `None` for anything outside the supported builtin factories (§10 keeps
/// to the concrete builtins — no first-class type objects).
pub(super) fn defaultdict_factory(name: &str) -> Option<(i64, SemTy)> {
    let tag = match name {
        "int" => 0,
        "float" => 1,
        "str" => 2,
        "bool" => 3,
        "list" => 4,
        "dict" => 5,
        "set" => 6,
        _ => return None,
    };
    Some((tag, SemTy::defaultdict_value_ty(tag)))
}

