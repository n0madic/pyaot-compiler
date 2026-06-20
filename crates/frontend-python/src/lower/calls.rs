use super::*;

impl<'a> FnLowerer<'a> {
    /// A call used as a value (builtins now; user functions in 2d). `print` is a
    /// statement, not a value-call, so reject it here.
    pub(super) fn lower_call_expr(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if let Expr::Name(n) = c.func.as_ref() {
            if n.id.as_str() == "print" {
                return Err(parse_error(
                    "print() is only supported as a statement",
                    span,
                ));
            }
        }
        // `object.__new__(cls)` (§3): the allocator hook, called inside a user
        // `__new__`. Lowers to `rt_object_new(cls as i8)` → a bare heap instance.
        if let Expr::Attribute(a) = c.func.as_ref() {
            if a.attr.as_str() == "__new__" {
                if let Expr::Name(base) = a.value.as_ref() {
                    let object_iname = self.intern("object");
                    if base.id.as_str() == "object" && !self.scope.contains_key(&object_iname) {
                        if c.args.len() != 1 || !c.keywords.is_empty() {
                            return Err(parse_error(
                                "object.__new__ takes a single positional `cls` argument",
                                span,
                            ));
                        }
                        let cls = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(HirExprKind::ObjectNew { cls }, SemTy::Dyn, span));
                    }
                }
            }
        }
        // `Cls[T](args)` → a subscripted generic construction (Phase 5E).
        if let Expr::Subscript(s) = c.func.as_ref() {
            if let Expr::Name(n) = s.value.as_ref() {
                if let Some((class_id, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
                    reject_call_extras(c, span, "generic construction")?;
                    let type_args = subscript_type_args(s.slice.as_ref(), self.ctx);
                    let args = self.lower_expr_list(&c.args)?;
                    return Ok(self.alloc(
                        HirExprKind::GenericConstruct {
                            class_id,
                            type_args,
                            args,
                        },
                        SemTy::Dyn,
                        span,
                    ));
                }
            }
        }
        // `isinstance(value, Cls)` against a known user class → the runtime
        // inheritance-aware check (Phase 5B). `isinstance(value, str|int|
        // float|bool)` → the static fold (Phase 8B). Other forms fall through.
        if let Expr::Name(n) = c.func.as_ref() {
            if n.id.as_str() == "isinstance" && c.args.len() == 2 && c.keywords.is_empty() {
                if let Expr::Name(cls) = &c.args[1] {
                    if let Some((class_id, _)) = self.ctx.class_map.get(cls.id.as_str()).copied() {
                        let value = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::IsInstance { value, class_id },
                            SemTy::Bool,
                            span,
                        ));
                    }
                    if let Some(target) = isinstance_builtin_target(cls.id.as_str()) {
                        let value = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::IsInstanceBuiltin { value, target },
                            SemTy::Bool,
                            span,
                        ));
                    }
                    // A single Name that is neither a known class nor a builtin
                    // type: fall through WITHOUT lowering the receiver.
                }
                // `isinstance(value, (A, B, ...))` ≡ `isinstance(value, A) or
                // isinstance(value, B) or ...`, with the receiver evaluated ONCE
                // (CPython semantics) and an empty tuple ⇒ `False`. Pure frontend
                // desugar over the existing per-element checks; nested type-tuples
                // flatten. User-class (`IsInstance`, runtime check) and builtin
                // (`IsInstanceBuiltin`, static fold) elements mix freely.
                if let Expr::Tuple(t) = &c.args[1] {
                    let recv = self.stage_arg(&c.args[0])?;
                    let mut checks = Vec::new();
                    self.collect_isinstance_checks(&t.elts, recv, &mut checks, span)?;
                    return Ok(self.or_combine_checks(checks, span));
                }
            }
        }
        // The builtin `open(...)` (Phase 8C) → the synthetic File-open
        // descriptor, unless a user binding (local/param/top-level def) shadows
        // the name.
        if let Expr::Name(n) = c.func.as_ref() {
            if n.id.as_str() == "open" {
                let iname = self.intern("open");
                if !self.scope.contains_key(&iname) && !self.ctx.top_defs.contains_key("open") {
                    return self.lower_open_builtin(c, span);
                }
            }
        }
        // A from-imported stdlib function called by its bound name (Phase 8B):
        // `sqrt(2.0)` after `from math import sqrt`. A local/param of the same
        // name shadows the binding.
        if let Expr::Name(n) = c.func.as_ref() {
            let iname = self.intern(n.id.as_str());
            if !self.scope.contains_key(&iname) {
                if let Some(def) = self.ctx.stdlib.funcs.get(n.id.as_str()).copied() {
                    if is_reduce_def(def) {
                        reject_call_extras(c, span, "reduce()")?;
                        return self.lower_reduce(&c.args, span);
                    }
                    if is_counter_def(def) {
                        return self.lower_counter_construct(c, span);
                    }
                    if is_deque_def(def) {
                        return self.lower_deque_construct(c, span);
                    }
                    if is_defaultdict_def(def) {
                        return self.lower_defaultdict_construct(c, span);
                    }
                    return self.lower_stdlib_call(def, c, span);
                }
            }
        }
        // `M.f(...)` / `M.sub.f(...)` through an `import M` stdlib alias (Phase
        // 8B/8D): flatten the (possibly multi-level) attribute chain to a dotted
        // key and dispatch to the runtime descriptor — `os.getcwd()` and
        // `os.path.join(...)` alike. The leftmost name must be a stdlib alias
        // and unshadowed.
        if let Some((leftmost, dotted)) = flatten_attr_chain(c.func.as_ref()) {
            let lname = self.intern(leftmost);
            if self.ctx.stdlib.aliases.contains(leftmost) && !self.scope.contains_key(&lname) {
                if let Some(def) = self.ctx.stdlib.funcs.get(&dotted).copied() {
                    // `functools.reduce(...)` (qualified) — same HOF desugar as
                    // the from-imported bare form above.
                    if is_reduce_def(def) {
                        reject_call_extras(c, span, "reduce()")?;
                        return self.lower_reduce(&c.args, span);
                    }
                    // `collections.Counter(...)` (qualified) — same construction
                    // intercept as the from-imported bare form above.
                    if is_counter_def(def) {
                        return self.lower_counter_construct(c, span);
                    }
                    // `collections.deque(...)` (qualified) — same as the bare form.
                    if is_deque_def(def) {
                        return self.lower_deque_construct(c, span);
                    }
                    // `collections.defaultdict(...)` (qualified) — same as bare.
                    if is_defaultdict_def(def) {
                        return self.lower_defaultdict_construct(c, span);
                    }
                    return self.lower_stdlib_call(def, c, span);
                }
                // Not a stdlib function. A LONGER chain whose 2-link prefix is a
                // known module attr (`sys.path.append(...)` — a list method on
                // the `sys.path` attr) falls through to the method-call path.
                // An unknown 2-link `module.attr(...)` (e.g. `re.findall`) is a
                // loud CPython-style AttributeError diagnostic instead of the
                // misleading "undefined name" from the generic path.
                if let Some((module, attr)) = dotted.split_once('.') {
                    if !attr.contains('.') && !self.stdlib_module_attr_exists(&dotted) {
                        return Err(parse_error(
                            format!("module '{module}' has no attribute '{attr}'"),
                            span,
                        ));
                    }
                }
            }
        }
        // `M.f(args)` / `M.Cls(args)` through an `import M` user-module alias
        // (Phase 8): a qualified access folds to an ordinary direct call / class
        // construction (the imported FuncId/ClassId lives under the `"M.name"`
        // key in `top_defs` / `class_map`). Handled before the method-call path
        // so the alias receiver is never mistaken for an object receiver.
        if let Expr::Attribute(attr) = c.func.as_ref() {
            if let Expr::Name(m) = attr.value.as_ref() {
                let mname = self.intern(m.id.as_str());
                if self.ctx.aliases.contains(m.id.as_str()) && !self.scope.contains_key(&mname) {
                    let qual = format!("{}.{}", m.id.as_str(), attr.attr.as_str());
                    if let Some((class_id, _)) = self.ctx.class_map.get(&qual).copied() {
                        reject_call_extras(c, span, "module class construction")?;
                        let args = self.lower_expr_list(&c.args)?;
                        return Ok(self.alloc(
                            HirExprKind::GenericConstruct {
                                class_id,
                                type_args: vec![],
                                args,
                            },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                    if let Some(info) = self.ctx.top_defs.get(&qual).cloned() {
                        return self.lower_direct_known_call(&info, &qual, c, span);
                    }
                    return Err(parse_error(
                        format!(
                            "module `{}` has no callable attribute `{}`",
                            m.id.as_str(),
                            attr.attr.as_str()
                        ),
                        span,
                    ));
                }
            }
        }
        // Generator `g.send(v)` / `g.close()` (Phase 6E): a generator-specific
        // method (no user class in our subset defines these), routed to the
        // runtime generator ops. `g.throw(...)` is out of scope.
        if let Expr::Attribute(attr) = c.func.as_ref() {
            match attr.attr.as_str() {
                "send" if c.args.len() == 1 && c.keywords.is_empty() => {
                    let gen = self.lower_expr(attr.value.as_ref())?;
                    let value = self.lower_expr(&c.args[0])?;
                    return Ok(self.alloc(
                        HirExprKind::GenQuery {
                            op: GenOp::Send,
                            gen,
                            imm: 0,
                            value: Some(value),
                        },
                        SemTy::Dyn,
                        span,
                    ));
                }
                "close" if c.args.is_empty() && c.keywords.is_empty() => {
                    let gen = self.lower_expr(attr.value.as_ref())?;
                    return Ok(self.alloc(
                        HirExprKind::GenQuery {
                            op: GenOp::Close,
                            gen,
                            imm: 0,
                            value: None,
                        },
                        SemTy::NoneTy,
                        span,
                    ));
                }
                _ => {}
            }
        }
        // `recv.method(args)` → a method call carrying the interned name. Lowering
        // dispatches by the receiver's static type: a container receiver to the
        // Phase-4D `ContainerMethod` path, a class receiver to the method's FuncId
        // (Phase 5). `super().method(args)` carries a `Super` receiver resolved at
        // lowering against the enclosing class's MRO. Unknown names are not rejected.
        if let Expr::Attribute(attr) = c.func.as_ref() {
            if has_starred_arg(c) {
                return Err(parse_error(
                    "`*args` spreading is not supported for method calls",
                    span,
                ));
            }
            if has_doublestar_kwarg(c) {
                return Err(parse_error(
                    "`**kwargs` spreading is not supported for method calls",
                    span,
                ));
            }
            // `dict.fromkeys(keys[, value])` — the CLASS-method form (§9). A bare
            // unshadowed `dict` in receiver position otherwise lowers to an
            // unresolved `Dyn` value, so the instance-form `ContainerMethod::
            // Fromkeys` dispatch (which keys off a dict-typed receiver) never
            // fires. Desugar to a `MethodCall` on a throwaway empty-dict receiver:
            // an empty `DictLit` types to `dict[Never, Never]`, so both typeck and
            // lowering select the Dict path and reuse the existing `Fromkeys`
            // machinery (the receiver value is discarded inside it). Gated on an
            // unshadowed `dict` like the `dict(...)`/`set(...)` constructor
            // interceptions, so a user binding named `dict` keeps winning.
            if attr.attr.as_str() == "fromkeys" && c.keywords.is_empty() {
                if let Expr::Name(base) = attr.value.as_ref() {
                    let dict_iname = self.intern("dict");
                    if base.id.as_str() == "dict"
                        && !self.scope.contains_key(&dict_iname)
                        && self.global_read_slot(dict_iname).is_none()
                        && !self.ctx.top_defs.contains_key("dict")
                        && !self.ctx.class_map.contains_key("dict")
                    {
                        if c.args.is_empty() || c.args.len() > 2 {
                            return Err(parse_error(
                                "dict.fromkeys() takes 1 or 2 positional arguments",
                                span,
                            ));
                        }
                        let recv =
                            self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
                        let method_name = self.intern("fromkeys");
                        let args = self.lower_expr_list(&c.args)?;
                        return Ok(self.alloc(
                            HirExprKind::MethodCall {
                                recv,
                                method_name,
                                args,
                                kwargs: vec![],
                            },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                }
            }
            // `.sort(key=K)` with a non-None key desugars HERE, by method NAME
            // (the receiver's type is not known until typeck). Documented
            // caveat: a user class with a `sort(key=)` method would mis-route;
            // mitigated by the type-tag TypeError guard in
            // `rt_list_sort_by_keys` (precedent: `g.send()`/`g.close()` above
            // are name-dispatched the same way).
            if attr.attr.as_str() == "sort"
                && !c.keywords.is_empty()
                && !is_super_call(attr.value.as_ref())
            {
                if let Some(out) = self.lower_sort_kwargs(attr, c, span)? {
                    return Ok(out);
                }
            }
            // `"literal".format(...)` (§9) desugars HERE, on a STRING-LITERAL
            // receiver, into the same `FormatValue` field machinery f-strings
            // use (the fields bind to positional / keyword args at compile time,
            // so `.format(name=…)` never reaches the keyword-less method gate). A
            // non-literal `var.format(...)` falls through to the generic
            // `MethodCall`, which reports an unsupported-method error.
            if attr.attr.as_str() == "format" && !is_super_call(attr.value.as_ref()) {
                if let Expr::Constant(rc) = attr.value.as_ref() {
                    if let Constant::Str(template) = &rc.value {
                        let template = template.clone();
                        return self.lower_str_format(&template, c, span);
                    }
                }
            }
            let staging = !c.keywords.is_empty();
            let recv = if is_super_call(attr.value.as_ref()) {
                let cid = self
                    .enclosing_class
                    .ok_or_else(|| parse_error("super() is only valid inside a method", span))?;
                self.alloc(HirExprKind::Super(cid), SemTy::Dyn, span)
            } else if staging && !matches!(attr.value.as_ref(), Expr::Name(_)) {
                // Keyword calls stage a compound receiver too — its side
                // effects come before every argument's (written order). A bare
                // name is a pure read AND may be a class reference
                // (`Cls.method(kw=…)`), which cannot live in a value slot.
                let l = self.stage_arg(attr.value.as_ref())?;
                self.local_ref(l, span)
            } else {
                self.lower_expr(attr.value.as_ref())?
            };
            let method_name = self.intern(attr.attr.as_str());
            let (args, kwargs) = if staging {
                // Stage positionals then keyword values in WRITTEN order.
                let mut args = Vec::with_capacity(c.args.len());
                for a in &c.args {
                    let src = self.stage_arg_src(a)?;
                    args.push(self.arg_src_value(src, span)?);
                }
                let mut kwargs = Vec::with_capacity(c.keywords.len());
                for kw in &c.keywords {
                    let kname = kw.arg.as_ref().expect("** rejected above");
                    let id = self.intern(kname.as_str());
                    let src = self.stage_arg_src(&kw.value)?;
                    kwargs.push((id, self.arg_src_value(src, span)?));
                }
                (args, kwargs)
            } else {
                (self.lower_expr_list(&c.args)?, vec![])
            };
            return Ok(self.alloc(
                HirExprKind::MethodCall {
                    recv,
                    method_name,
                    args,
                    kwargs,
                },
                SemTy::Dyn,
                span,
            ));
        }
        // Builtins that desugar to reduce / iterator loops are recognized by name
        // (like `print`/`range`; shadowing these names is not supported).
        if let Expr::Name(n) = c.func.as_ref() {
            // `min`/`max` accept the `key=` keyword (Phase 7).
            if matches!(n.id.as_str(), "min" | "max") {
                if has_starred_arg(c) {
                    return Err(parse_error(
                        "`*args` spreading is not supported for min()/max()",
                        span,
                    ));
                }
                let mut key: Option<&Expr> = None;
                for kw in &c.keywords {
                    match kw.arg.as_ref().map(|i| i.as_str()) {
                        Some("key") => key = Some(&kw.value),
                        Some(other) => {
                            return Err(parse_error(
                                format!(
                                    "min()/max() got an unsupported keyword argument '{other}'"
                                ),
                                span,
                            ))
                        }
                        None => {
                            return Err(parse_error("min()/max() do not support **kwargs", span))
                        }
                    }
                }
                return self.lower_minmax(&c.args, key, span, n.id.as_str() == "min");
            }
            if matches!(n.id.as_str(), "sum" | "set" | "next" | "iter") {
                reject_call_extras(c, span, "this builtin")?;
                match n.id.as_str() {
                    "sum" => return self.lower_sum(&c.args, span),
                    "set" => return self.lower_set_call(&c.args, span),
                    // `next(g)` (Phase 6E): resume the generator → its next value.
                    "next" => {
                        if c.args.len() != 1 {
                            return Err(parse_error("next() takes exactly one argument", span));
                        }
                        let gen = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::GenQuery {
                                op: GenOp::Next,
                                gen,
                                imm: 0,
                                value: None,
                            },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                    // `iter(iterable)`: build a runtime iterator object (the same
                    // `ContainerOp::Iter` → `rt_iter_value` the for-loop drives, so
                    // a File iterable routes through `rt_file_readlines` in lowering
                    // too). `next(it)` then consumes it via the raising `rt_iter_next`.
                    // The 2-arg sentinel form `iter(callable, sentinel)` is out of scope.
                    "iter" => {
                        if c.args.len() != 1 {
                            return Err(parse_error(
                                "only the 1-argument form iter(iterable) is supported",
                                span,
                            ));
                        }
                        let iterable = self.lower_expr(&c.args[0])?;
                        return Ok(self.alloc(
                            HirExprKind::ContainerExpr {
                                op: ContainerOp::Iter,
                                args: vec![iterable],
                            },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                    _ => {}
                }
            }
        }
        // Scalar / value builtins: `pow`, `divmod`, `all`, `any`,
        // `id`, `round`, `bin`, `hex`, `oct`. Gated on an UNSHADOWED bare name
        // (a local / global / top-def binding keeps winning — `id = 5; id(x)`
        // reads the local), slightly stricter than the unconditional min/max
        // intercept. None take keywords or `*`/`**` spreads.
        if let Expr::Name(n) = c.func.as_ref() {
            let iname = self.intern(n.id.as_str());
            let unshadowed = !self.scope.contains_key(&iname)
                && self.global_read_slot(iname).is_none()
                && !self.ctx.top_defs.contains_key(n.id.as_str());
            if unshadowed {
                use pyaot_stdlib_defs::modules::builtins as bd;
                match n.id.as_str() {
                    "pow" => {
                        reject_call_extras(c, span, "pow()")?;
                        return self.lower_pow(&c.args, span);
                    }
                    "divmod" => {
                        reject_call_extras(c, span, "divmod()")?;
                        return self.lower_divmod(&c.args, span);
                    }
                    "all" | "any" => {
                        reject_call_extras(c, span, "all()/any()")?;
                        return self.lower_all_any(&c.args, span, n.id.as_str() == "all");
                    }
                    "map" => {
                        reject_call_extras(c, span, "map()")?;
                        return self.lower_map(&c.args, span);
                    }
                    "filter" => {
                        reject_call_extras(c, span, "filter()")?;
                        return self.lower_filter(&c.args, span);
                    }
                    "format" => {
                        reject_call_extras(c, span, "format()")?;
                        return self.lower_format_builtin(&c.args, span);
                    }
                    // `str()` with no args → the empty string. Interned HERE (the
                    // frontend owns the mutable interner; lowering's is immutable
                    // and cannot mint the `""` literal). The one-arg `str(x)` form
                    // fails this guard and falls through to the `Symbol::Builtin`
                    // path, which honours `__str__`/`__repr__`.
                    "str" if c.args.is_empty() => {
                        reject_call_extras(c, span, "str()")?;
                        let id = self.intern("");
                        return Ok(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span));
                    }
                    "getattr" => {
                        reject_call_extras(c, span, "getattr()")?;
                        return self.lower_getattr_builtin(&c.args, span);
                    }
                    "setattr" => {
                        reject_call_extras(c, span, "setattr()")?;
                        return self.lower_setattr_builtin(&c.args, span);
                    }
                    "hasattr" => {
                        reject_call_extras(c, span, "hasattr()")?;
                        return self.lower_hasattr_builtin(&c.args, span);
                    }
                    "issubclass" => {
                        reject_call_extras(c, span, "issubclass()")?;
                        return self.lower_issubclass_builtin(&c.args, span);
                    }
                    "id" => {
                        reject_call_extras(c, span, "id()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_ID, c, span);
                    }
                    "round" => {
                        reject_call_extras(c, span, "round()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_ROUND, c, span);
                    }
                    "bin" => {
                        reject_call_extras(c, span, "bin()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_BIN, c, span);
                    }
                    "hex" => {
                        reject_call_extras(c, span, "hex()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_HEX, c, span);
                    }
                    "oct" => {
                        reject_call_extras(c, span, "oct()")?;
                        return self.lower_stdlib_call(&bd::BUILTIN_OCT, c, span);
                    }
                    _ => {}
                }
            }
        }
        // Direct self-recursion (Phase 6A): a nested function calling its own
        // name through its self-capture cell becomes a direct call to itself,
        // passing its env through (the cells stay shared).
        if c.keywords.is_empty() && !has_starred_arg(c) {
            if let Expr::Name(n) = c.func.as_ref() {
                if let Some((cell_lid, synth)) = self.self_capture {
                    let name = self.intern(n.id.as_str());
                    if self.scope.get(&name) == Some(&Binding::Cell(cell_lid)) {
                        let callee = self.alloc(
                            HirExprKind::Name(SymbolRef::Unresolved(synth)),
                            SemTy::Dyn,
                            span,
                        );
                        let mut args = vec![self.local_ref(LocalId::new(0), span)];
                        for a in &c.args {
                            args.push(self.lower_expr(a)?);
                        }
                        return Ok(self.alloc(
                            HirExprKind::Call { callee, args },
                            SemTy::Dyn,
                            span,
                        ));
                    }
                }
            }
        }
        // A decorated module-level function called by name (Phase 6D): its
        // promoted slot holds the decorator-wrapped uniform-thunk closure, so the
        // call is just an ordinary uniform indirect call on that slot (the
        // `lower_callee` global-read path reads it; lowering packs the args and
        // routes through `CallIndirect`). No bespoke `(*args, **kwargs)` pre-pack.
        //
        // A known top-level function called by name (not shadowed locally): the
        // frontend adapts keywords / defaults / `*args` packing at compile time
        // (Phase 6C). Everything else (indirect, builtins, classes) just lowers
        // its positional + spread args.
        if let Expr::Name(n) = c.func.as_ref() {
            let iname = self.intern(n.id.as_str());
            if !self.scope.contains_key(&iname) && self.global_read_slot(iname).is_none() {
                if let Some(info) = self.ctx.top_defs.get(n.id.as_str()).cloned() {
                    return self.lower_direct_known_call(&info, n.id.as_str(), c, span);
                }
            }
        }
        // Keyword arguments on container/iteration builtins (Phase 10):
        // `sorted(key=, reverse=)`, `enumerate(start=)`, `dict(a=1)`. Only for
        // a bare unshadowed name — user bindings keep winning above, and the
        // no-keyword forms keep their existing paths untouched.
        if let Expr::Name(n) = c.func.as_ref() {
            if !c.keywords.is_empty() {
                let iname = self.intern(n.id.as_str());
                if !self.scope.contains_key(&iname)
                    && self.global_read_slot(iname).is_none()
                    && !self.ctx.top_defs.contains_key(n.id.as_str())
                {
                    if let Some(out) = self.lower_builtin_kwargs_call(n.id.as_str(), c, span)? {
                        return Ok(out);
                    }
                }
            }
        }
        self.lower_indirect_or_unknown_call(c, span)
    }

    /// Lower a keyword-carrying call to a recognized builtin (Phase 10), or
    /// `None` to fall through to the generic (rejecting) path. Builtins that
    /// take no keywords get a precise diagnostic here instead of the generic
    /// indirect-call rejection.
    pub(super) fn lower_builtin_kwargs_call(
        &mut self,
        name: &str,
        c: &ExprCall,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        match name {
            "sorted" => Ok(Some(self.lower_sorted_kwargs(c, span)?)),
            "enumerate" => Ok(Some(self.lower_enumerate_kwargs(c, span)?)),
            "dict" => Ok(Some(self.lower_dict_kwargs(c, span)?)),
            "list" | "tuple" | "zip" | "reversed" | "len" | "bytes" | "set" | "sum" | "next"
            | "range" => Err(parse_error(
                format!("`{name}()` takes no keyword arguments"),
                span,
            )),
            _ => Ok(None),
        }
    }

    /// `sorted(xs, *, key=None, reverse=False)` with keywords (Phase 10).
    /// Without a key (or `key=None`): the standard container path with the
    /// reverse flag. With a key: copy → compiled key loop building a parallel
    /// keys list → `ListSortByKeys` tandem sort (no runtime callbacks); the
    /// result is the sorted copy. All argument values evaluate in written order.
    pub(super) fn lower_sorted_kwargs(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if c.args.len() != 1 || has_starred_arg(c) {
            return Err(parse_error(
                "sorted() takes exactly one positional argument",
                span,
            ));
        }
        let xs = self.stage_arg(&c.args[0])?;
        let mut key_mode: Option<KeyMode> = None;
        let mut rev: Option<LocalId> = None;
        for kw in &c.keywords {
            match kw.arg.as_ref().map(|i| i.as_str()) {
                Some("key") => {
                    if is_none_lit(&kw.value) {
                        continue;
                    }
                    // Same discipline as min/max: a bare out-of-scope name is
                    // called directly per element (builtins have no
                    // value-position thunk); anything else is staged once.
                    key_mode = Some(match &kw.value {
                        k @ Expr::Name(nm)
                            if {
                                let kn = self.intern(nm.id.as_str());
                                self.scope.contains_key(&kn)
                            } =>
                        {
                            KeyMode::Staged(self.stage_arg(k)?)
                        }
                        k @ Expr::Name(_) => KeyMode::ByName(k),
                        k => KeyMode::Staged(self.stage_arg(k)?),
                    });
                }
                Some("reverse") => rev = Some(self.stage_arg(&kw.value)?),
                Some(other) => {
                    return Err(parse_error(
                        format!("sorted() got an unexpected keyword argument `{other}`"),
                        span,
                    ))
                }
                None => return Err(parse_error("sorted() does not support **kwargs", span)),
            }
        }
        let rev_ref = match rev {
            Some(l) => self.local_ref(l, span),
            None => self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span),
        };
        let Some(km) = key_mode else {
            // No key: `sorted(xs, rev)` through the container builtin.
            let cname = self.intern("sorted");
            let callee = self.alloc(
                HirExprKind::Name(SymbolRef::Unresolved(cname)),
                SemTy::Dyn,
                span,
            );
            let xs_ref = self.local_ref(xs, span);
            return Ok(self.alloc(
                HirExprKind::Call {
                    callee,
                    args: vec![xs_ref, rev_ref],
                },
                SemTy::Dyn,
                span,
            ));
        };
        // copy = list(iter(xs)) — sorted never mutates its input.
        let xs_ref = self.local_ref(xs, span);
        let it = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![xs_ref],
            },
            SemTy::Dyn,
            span,
        );
        let copy_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::ListFromIter,
                args: vec![it],
            },
            SemTy::Dyn,
            span,
        );
        let copy = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: copy,
            value: copy_e,
        });
        // keys = [key(e) for e in copy] — the key call stays compiled code.
        let keys = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: keys,
            value: empty,
        });
        let copy_iter_ref = self.local_ref(copy, span);
        let lp = self.begin_iter_loop(copy_iter_ref, span)?;
        let kv = self.emit_key_call(&km, lp.elem, span)?;
        self.push_stmt(HirStmt::ContainerPush {
            container: keys,
            value: kv,
        });
        self.end_iter_loop(lp);
        // Tandem sort of copy by keys, then the copy IS the result.
        let copy_ref = self.local_ref(copy, span);
        let keys_ref = self.local_ref(keys, span);
        let sort_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::ListSortByKeys,
                args: vec![copy_ref, keys_ref, rev_ref],
            },
            SemTy::NoneTy,
            span,
        );
        let sink = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: sink,
            value: sort_e,
        });
        Ok(self.local_ref(copy, span))
    }

    /// `xs.sort(key=K[, reverse=R])` with a non-None key (Phase 10): stage the
    /// receiver and keyword values in written order, build the parallel keys
    /// list with a compiled loop, and tandem-sort in place via
    /// `ListSortByKeys`. Returns `None` (falls through to the generic
    /// `MethodCall` path) when the key is absent / the `None` literal — that
    /// form needs no name-dispatch caveat. The expression's value is `None`
    /// (in-place sort).
    pub(super) fn lower_sort_kwargs(
        &mut self,
        attr: &rustpython_parser::ast::ExprAttribute,
        c: &ExprCall,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        if !c.keywords.iter().any(|kw| {
            kw.arg.as_ref().is_some_and(|a| a.as_str() == "key") && !is_none_lit(&kw.value)
        }) {
            return Ok(None);
        }
        if !c.args.is_empty() {
            return Err(parse_error("sort() takes no positional arguments", span));
        }
        let recv = self.stage_arg(attr.value.as_ref())?;
        let mut key_mode: Option<KeyMode> = None;
        let mut rev: Option<LocalId> = None;
        for kw in &c.keywords {
            match kw.arg.as_ref().map(|i| i.as_str()) {
                Some("key") => {
                    key_mode = Some(match &kw.value {
                        k @ Expr::Name(nm)
                            if {
                                let kn = self.intern(nm.id.as_str());
                                self.scope.contains_key(&kn)
                            } =>
                        {
                            KeyMode::Staged(self.stage_arg(k)?)
                        }
                        k @ Expr::Name(_) => KeyMode::ByName(k),
                        k => KeyMode::Staged(self.stage_arg(k)?),
                    });
                }
                Some("reverse") => rev = Some(self.stage_arg(&kw.value)?),
                Some(other) => {
                    return Err(parse_error(
                        format!("sort() got an unexpected keyword argument `{other}`"),
                        span,
                    ))
                }
                None => return Err(parse_error("sort() does not support **kwargs", span)),
            }
        }
        let km = key_mode.expect("checked above: a non-None key is present");
        // keys = [key(e) for e in recv] — compiled key calls, no runtime callback.
        let keys = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: keys,
            value: empty,
        });
        let recv_iter_ref = self.local_ref(recv, span);
        let lp = self.begin_iter_loop(recv_iter_ref, span)?;
        let kv = self.emit_key_call(&km, lp.elem, span)?;
        self.push_stmt(HirStmt::ContainerPush {
            container: keys,
            value: kv,
        });
        self.end_iter_loop(lp);
        let recv_ref = self.local_ref(recv, span);
        let keys_ref = self.local_ref(keys, span);
        let rev_ref = match rev {
            Some(l) => self.local_ref(l, span),
            None => self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span),
        };
        let sort_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::ListSortByKeys,
                args: vec![recv_ref, keys_ref, rev_ref],
            },
            SemTy::NoneTy,
            span,
        );
        let sink = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: sink,
            value: sort_e,
        });
        Ok(Some(self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span)))
    }

    /// `enumerate(xs, start=k)` (Phase 10) — fold the keyword into the
    /// positional form the container path already accepts.
    pub(super) fn lower_enumerate_kwargs(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if c.args.is_empty() || c.args.len() > 2 || has_starred_arg(c) {
            return Err(parse_error(
                "enumerate() takes 1 positional argument plus optional `start`",
                span,
            ));
        }
        let it_src = self.stage_arg_src(&c.args[0])?;
        let mut start: Option<ArgSrc> = match c.args.get(1) {
            Some(a) => Some(self.stage_arg_src(a)?),
            None => None,
        };
        for kw in &c.keywords {
            match kw.arg.as_ref().map(|i| i.as_str()) {
                Some("start") => {
                    if start.is_some() {
                        return Err(parse_error(
                            "enumerate() got multiple values for argument `start`",
                            span,
                        ));
                    }
                    start = Some(self.stage_arg_src(&kw.value)?);
                }
                Some(other) => {
                    return Err(parse_error(
                        format!("enumerate() got an unexpected keyword argument `{other}`"),
                        span,
                    ))
                }
                None => return Err(parse_error("enumerate() does not support **kwargs", span)),
            }
        }
        let cname = self.intern("enumerate");
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(cname)),
            SemTy::Dyn,
            span,
        );
        let it_ref = self.arg_src_value(it_src, span)?;
        let start_ref = match start {
            Some(s) => self.arg_src_value(s, span)?,
            None => self.alloc(HirExprKind::IntLit(0), SemTy::Int, span),
        };
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![it_ref, start_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `dict(a=1, b=2)` / `dict(pos, a=1)` (Phase 10): pure-keyword form is a
    /// `DictLit` with string keys in written order; the mixed form builds the
    /// positional dict first, then inserts the keywords (CPython update order).
    pub(super) fn lower_dict_kwargs(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if has_starred_arg(c) {
            return Err(parse_error(
                "`*` spreading into dict() is out of scope",
                span,
            ));
        }
        if c.args.is_empty() {
            let mut pairs = Vec::with_capacity(c.keywords.len());
            for kw in &c.keywords {
                let Some(kname) = &kw.arg else {
                    return Err(parse_error("dict() does not support **kwargs", span));
                };
                let key_id = self.intern(kname.as_str());
                let key = self.alloc(HirExprKind::StrLit(key_id), SemTy::Str, span);
                let val = self.lower_expr(&kw.value)?;
                pairs.push((key, val));
            }
            return Ok(self.alloc(HirExprKind::DictLit { pairs }, SemTy::Dyn, span));
        }
        if c.args.len() > 1 {
            return Err(parse_error(
                "dict() takes at most 1 positional argument",
                span,
            ));
        }
        // Stage everything in written order, then build + insert.
        let pos = self.stage_arg(&c.args[0])?;
        let mut kwargs: Vec<(InternedString, ArgSrc)> = Vec::with_capacity(c.keywords.len());
        for kw in &c.keywords {
            let Some(kname) = &kw.arg else {
                return Err(parse_error("dict() does not support **kwargs", span));
            };
            let id = self.intern(kname.as_str());
            let src = self.stage_arg_src(&kw.value)?;
            kwargs.push((id, src));
        }
        let cname = self.intern("dict");
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(cname)),
            SemTy::Dyn,
            span,
        );
        let pos_ref = self.local_ref(pos, span);
        let call = self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![pos_ref],
            },
            SemTy::Dyn,
            span,
        );
        let d = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: d,
            value: call,
        });
        for (key_id, src) in kwargs {
            let key = self.alloc(HirExprKind::StrLit(key_id), SemTy::Str, span);
            let val = self.arg_src_value(src, span)?;
            self.push_stmt(HirStmt::ContainerInsert {
                container: d,
                key,
                value: val,
            });
        }
        Ok(self.local_ref(d, span))
    }

    /// Emit a decorated module-level function's rebinding into `__main__`
    /// (Phase 6D): `slot := dN(…d1(closure(<orig>.<thunk>)))`, decorators
    /// applied innermost-first.
    /// Replay one import statement's precomputed effect (Phase 8): emit the
    /// module-`<init>` calls (execute-once on first import) followed by the
    /// `from M import VAR` snapshot copies.
    pub(super) fn emit_import_action(&mut self, action: &ImportAction) {
        let span = Span::dummy();
        for name in &action.init_calls {
            let callee = self.alloc(
                HirExprKind::Name(SymbolRef::Unresolved(*name)),
                SemTy::Dyn,
                span,
            );
            let call = self.alloc(
                HirExprKind::Call {
                    callee,
                    args: vec![],
                },
                SemTy::NoneTy,
                span,
            );
            self.push_stmt(HirStmt::Expr(call));
        }
        for (dst, src) in &action.snapshots {
            let val = self.alloc(HirExprKind::GlobalGet { var_id: *src }, SemTy::Dyn, span);
            self.push_stmt(HirStmt::GlobalSet {
                var_id: *dst,
                value: val,
            });
        }
    }

    /// Emit the once-eval `GlobalSet`s for a top-level def's non-literal (slot)
    /// parameter defaults at its module-init position. Each default expression is
    /// lowered in module scope (names resolve to module globals; there are no
    /// enclosing locals — which is why this is top-level-only and free of the
    /// free-var-capture trap) and stored into its synthetic global slot, so every
    /// defaulted call reads the same shared object (CPython aliasing semantics).
    pub(super) fn emit_default_slots(&mut self, f: &StmtFunctionDef, slots: &DefaultSlotMap) -> Result<()> {
        let fname = self.intern(f.name.as_str());
        let args = f.args.as_ref();
        for awd in args
            .posonlyargs
            .iter()
            .chain(args.args.iter())
            .chain(args.kwonlyargs.iter())
        {
            let Some(default_expr) = &awd.default else {
                continue;
            };
            let pname = self.intern(awd.def.arg.as_str());
            if let Some(&var_id) = slots.get(&(fname, pname)) {
                let value = self.lower_expr(default_expr)?;
                self.push_stmt(HirStmt::GlobalSet { var_id, value });
            }
        }
        Ok(())
    }

    /// §6: the method analogue of [`Self::emit_default_slots`] — evaluate a
    /// class's methods' non-literal parameter defaults ONCE at the class's
    /// module-init position (CPython def-time once-evaluation), storing each
    /// shared object into its synthetic global slot. Keyed by the SYNTHETIC
    /// method name (`Counter.__init__`), matching the slot-collection pass.
    pub(super) fn emit_class_default_slots(
        &mut self,
        cdef: &StmtClassDef,
        slots: &DefaultSlotMap,
    ) -> Result<()> {
        for stmt in &cdef.body {
            let Stmt::FunctionDef(m) = stmt else { continue };
            let synthetic = format!(
                "{}.{}{}",
                cdef.name.as_str(),
                m.name.as_str(),
                method_synthetic_suffix(m)
            );
            let fname = self.intern(&synthetic);
            let margs = m.args.as_ref();
            for awd in margs
                .posonlyargs
                .iter()
                .chain(margs.args.iter())
                .chain(margs.kwonlyargs.iter())
            {
                let Some(default_expr) = &awd.default else {
                    continue;
                };
                let pname = self.intern(awd.def.arg.as_str());
                if let Some(&var_id) = slots.get(&(fname, pname)) {
                    let value = self.lower_expr(default_expr)?;
                    self.push_stmt(HirStmt::GlobalSet { var_id, value });
                }
            }
        }
        Ok(())
    }

    /// Apply a class's decorators (§5) at module-init, over the class-id int —
    /// for their side effects (a decorator that runs for effect — increments a
    /// counter, appends a marker — and returns the class). The class name stays
    /// bound to its class id via the static `class_map`, so `C(...)` still
    /// constructs; a decorator that returns a *different* class, or stores the
    /// class as a value, is out of scope (classes aren't first-class values yet)
    /// — the chain's result is evaluated for effect and discarded.
    pub(super) fn emit_class_decorators(&mut self, cdef: &StmtClassDef) -> Result<()> {
        if cdef.decorator_list.is_empty() {
            return Ok(());
        }
        let span = to_span(cdef.range());
        let Some((class_id, _)) = self.ctx.class_map.get(cdef.name.as_str()).copied() else {
            return Err(parse_error(
                "internal: decorated class missing from class_map",
                span,
            ));
        };
        // The "class value" passed to the decorator is the class-id int (the same
        // convention as a `@classmethod`'s `cls` and `object.__new__(cls)`).
        let mut v = self.alloc(HirExprKind::IntLit(class_id.0 as i64), SemTy::Int, span);
        // Innermost decorator first (CPython applies bottom-up).
        for deco in cdef.decorator_list.iter().rev() {
            // `@runtime_checkable` (typing) is a no-op marker on a `Protocol` — it
            // only enables `isinstance` against the protocol, which this compiler
            // supports structurally regardless. Emit no runtime call.
            if matches!(deco, Expr::Name(n) if n.id.as_str() == "runtime_checkable") {
                continue;
            }
            v = self.apply_decorator(deco, v, span)?;
        }
        self.push_stmt(HirStmt::Expr(v));
        Ok(())
    }

    pub(super) fn emit_decorated_rebinding(
        &mut self,
        f: &StmtFunctionDef,
        thunk_fid: FuncId,
        slot: u32,
    ) -> Result<()> {
        let span = to_span(f.range());
        let mut v = self.alloc(
            HirExprKind::MakeClosure {
                func: thunk_fid,
                captures: vec![],
            },
            SemTy::Dyn,
            span,
        );
        for deco in f.decorator_list.iter().rev() {
            v = self.apply_decorator(deco, v, span)?;
        }
        self.push_stmt(HirStmt::GlobalSet {
            var_id: slot,
            value: v,
        });
        Ok(())
    }

    /// Apply one decorator expression to a value (Phase 6D): `deco(v)`. The
    /// decorator is lowered as an ordinary value (a top-level function → its
    /// thunk; a factory `@repeat(3)` → the call result), so the application is a
    /// uniform indirect call.
    pub(super) fn apply_decorator(
        &mut self,
        deco: &Expr,
        v: Idx<HirExpr>,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let dval = self.lower_expr(deco)?;
        Ok(self.alloc(
            HirExprKind::Call {
                callee: dval,
                args: vec![v],
            },
            SemTy::Dyn,
            span,
        ))
    }

}

impl<'a> FnLowerer<'a> {
    pub(super) fn lower_direct_known_call(
        &mut self,
        info: &TopDefInfo,
        fname: &str,
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        // Classify positional args. A `*list` / `*tuple` LITERAL spread has a
        // compile-time-known arity, so its elements flatten into plain
        // positionals and reuse the slot-matching path below. A runtime `*seq`
        // spread (a variable / call result / comprehension) has an unknown
        // length, so it routes to the general runtime-spread path.
        let (items, has_runtime_spread) = classify_pos_args(c);
        let has_kw_spread = has_doublestar_kwarg(c);
        if has_runtime_spread {
            if has_kw_spread {
                return Err(parse_error(
                    "combining a runtime `*` spread with `**` unpacking in one call \
                     is out of scope",
                    span,
                ));
            }
            return self.lower_spread_call(info, fname, &items, c, span);
        }

        // With keywords present, slot matching below reorders arguments — so
        // pass 1 stages every (non-literal) argument value in WRITTEN order
        // (CPython's evaluation order), and pass 2 only assembles slot refs.
        let staging = !c.keywords.is_empty();
        let mut positionals: Vec<ArgSrc> = Vec::with_capacity(items.len());
        for item in &items {
            let PosItem::Plain(e) = *item else {
                unreachable!("runtime spreads handled above")
            };
            positionals.push(if staging {
                self.stage_arg_src(e)?
            } else {
                ArgSrc::Plain(e)
            });
        }
        // (keyword name, value, consumed?). Explicit keywords and the entries of
        // a `**{literal}` dict (string-literal keys, known at compile time) flatten
        // into this list — a duplicate across either source is a clean error
        // (`got multiple values for keyword argument`). A non-literal `**d`
        // (a variable / call result / comprehension) is evaluated ONCE into
        // `kw_dict` and bound per parameter by name at run time.
        let mut keywords: Vec<(String, ArgSrc, bool)> = Vec::new();
        let mut kw_dict: Option<LocalId> = None;
        for kw in &c.keywords {
            match &kw.arg {
                Some(name) => {
                    let src = self.stage_arg_src(&kw.value)?;
                    push_call_keyword(&mut keywords, name.as_str(), src, fname, span)?;
                }
                None => match literal_kwargs_dict(&kw.value) {
                    // `**{"a": 1, "b": 2}` — keys known now; flatten to keywords.
                    Some(entries) => {
                        for (k, ve) in entries {
                            let src = self.stage_arg_src(ve)?;
                            push_call_keyword(&mut keywords, &k, src, fname, span)?;
                        }
                    }
                    // `**d` — a runtime dict, bound by name per slot below.
                    None => {
                        if info.kwargs.is_some() {
                            return Err(parse_error(
                                format!(
                                    "`{fname}()` — a runtime `**dict` spread into a function \
                                     with `**kwargs` is out of scope"
                                ),
                                span,
                            ));
                        }
                        if kw_dict.is_some() {
                            return Err(parse_error(
                                "multiple runtime `**dict` spreads in one call is out of scope",
                                span,
                            ));
                        }
                        kw_dict = Some(self.stage_arg(&kw.value)?);
                    }
                },
            }
        }

        let n_fixed = info.fixed.len();
        let mut out: Vec<Idx<HirExpr>> = Vec::with_capacity(n_fixed + 2);

        // ── fixed positional / keyword / default slot matching ──
        let star_tuple: Option<Idx<HirExpr>> = {
            let n_pos = positionals.len();
            if n_pos > n_fixed && info.varargs.is_none() {
                return Err(parse_error(
                    format!(
                        "`{fname}()` takes {n_fixed} positional argument(s) but {n_pos} were given"
                    ),
                    span,
                ));
            }
            let pos_for_fixed = n_pos.min(n_fixed);
            for (i, p) in info.fixed.iter().enumerate() {
                let v = if i < pos_for_fixed {
                    self.arg_src_value(positionals[i], span)?
                } else if let Some(kv) = take_keyword(&mut keywords, self.interner.resolve(p.name))
                {
                    self.arg_src_value(kv, span)?
                } else if let Some(v) = self.bind_from_kw_dict(kw_dict, p, span) {
                    v
                } else if let Some(def) = &p.default {
                    self.lower_param_default(def, span)
                } else {
                    return Err(parse_error(
                        format!(
                            "`{fname}()` missing required argument `{}`",
                            self.interner.resolve(p.name)
                        ),
                        span,
                    ));
                };
                out.push(v);
            }
            if info.varargs.is_some() {
                let mut excess = Vec::new();
                for p in positionals
                    .iter()
                    .skip(n_fixed)
                    .copied()
                    .collect::<Vec<_>>()
                {
                    excess.push(self.arg_src_value(p, span)?);
                }
                Some(self.alloc(HirExprKind::TupleLit { elems: excess }, SemTy::Dyn, span))
            } else {
                None
            }
        };

        // ── keyword-only params ──
        for p in &info.kwonly {
            let v = if let Some(kv) = take_keyword(&mut keywords, self.interner.resolve(p.name)) {
                self.arg_src_value(kv, span)?
            } else if let Some(v) = self.bind_from_kw_dict(kw_dict, p, span) {
                v
            } else if let Some(def) = &p.default {
                self.lower_param_default(def, span)
            } else {
                return Err(parse_error(
                    format!(
                        "`{fname}()` missing required keyword-only argument `{}`",
                        self.interner.resolve(p.name)
                    ),
                    span,
                ));
            };
            out.push(v);
        }

        // ── *args tuple slot ──
        if info.varargs.is_some() {
            match star_tuple {
                Some(t) => out.push(t),
                None => {
                    out.push(self.alloc(HirExprKind::TupleLit { elems: vec![] }, SemTy::Dyn, span))
                }
            }
        }

        // ── **kwargs dict slot: leftover keywords (source order) ──
        if info.kwargs.is_some() {
            let mut pairs = Vec::new();
            // Re-borrow names first to avoid a borrow conflict with lower_expr.
            let leftover: Vec<(InternedString, ArgSrc)> = keywords
                .iter()
                .filter(|(_, _, used)| !*used)
                .map(|(name, v, _)| (self.interner.intern(name), *v))
                .collect();
            for (key_id, v) in leftover {
                let key = self.alloc(HirExprKind::StrLit(key_id), SemTy::Str, span);
                let val = self.arg_src_value(v, span)?;
                pairs.push((key, val));
            }
            out.push(self.alloc(HirExprKind::DictLit { pairs }, SemTy::Dyn, span));
        } else if let Some((name, _, _)) = keywords.iter().find(|(_, _, used)| !*used) {
            return Err(parse_error(
                format!("`{fname}()` got an unexpected keyword argument `{name}`"),
                span,
            ));
        }

        let target = self.intern(fname);
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(target)),
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(HirExprKind::Call { callee, args: out }, SemTy::Dyn, span))
    }

    /// Bind parameter `p` from a runtime `**dict` spread (`kw_dict`), if present:
    /// a defaulted parameter reads `dict.get(name, default)` (the default fills an
    /// absent key); a required parameter reads `dict[name]`. Returns `None` when
    /// there is no runtime dict, so the caller falls through to the literal default
    /// (or the missing-argument error). Documented gaps (the static callee shape
    /// can't see a runtime dict's contents at compile time): an unexpected key in
    /// the dict is not diagnosed, and a key that collides with an explicit
    /// positional/keyword is not detected as a duplicate — the corpus never
    /// exercises either (its dicts match the parameter names exactly and avoid
    /// conflicts; see `corpus/test_functions.py`).
    pub(super) fn bind_from_kw_dict(
        &mut self,
        kw_dict: Option<LocalId>,
        p: &ParamInfo,
        span: Span,
    ) -> Option<Idx<HirExpr>> {
        let dvar = kw_dict?;
        let key = self.alloc(HirExprKind::StrLit(p.name), SemTy::Str, span);
        let dref = self.local_ref(dvar, span);
        let node = match &p.default {
            Some(def) => {
                let default = self.lower_param_default(def, span);
                HirExprKind::ContainerExpr {
                    op: ContainerOp::DictGetDefault,
                    args: vec![dref, key, default],
                }
            }
            None => HirExprKind::ContainerExpr {
                op: ContainerOp::DictGet,
                args: vec![dref, key],
            },
        };
        Some(self.alloc(node, SemTy::Dyn, span))
    }

    /// Lower a known-callee call carrying a runtime `*seq` spread — a sequence
    /// whose length is unknown until run time (`f(*xs)`, `f(a, *xs, b)`,
    /// `f(*xs, *ys)`). The full positional sequence is materialized into a fresh
    /// `argv` list in WRITTEN order ([`Self::build_spread_argv`]), an argument-
    /// count guard runs against the callee's arity, then each parameter slot is
    /// bound by position: required slots read `argv[i]`, defaulted slots read
    /// `argv[i]` when present else the default, a `*args` callee takes
    /// `tuple(argv[n_fixed:])` as its rest tuple. Keyword args are not combined
    /// with a runtime spread (the corpus never does, and it keeps slot matching
    /// simple).
    pub(super) fn lower_spread_call(
        &mut self,
        info: &TopDefInfo,
        fname: &str,
        items: &[PosItem],
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        if !c.keywords.is_empty() {
            return Err(parse_error(
                format!(
                    "`{fname}()`: keyword arguments combined with a runtime `*` spread are out of scope"
                ),
                span,
            ));
        }
        let argv = self.build_spread_argv(items, span)?;
        let n_fixed = info.fixed.len();
        // Required = leading fixed params without a default (Python keeps
        // defaults trailing, so the first defaulted index IS the required count).
        let req = info
            .fixed
            .iter()
            .position(|p| p.default.is_some())
            .unwrap_or(n_fixed);

        // n = len(argv), reused by the count guard and the default-slot tests.
        let n_local = self.fresh_local(SemTy::Int);
        let argv_ref = self.local_ref(argv, span);
        let n_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Len,
                args: vec![argv_ref],
            },
            SemTy::Int,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: n_local,
            value: n_expr,
        });
        let max = if info.varargs.is_some() {
            None
        } else {
            Some(n_fixed)
        };
        self.emit_argcount_check(n_local, req, max, fname, span);

        // Build the param-aligned argument vector (fixed → kw-only → *args tuple
        // → **kwargs dict), matching the callee's MIR parameter order.
        let mut out: Vec<Idx<HirExpr>> = Vec::with_capacity(n_fixed + 2);
        for (i, p) in info.fixed.iter().enumerate() {
            let raw = if i < req {
                // Required: `argv[i]`, in-bounds after the count guard.
                let base = self.local_ref(argv, span);
                let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
                self.alloc(
                    HirExprKind::Subscript { base, index: idx },
                    SemTy::Dyn,
                    span,
                )
            } else {
                let def = p
                    .default
                    .as_ref()
                    .expect("trailing fixed param has a default");
                let default = self.lower_param_default(def, span);
                self.emit_spread_default(argv, n_local, i, default, span)
            };
            let v = self.launder_arg(raw, &p.ty, span);
            out.push(v);
        }
        // Keyword-only params: a `*` spread fills no keywords, so each must carry
        // a default (else the call cannot be satisfied).
        for p in &info.kwonly {
            let Some(def) = &p.default else {
                return Err(parse_error(
                    format!(
                        "`{fname}()` keyword-only parameter `{}` cannot be filled from a `*` spread",
                        self.interner.resolve(p.name)
                    ),
                    span,
                ));
            };
            let d = self.lower_param_default(def, span);
            let v = self.launder_arg(d, &p.ty, span);
            out.push(v);
        }
        // `*args` rest tuple = `tuple(argv[n_fixed:])`.
        if info.varargs.is_some() {
            let base = self.local_ref(argv, span);
            let start = self.alloc(HirExprKind::IntLit(n_fixed as i64), SemTy::Int, span);
            let slice = self.alloc(
                HirExprKind::Slice {
                    base,
                    start: Some(start),
                    end: None,
                    step: None,
                },
                SemTy::list_of(SemTy::Dyn),
                span,
            );
            let it = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::Iter,
                    args: vec![slice],
                },
                SemTy::Dyn,
                span,
            );
            let rest = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::TupleFromIter,
                    args: vec![it],
                },
                SemTy::tuple_var_of(SemTy::Dyn),
                span,
            );
            out.push(rest);
        }
        // `**kwargs` dict slot: a `*` spread supplies no keywords → empty.
        if info.kwargs.is_some() {
            out.push(self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span));
        }

        let target = self.intern(fname);
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(target)),
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(HirExprKind::Call { callee, args: out }, SemTy::Dyn, span))
    }

    /// Materialize the full positional sequence of a `*`-spread call into a fresh
    /// `list[Dyn]` local, evaluating each item in WRITTEN (left-to-right) order:
    /// a plain arg is appended once; a `*seq` spread is iterated (the iterator
    /// protocol, so any iterable — list / tuple / deque / generator / range —
    /// works) and each element appended.
    pub(super) fn build_spread_argv(&mut self, items: &[PosItem], span: Span) -> Result<LocalId> {
        let argv = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: argv,
            value: empty,
        });
        for item in items {
            match *item {
                PosItem::Plain(e) => {
                    let v = self.lower_expr(e)?;
                    self.push_stmt(HirStmt::ContainerPush {
                        container: argv,
                        value: v,
                    });
                }
                PosItem::Spread(e) => {
                    let src = self.lower_expr(e)?;
                    let lp = self.begin_iter_loop(src, span)?;
                    let elem = self.local_ref(lp.elem, span);
                    self.push_stmt(HirStmt::ContainerPush {
                        container: argv,
                        value: elem,
                    });
                    self.end_iter_loop(lp);
                }
            }
        }
        Ok(argv)
    }

    /// `(i < n) ? argv[i] : default` — the value for a defaulted fixed slot under
    /// a runtime spread, as a short-circuit CFG ternary (`argv[i]` is only read
    /// on the in-bounds arm). Returns a read of the result local.
    pub(super) fn emit_spread_default(
        &mut self,
        argv: LocalId,
        n_local: LocalId,
        i: usize,
        default: Idx<HirExpr>,
        span: Span,
    ) -> Idx<HirExpr> {
        let res = self.fresh_local(SemTy::Dyn);
        let i_lit = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
        let n_ref = self.local_ref(n_local, span);
        let cond = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Lt,
                l: i_lit,
                r: n_ref,
            },
            SemTy::Bool,
            span,
        );
        let then_b = self.new_block();
        let else_b = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: then_b,
            else_: else_b,
        });
        self.switch(then_b);
        let base = self.local_ref(argv, span);
        let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
        let av = self.alloc(
            HirExprKind::Subscript { base, index: idx },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: av,
        });
        self.seal(HirTerminator::Jump(join));
        self.switch(else_b);
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: default,
        });
        self.seal(HirTerminator::Jump(join));
        self.switch(join);
        self.local_ref(res, span)
    }

    /// A spread value reaches a fixed / kw-only slot as a gradual `Dyn` (it came
    /// from a runtime `argv` subscript). `int` (Tagged), `str` / containers
    /// (gradual `Heap`), and `Dyn` params admit it directly. A `float` / `bool`
    /// param reinterprets its bits by the annotated type (PITFALLS A2), so typeck
    /// rejects a `Dyn` there — launder the value through a `pin_tagged`
    /// authoritative-typed local: typeck sees the param type (the `pin_tagged`
    /// store skips the reinterpret check), and lowering unboxes the Tagged value
    /// to the param's `Raw` repr at the call.
    pub(super) fn launder_arg(&mut self, value: Idx<HirExpr>, param_ty: &SemTy, span: Span) -> Idx<HirExpr> {
        if !matches!(param_ty, SemTy::Float | SemTy::Bool) {
            return value;
        }
        let slot = self.fresh_local_pinned(param_ty.clone());
        self.push_stmt(HirStmt::Assign {
            target: slot,
            value,
        });
        self.local_ref(slot, span)
    }

    /// Bind a gradual (`Dyn`) argument into a `float` / `bool` parameter slot
    /// through the Phase-1 **checked** unbox (`rt_unbox_float` / `rt_unbox_bool`),
    /// for the uniform-thunk arg→param bind where the value comes from the
    /// untyped `__args__` tuple. Unlike [`Self::launder_arg`] (a `pin_tagged`
    /// slot → unchecked `UnboxFloat`/`UntagBool` at the call coercion), this
    /// stores into a *real* annotated `float`/`bool` local: the annotated-local
    /// `Assign` seam routes the `Dyn → Raw(F64/I8)` store through `coerce_value`,
    /// which emits the **checked** unbox (TypeError on a wrong tag, never SEGV) —
    /// the soundness crux of the uniform value-call convention. `int` / `str` /
    /// container / `Dyn` params keep the value as-is (the existing tagged /
    /// gradual-heap seams).
    pub(super) fn bind_arg_checked(
        &mut self,
        value: Idx<HirExpr>,
        param_ty: &SemTy,
        span: Span,
    ) -> Idx<HirExpr> {
        if !matches!(param_ty, SemTy::Float | SemTy::Bool) {
            return value;
        }
        let slot = self.fresh_local(param_ty.clone());
        self.push_stmt(HirStmt::Assign {
            target: slot,
            value,
        });
        self.local_ref(slot, span)
    }

    /// Emit the body of a uniform thunk (params already installed as
    /// `env`/`__args__`/`__kwargs__`): bind the packed positional tuple to `F`'s
    /// parameters and make ONE direct call to `F`, returning its (boxed) result.
    /// `F`'s value-call binding mirrors the `*seq`-spread slot matching
    /// ([`Self::lower_spread_call`]) but sources from the `__args__` tuple param,
    /// uses the **checked** float/bool unbox, and forwards `env` first for nested
    /// targets. See [`build_uniform_thunk`].
    pub(super) fn emit_uniform_dispatch(
        &mut self,
        target: &UniformTarget,
        fname: &str,
        span: Span,
    ) -> Result<()> {
        // Re-type the `Dyn` `__args__` param (param 1, repr `Tagged`) into a tuple
        // local so subscript / len / slice have a concrete container type. The
        // runtime value IS a tuple (the indirect-call site packs one), so the
        // gradual `Tagged → tuple` retype is the existing sound seam.
        let args_t = self.fresh_local(SemTy::tuple_var_of(SemTy::Dyn));
        let args_param = self.local_ref(LocalId::new(1), span);
        self.push_stmt(HirStmt::Assign {
            target: args_t,
            value: args_param,
        });

        let n_fixed = target.fixed.len();
        // Required = leading fixed params without a default (Python keeps defaults
        // trailing, so the first defaulted index IS the required count).
        let req = target
            .fixed
            .iter()
            .position(|p| p.default.is_some())
            .unwrap_or(n_fixed);

        // n = len(__args__): the arity guard + default-slot tests read it.
        let n_local = self.fresh_local(SemTy::Int);
        let args_len_recv = self.local_ref(args_t, span);
        let n_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Len,
                args: vec![args_len_recv],
            },
            SemTy::Int,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: n_local,
            value: n_expr,
        });
        let max = if target.varargs { None } else { Some(n_fixed) };
        // A method thunk binds fixed params from `__kwargs__` too, so a keyword
        // may legitimately fill a "required" slot (`obj.m(other=5)`); drop the
        // positional lower-bound guard there (a truly-missing required param
        // still raises at its per-param `__kwargs__` read). The upper-bound (too
        // many positional) guard stays. Value-call thunks keep the strict guard.
        let min = if target.kw_bindable { 0 } else { req };
        self.emit_argcount_check(n_local, min, max, fname, span);

        // Materialize a real keyword dict ONLY when `F` consumes keywords
        // (keyword-only or `**kwargs`). The indirect call site passes the null
        // sentinel for `__kwargs__` on the common (no-keyword) path, so normalize
        // it: `kd = {}; kd.update(__kwargs__)`. `rt_dict_update` is null-tolerant (a
        // null `other` is a no-op), so a null `__kwargs__` yields a fresh EMPTY
        // dict and a real keyword dict is copied in — kwonly `dict.get` /
        // `**kwargs` forwarding then never dereferences the null sentinel.
        let kwargs_dict = if !target.kwonly.is_empty() || target.kwargs || target.kw_bindable {
            let kd = self.fresh_local(SemTy::dict_of(SemTy::Str, SemTy::Dyn));
            let fresh = self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
            self.push_stmt(HirStmt::Assign {
                target: kd,
                value: fresh,
            });
            let kd_ref = self.local_ref(kd, span);
            let kparam = self.local_ref(LocalId::new(2), span);
            let upd = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::DictUpdate,
                    args: vec![kd_ref, kparam],
                },
                SemTy::NoneTy,
                span,
            );
            self.push_stmt(HirStmt::Expr(upd));
            Some(kd)
        } else {
            None
        };

        // Build the param-aligned argument vector matching `F`'s MIR parameter
        // order: [env?] → fixed → keyword-only → *args tuple → **kwargs dict.
        let mut out: Vec<Idx<HirExpr>> = Vec::new();
        if target.pass_env {
            out.push(self.local_ref(LocalId::new(0), span));
        }
        for (i, p) in target.fixed.iter().enumerate() {
            let raw = if target.kw_bindable {
                // A method's fixed param may be passed positionally OR by keyword
                // (`obj.m(a, scale=2)`): bind `__args__[i]` when present, else
                // `__kwargs__[name]` (with the param's default, or raising on a
                // truly-missing required param). `bind_from_kw_dict` reads the
                // already-materialized keyword dict by name.
                let pinfo = ParamInfo {
                    name: p.name,
                    ty: p.ty.clone(),
                    default: p.default.clone(),
                };
                let kw_or_default = self
                    .bind_from_kw_dict(kwargs_dict, &pinfo, span)
                    .expect("kw_bindable materializes the keyword dict");
                self.emit_spread_default(args_t, n_local, i, kw_or_default, span)
            } else if i < req {
                let base = self.local_ref(args_t, span);
                let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
                self.alloc(
                    HirExprKind::Subscript { base, index: idx },
                    SemTy::Dyn,
                    span,
                )
            } else {
                let def = p
                    .default
                    .as_ref()
                    .expect("trailing fixed param has a default");
                let default = self.lower_param_default(def, span);
                self.emit_spread_default(args_t, n_local, i, default, span)
            };
            out.push(self.bind_arg_checked(raw, &p.ty, span));
        }
        for p in &target.kwonly {
            let pinfo = ParamInfo {
                name: p.name,
                ty: p.ty.clone(),
                default: p.default.clone(),
            };
            let raw = self
                .bind_from_kw_dict(kwargs_dict, &pinfo, span)
                .expect("kwargs_dict is Some when keyword-only params are present");
            out.push(self.bind_arg_checked(raw, &p.ty, span));
        }
        if target.varargs {
            let base = self.local_ref(args_t, span);
            let start = self.alloc(HirExprKind::IntLit(n_fixed as i64), SemTy::Int, span);
            let slice = self.alloc(
                HirExprKind::Slice {
                    base,
                    start: Some(start),
                    end: None,
                    step: None,
                },
                SemTy::tuple_var_of(SemTy::Dyn),
                span,
            );
            let it = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::Iter,
                    args: vec![slice],
                },
                SemTy::Dyn,
                span,
            );
            let rest = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::TupleFromIter,
                    args: vec![it],
                },
                SemTy::tuple_var_of(SemTy::Dyn),
                span,
            );
            out.push(rest);
        }
        // `**kwargs` slot: forward the whole `__kwargs__` dict (the value-call
        // path supplies no separate keywords, so leftover == the full dict).
        if target.kwargs {
            out.push(self.local_ref(
                kwargs_dict.expect("kwargs_dict is Some when **kwargs is present"),
                span,
            ));
        }

        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(target.name)),
            SemTy::Dyn,
            span,
        );
        // `F`'s declared return type — typeck re-derives it from the resolved
        // callee, and the return terminator boxes it to the thunk's `Dyn` (Tagged)
        // result regardless.
        let call = self.alloc(
            HirExprKind::Call { callee, args: out },
            target.ret.clone(),
            span,
        );
        self.seal(HirTerminator::Return(Some(call)));
        Ok(())
    }

    /// Emit the argument-count guards for a runtime spread: too few values
    /// (`len(argv) < min`) and, for a non-`*args` callee, too many
    /// (`len(argv) > max`). Each raises `TypeError`, matching CPython's
    /// wrong-arity behavior (the success path never trips them).
    pub(super) fn emit_argcount_check(
        &mut self,
        n_local: LocalId,
        min: usize,
        max: Option<usize>,
        fname: &str,
        span: Span,
    ) {
        if min > 0 {
            self.emit_count_guard(
                n_local,
                CmpOp::Lt,
                min,
                format!("`{fname}()` missing required positional argument(s) (too few values to spread)"),
                span,
            );
        }
        if let Some(max) = max {
            self.emit_count_guard(
                n_local,
                CmpOp::Gt,
                max,
                format!("`{fname}()` takes {max} positional argument(s) but more were spread"),
                span,
            );
        }
    }

    /// `if (n <op> bound): raise TypeError(msg)` — one arity guard for a runtime
    /// spread. Mirrors the `assert … , msg` desugar (branch → raise →
    /// `Unreachable`), then continues in the pass block.
    pub(super) fn emit_count_guard(
        &mut self,
        n_local: LocalId,
        op: CmpOp,
        bound: usize,
        msg: String,
        span: Span,
    ) {
        let n_ref = self.local_ref(n_local, span);
        let b = self.alloc(HirExprKind::IntLit(bound as i64), SemTy::Int, span);
        let cond = self.alloc(
            HirExprKind::Compare { op, l: n_ref, r: b },
            SemTy::Bool,
            span,
        );
        let fail = self.new_block();
        let ok = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: fail,
            else_: ok,
        });
        self.switch(fail);
        let msg_id = self.intern(&msg);
        let m = self.alloc(HirExprKind::StrLit(msg_id), SemTy::Str, span);
        self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
            tag: pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
            msg: Some(m),
        }));
        self.seal(HirTerminator::Unreachable);
        self.switch(ok);
    }

    /// Lower an indirect / unknown-callee call. A **simple positional** call
    /// (`f(a, b)`) lowers to a flat [`HirExprKind::Call`] whose args lowering packs
    /// into the uniform `(args_tuple, kwargs) → Value` ABI when the callee resolves
    /// to a value (or passes individually when it resolves to a function/builtin —
    /// the resolution-agnostic flat form). A `*seq` spread, a `**dict` forward, or
    /// **named keyword args** cannot be expressed as flat args, so they pre-build
    /// the positional `tuple` + keyword `dict` and emit a [`HirExprKind::CallValue`]
    /// handed straight to the closure ABI. Keywords reach the callee's keyword-only
    /// / `**kwargs` parameters (bound by name in its uniform thunk); the closure's
    /// own positional params are still matched positionally (binding a positional
    /// param BY keyword through a value call stays out of scope).
    pub(super) fn lower_indirect_or_unknown_call(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        let callee = self.lower_callee(c.func.as_ref())?;
        // No spread and no keyword of any kind → the flat positional form (lowering
        // packs / direct-resolves).
        if !has_starred_arg(c) && c.keywords.is_empty() {
            let mut args = Vec::with_capacity(c.args.len());
            for a in &c.args {
                args.push(self.lower_expr(a)?);
            }
            return Ok(self.alloc(HirExprKind::Call { callee, args }, SemTy::Dyn, span));
        }

        // Pre-pack the positional tuple. A runtime `*seq` spread materializes
        // through `argv`; an all-plain (incl. flattened literal-spread) call builds
        // a fixed tuple literal directly. Positionals evaluate BEFORE keywords.
        let (items, has_runtime_spread) = classify_pos_args(c);
        let args_tuple = if has_runtime_spread {
            let argv = self.build_spread_argv(&items, span)?;
            let argv_ref = self.local_ref(argv, span);
            let it = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::Iter,
                    args: vec![argv_ref],
                },
                SemTy::Dyn,
                span,
            );
            self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::TupleFromIter,
                    args: vec![it],
                },
                SemTy::tuple_var_of(SemTy::Dyn),
                span,
            )
        } else {
            let mut elems = Vec::with_capacity(items.len());
            for item in &items {
                let PosItem::Plain(e) = *item else {
                    unreachable!("runtime spreads handled above")
                };
                elems.push(self.lower_expr(e)?);
            }
            self.alloc(HirExprKind::TupleLit { elems }, SemTy::Dyn, span)
        };
        // Build the keyword dict from named keywords and `**d` forwards (or the
        // null sentinel when there are none).
        let kwargs = self.build_indirect_kwargs(c, span)?;
        Ok(self.alloc(
            HirExprKind::CallValue {
                callee,
                args: args_tuple,
                kwargs,
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// Build the keyword dict for a value-position call from its `key=value` and
    /// `**d` keyword sources, in source order (CPython left-to-right) — `None` when
    /// the call has no keywords (the indirect call site then passes the null
    /// sentinel, no allocation). The dict reaches the callee's keyword-only /
    /// `**kwargs` params via its uniform thunk.
    pub(super) fn build_indirect_kwargs(&mut self, c: &ExprCall, span: Span) -> Result<Option<Idx<HirExpr>>> {
        if c.keywords.is_empty() {
            return Ok(None);
        }
        let kd = self.fresh_local(SemTy::dict_of(SemTy::Str, SemTy::Dyn));
        let empty = self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: kd,
            value: empty,
        });
        for kw in &c.keywords {
            match &kw.arg {
                // `name=value` → insert a single entry.
                Some(name) => {
                    let key_id = self.intern(name.as_str());
                    let key = self.alloc(HirExprKind::StrLit(key_id), SemTy::Str, span);
                    let val = self.lower_expr(&kw.value)?;
                    self.push_stmt(HirStmt::ContainerInsert {
                        container: kd,
                        key,
                        value: val,
                    });
                }
                // `**d` → merge `d`'s entries (CPython update order).
                None => {
                    let other = self.lower_expr(&kw.value)?;
                    let kd_ref = self.local_ref(kd, span);
                    let upd = self.alloc(
                        HirExprKind::ContainerExpr {
                            op: ContainerOp::DictUpdate,
                            args: vec![kd_ref, other],
                        },
                        SemTy::NoneTy,
                        span,
                    );
                    self.push_stmt(HirStmt::Expr(upd));
                }
            }
        }
        Ok(Some(self.local_ref(kd, span)))
    }

    /// Materialize a constant default value (Phase 6C) as a literal expr.
    pub(super) fn lower_param_default(&mut self, init: &ParamDefault, span: Span) -> Idx<HirExpr> {
        let (kind, ty) = match init {
            // A mutable/computed top-level default reads its once-evaluated,
            // GC-rooted global slot (the shared object, CPython aliasing). The
            // tagged `Dyn` read coerces into the param's repr at the call seam.
            ParamDefault::Slot(var_id) => {
                return self.alloc(HirExprKind::GlobalGet { var_id: *var_id }, SemTy::Dyn, span)
            }
            ParamDefault::Const(ClassAttrInit::Int(v)) => (HirExprKind::IntLit(*v), SemTy::Int),
            ParamDefault::Const(ClassAttrInit::BigInt(s)) => {
                (HirExprKind::BigIntLit(*s), SemTy::Int)
            }
            ParamDefault::Const(ClassAttrInit::Float(f)) => {
                (HirExprKind::FloatLit(*f), SemTy::Float)
            }
            ParamDefault::Const(ClassAttrInit::Bool(b)) => (HirExprKind::BoolLit(*b), SemTy::Bool),
            ParamDefault::Const(ClassAttrInit::Str(s)) => (HirExprKind::StrLit(*s), SemTy::Str),
            ParamDefault::Const(ClassAttrInit::Bytes(s)) => {
                (HirExprKind::BytesLit(*s), SemTy::Bytes)
            }
            ParamDefault::Const(ClassAttrInit::None) => (HirExprKind::NoneLit, SemTy::NoneTy),
            // `()` default → a fresh empty tuple (immutable, so per-call freshness
            // matches CPython's shared singleton observably).
            ParamDefault::Const(ClassAttrInit::EmptyTuple) => {
                (HirExprKind::TupleLit { elems: vec![] }, SemTy::Dyn)
            }
        };
        self.alloc(kind, ty, span)
    }

    /// Lower a call's callee. A bare name NOT bound in this scope stays a
    /// `Name` (a direct call resolved by `semantics` — never a value-position
    /// thunk); anything else (closure-typed locals/cells, call results) lowers
    /// normally and the call goes indirect.
    pub(super) fn lower_callee(&mut self, func: &Expr) -> Result<Idx<HirExpr>> {
        if let Expr::Name(n) = func {
            let name = self.intern(n.id.as_str());
            if !self.scope.contains_key(&name) {
                let span = to_span(func.range());
                // A promoted module-global callee (e.g. a decorated top-level
                // function, Phase 6D) reads its slot and calls indirectly.
                if let Some(var_id) = self.global_read_slot(name) {
                    return Ok(self.alloc(HirExprKind::GlobalGet { var_id }, SemTy::Dyn, span));
                }
                return Ok(self.alloc(
                    HirExprKind::Name(SymbolRef::Unresolved(name)),
                    SemTy::Dyn,
                    span,
                ));
            }
        }
        self.lower_expr(func)
    }

    pub(super) fn lower_constant(&mut self, c: &Constant, span: Span) -> Result<Idx<HirExpr>> {
        let (kind, ty) = match c {
            Constant::Str(s) => (HirExprKind::StrLit(self.intern(s)), SemTy::Str),
            Constant::Int(big) => (self.int_literal(&big.to_string(), false), SemTy::Int),
            Constant::Float(f) => (HirExprKind::FloatLit(*f), SemTy::Float),
            Constant::Bool(b) => (HirExprKind::BoolLit(*b), SemTy::Bool),
            Constant::None => (HirExprKind::NoneLit, SemTy::NoneTy),
            // `...` (Ellipsis) appears as a Protocol/stub method body (`def m(self)
            // -> int: ...`). The stub never runs (a protocol receiver is
            // `Dyn`, so calls route through `rt_obj_method`), so lower it to a
            // gradual no-op value — the surrounding expr-statement discards it.
            Constant::Ellipsis => (HirExprKind::NoneLit, SemTy::Dyn),
            Constant::Bytes(b) => {
                // Interned as raw bytes (the interner stores byte blobs, not just
                // UTF-8 `String`s), so non-UTF-8 literals like `b"\xff"` round-trip
                // intact; lowering reads them back via `resolve_bytes`.
                (
                    HirExprKind::BytesLit(self.interner.intern_bytes(b)),
                    SemTy::Bytes,
                )
            }
            _ => {
                return Err(parse_error(
                    "unsupported literal kind for this milestone",
                    span,
                ))
            }
        };
        Ok(self.alloc(kind, ty, span))
    }

    /// Build an int-literal node, choosing the tagged-fixnum or bignum path.
    /// `decimal` is the non-negative magnitude text; `negative` applies a sign.
    pub(super) fn int_literal(&mut self, decimal: &str, negative: bool) -> HirExprKind {
        match decimal.parse::<i64>() {
            Ok(mag) if pyaot_core_defs::int_fits(if negative { -mag } else { mag }) => {
                HirExprKind::IntLit(if negative { -mag } else { mag })
            }
            _ => {
                let text = if negative {
                    format!("-{decimal}")
                } else {
                    decimal.to_string()
                };
                HirExprKind::BigIntLit(self.intern(&text))
            }
        }
    }
}

/// Build the **single uniform thunk** for a callable value (replaces the Phase-6D
/// decorator generic thunk and the Phase-6A top-level-fn-value typed thunk). The
/// thunk is `F.<uniform>(env, __args__, __kwargs__) → Dyn` where `__args__` /
/// `__kwargs__` are `Dyn` (repr `Tagged`, the visible `GENERIC_SIG` shape). Its
/// body binds the packed positional tuple / keyword dict to `F`'s parameters at
/// run time — positional, defaults, `*args` — using the Phase-1 **checked** unbox
/// for `float` / `bool` params, then makes ONE **direct** call to `F` (specialized
/// native ABI, the hot path), forwarding `env` first for nested targets. The
/// result is boxed to `Value` by the return terminator (thunk ret = `Dyn`).
///
/// Keyword-only / `**kwargs` parameters are bound from `__kwargs__`, but a value
/// call never carries keywords in the corpus (the call site passes the null
/// `__kwargs__` sentinel), so that binding is the deferred path — it runs only if
/// such a closure is invoked indirectly. A keyword-bearing closure that is only
/// ever called directly never invokes this thunk, so building it is harmless.
pub(super) fn build_uniform_thunk(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    target: &UniformTarget,
) -> Result<FuncId> {
    let span = Span::dummy();
    let fid = shared.reserve();
    let base = interner.resolve(target.name).to_string();
    let tname = interner.intern(&format!("{base}.<uniform>"));
    let mut fl = FnLowerer::new(interner, ctx, shared, tname, &base, SemTy::Dyn, None);
    let env_name = fl.intern("__env__");
    fl.add_param(env_name, SemTy::Dyn);
    // `__args__` / `__kwargs__` are `Dyn` (repr `Tagged`) so the thunk's visible
    // signature is exactly `GENERIC_SIG` — the verifier's strict closure-sig
    // check then holds with no relaxation.
    let args_name = fl.intern("__args__");
    fl.add_param(args_name, SemTy::Dyn);
    let kwargs_name = fl.intern("__kwargs__");
    fl.add_param(kwargs_name, SemTy::Dyn);
    fl.emit_uniform_dispatch(target, &base, span)?;
    let f = fl.finish(HirTerminator::Return(None));
    // The thunk itself is a plain 3-param function — no `*args`/`**kwargs` ABI.
    shared.fill(fid, f);
    Ok(fid)
}

/// Take (mark consumed) the first unconsumed keyword named `name`, returning its
/// value source (Phase 6C).
pub(super) fn take_keyword<'a>(keywords: &mut [(String, ArgSrc<'a>, bool)], name: &str) -> Option<ArgSrc<'a>> {
    for (k, v, used) in keywords.iter_mut() {
        if !*used && k == name {
            *used = true;
            return Some(*v);
        }
    }
    None
}

/// True iff any positional arg is a `*t` spread.
pub(super) fn has_starred_arg(c: &ExprCall) -> bool {
    c.args.iter().any(|a| matches!(a, Expr::Starred(_)))
}

/// If `e` is a list/tuple LITERAL with no nested `*` element, return its element
/// expressions — a compile-time-known spread (`f(*[1, 2, 3])`) the slot-matching
/// path can flatten into plain positionals. `None` for a runtime sequence (a
/// variable / call result / comprehension), which must spread at runtime.
pub(super) fn flatten_literal_seq(e: &Expr) -> Option<&[Expr]> {
    match e {
        Expr::List(l) if !l.elts.iter().any(|x| matches!(x, Expr::Starred(_))) => Some(&l.elts),
        Expr::Tuple(t) if !t.elts.iter().any(|x| matches!(x, Expr::Starred(_))) => Some(&t.elts),
        _ => None,
    }
}

/// Classify a call's positional args, flattening literal `*` spreads
/// ([`flatten_literal_seq`]) into plain positionals. Returns the ordered items
/// plus whether any RUNTIME `*seq` spread remains (length unknown until run
/// time, so the call routes to the general spread path).
pub(super) fn classify_pos_args(c: &ExprCall) -> (Vec<PosItem<'_>>, bool) {
    let mut items = Vec::with_capacity(c.args.len());
    let mut has_runtime_spread = false;
    for a in &c.args {
        match a {
            Expr::Starred(s) => match flatten_literal_seq(s.value.as_ref()) {
                Some(elts) => items.extend(elts.iter().map(PosItem::Plain)),
                None => {
                    items.push(PosItem::Spread(s.value.as_ref()));
                    has_runtime_spread = true;
                }
            },
            _ => items.push(PosItem::Plain(a)),
        }
    }
    (items, has_runtime_spread)
}

/// True iff the call has a `**d` spread.
pub(super) fn has_doublestar_kwarg(c: &ExprCall) -> bool {
    c.keywords.iter().any(|k| k.arg.is_none())
}

/// If `e` is a dict LITERAL whose every key is a string literal (`{"a": 1}`,
/// the compile-time-known form of a `**{...}` spread), return its `(key, value)`
/// entries in written order. `None` for any non-literal / non-string-keyed dict
/// (a runtime `**d`, or `{**x}` nested unpacking), which binds at run time.
pub(super) fn literal_kwargs_dict(e: &Expr) -> Option<Vec<(String, &Expr)>> {
    let Expr::Dict(d) = e else { return None };
    let mut out = Vec::with_capacity(d.values.len());
    for (k, v) in d.keys.iter().zip(&d.values) {
        let Some(Expr::Constant(c)) = k.as_ref() else {
            return None;
        };
        let Constant::Str(s) = &c.value else {
            return None;
        };
        out.push((s.clone(), v));
    }
    Some(out)
}

/// Append a resolved keyword `(name, value)` to a direct-call's keyword list,
/// rejecting a duplicate name (an explicit keyword colliding with a `**{literal}`
/// entry, or two literal-dict entries) the way CPython does:
/// `got multiple values for keyword argument`.
pub(super) fn push_call_keyword<'a>(
    keywords: &mut Vec<(String, ArgSrc<'a>, bool)>,
    name: &str,
    src: ArgSrc<'a>,
    fname: &str,
    span: Span,
) -> Result<()> {
    if keywords.iter().any(|(n, _, _)| n == name) {
        return Err(parse_error(
            format!("`{fname}()` got multiple values for keyword argument `{name}`"),
            span,
        ));
    }
    keywords.push((name.to_string(), src, false));
    Ok(())
}

pub(super) fn reject_call_extras(c: &ExprCall, span: Span, what: &str) -> Result<()> {
    if !c.keywords.is_empty() {
        return Err(parse_error(
            format!("keyword arguments are not supported for {what}"),
            span,
        ));
    }
    if has_starred_arg(c) {
        return Err(parse_error(
            format!("`*args` spreading is not supported for {what}"),
            span,
        ));
    }
    Ok(())
}

/// True iff `e` is a bare `super()` call (the zero-arg form; Phase 5B). The
/// explicit `super(Cls, self)` form is out of scope.
pub(super) fn is_super_call(e: &Expr) -> bool {
    matches!(e, Expr::Call(c)
        if c.args.is_empty() && c.keywords.is_empty()
            && matches!(c.func.as_ref(), Expr::Name(n) if n.id.as_str() == "super"))
}

