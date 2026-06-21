use super::*;

/// Which display syntax a `*`-spread element appears in (`[…]` / `{…}` / `(…)`).
/// List and Tuple both accumulate into a list first; a tuple is then frozen.
#[derive(Clone, Copy)]
enum DisplayKind {
    List,
    Set,
    Tuple,
}

impl<'a> FnLowerer<'a> {
    // ── expressions ──────────────────────────────────────────────────────────

    pub(super) fn lower_expr(&mut self, expr: &Expr) -> Result<Idx<HirExpr>> {
        let span = to_span(expr.range());
        match expr {
            Expr::Constant(c) => self.lower_constant(&c.value, span),
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                // `NotImplemented` (§4a) resolves to the runtime singleton — the
                // dunder-fallback control-flow signal. Intercepted before the
                // scope lookup (no user shadows the builtin in the corpus) so a
                // dunder body `return NotImplemented` produces the real value.
                if n.id.as_str() == "NotImplemented" && !self.scope.contains_key(&name) {
                    return Ok(self.alloc(
                        HirExprKind::NotImplementedLit,
                        SemTy::NotImplementedT,
                        span,
                    ));
                }
                // Inside a `@classmethod`, a bare `cls` is a compile-time alias of
                // the enclosing class — it resolves exactly like the written class
                // name (`Symbol::Class`), so `cls.attr` / `cls.method(...)` take
                // the standard class-reference paths. A local `cls` shadows it.
                if n.id.as_str() == "cls" && !self.scope.contains_key(&name) {
                    if let Some((_, class_name)) = self.cls_ref {
                        return Ok(self.alloc(
                            HirExprKind::Name(SymbolRef::Unresolved(class_name)),
                            SemTy::Dyn,
                            span,
                        ));
                    }
                }
                // A name the frontend already has in scope resolves directly
                // through its binding (a local read or a `CellGet`); a top-level
                // function used as a VALUE becomes its memoized thunk closure
                // (Phase 6A); everything else defers to `semantics`.
                if let Some(b) = self.scope.get(&name).copied() {
                    Ok(self.read_binding(b, span))
                } else if let Some(var_id) = self.global_read_slot(name) {
                    Ok(self.alloc(HirExprKind::GlobalGet { var_id }, SemTy::Dyn, span))
                } else if self.ctx.top_defs.contains_key(n.id.as_str()) {
                    self.lower_top_fn_value(n.id.as_str(), span)
                } else if let Some(c) = self.ctx.stdlib.consts.get(n.id.as_str()).copied() {
                    // A from-imported stdlib constant (`from math import pi`)
                    // folds to its literal at every use site (Phase 8B). A
                    // Python-value read, so out-of-range ints promote to bignum.
                    Ok(self.lower_stdlib_const(&c.value, span, true))
                } else if let Some(attr) = self.ctx.stdlib.attrs.get(n.id.as_str()).copied() {
                    // A from-imported module attribute (`from sys import argv`).
                    Ok(self.alloc(
                        HirExprKind::CallRuntime {
                            target: pyaot_hir::RuntimeCallTarget::Attr(attr),
                            args: vec![],
                            provided: 0,
                        },
                        SemTy::Dyn,
                        span,
                    ))
                } else {
                    Ok(self.alloc(
                        HirExprKind::Name(SymbolRef::Unresolved(name)),
                        SemTy::Dyn,
                        span,
                    ))
                }
            }
            Expr::Lambda(l) => self.lower_lambda(l, span),
            Expr::UnaryOp(u) => self.lower_unary(u, span),
            Expr::BinOp(b) => self.lower_binop(b, span),
            Expr::Compare(c) => self.lower_compare(c, span),
            Expr::BoolOp(b) => self.lower_boolop(b),
            Expr::IfExp(e) => self.lower_ifexp(e),
            Expr::Call(c) => self.lower_call_expr(c, span),
            // ── containers (Phase 4) ──
            Expr::List(l) => {
                if l.elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    return self.lower_display_with_spread(&l.elts, DisplayKind::List, span);
                }
                let elems = self.lower_expr_list(&l.elts)?;
                Ok(self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span))
            }
            Expr::Tuple(t) => {
                if t.elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    return self.lower_display_with_spread(&t.elts, DisplayKind::Tuple, span);
                }
                let elems = self.lower_expr_list(&t.elts)?;
                Ok(self.alloc(HirExprKind::TupleLit { elems }, SemTy::Dyn, span))
            }
            Expr::Set(s) => {
                if s.elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    return self.lower_display_with_spread(&s.elts, DisplayKind::Set, span);
                }
                let elems = self.lower_expr_list(&s.elts)?;
                Ok(self.alloc(HirExprKind::SetLit { elems }, SemTy::Dyn, span))
            }
            Expr::Dict(d) => {
                // `{**a, **b}` dict-merge (a `None` key marks a `**spread`): build
                // incrementally — insert literal pairs, `DictUpdate` each spread —
                // so later keys override earlier ones (CPython left-to-right). The
                // spread-free fast path stays a flat `DictLit`.
                if d.keys.iter().any(|k| k.is_none()) {
                    return self.lower_dict_with_spread(d, span);
                }
                let mut pairs = Vec::with_capacity(d.values.len());
                for (k, v) in d.keys.iter().zip(d.values.iter()) {
                    let k = k.as_ref().expect("spread-free dict has no `None` keys");
                    let kk = self.lower_expr(k)?;
                    let vv = self.lower_expr(v)?;
                    pairs.push((kk, vv));
                }
                Ok(self.alloc(HirExprKind::DictLit { pairs }, SemTy::Dyn, span))
            }
            Expr::Subscript(s) => self.lower_subscript_expr(s, span),
            Expr::Attribute(a) => {
                // `e.__class__.__name__` (Phase 7B): constant-fold to the bare
                // class name from the variable's static type. (Documented
                // divergence: a base-typed `except` clause folds the static —
                // not dynamic — class name; the corpus only reads exact
                // handler matches.)
                if a.attr.as_str() == "__name__" {
                    if let Expr::Attribute(inner) = a.value.as_ref() {
                        if inner.attr.as_str() == "__class__" {
                            return self.fold_class_name(inner.value.as_ref(), span);
                        }
                    }
                }
                // `M.VAR` / `M.func` through an `import M` alias (Phase 8): a live
                // module-variable read folds to a `GlobalGet` of the exporter's
                // slot; an aliased function used as a value becomes its thunk.
                if let Expr::Name(m) = a.value.as_ref() {
                    if self.ctx.aliases.contains(m.id.as_str()) {
                        let mname = self.intern(m.id.as_str());
                        if !self.scope.contains_key(&mname) {
                            let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                            if let Some(slot) = self.ctx.alias_vars.get(&qual).copied() {
                                return Ok(self.alloc(
                                    HirExprKind::GlobalGet { var_id: slot },
                                    SemTy::Dyn,
                                    span,
                                ));
                            }
                            if self.ctx.top_defs.contains_key(&qual) {
                                return self.lower_top_fn_value(&qual, span);
                            }
                        }
                    }
                    // `M.pi` / `M.argv` through an `import M` stdlib alias
                    // (Phase 8B): a constant folds to its literal; a module
                    // attribute becomes its getter call.
                    if self.ctx.stdlib.aliases.contains(m.id.as_str()) {
                        let mname = self.intern(m.id.as_str());
                        if !self.scope.contains_key(&mname) {
                            let qual = format!("{}.{}", m.id.as_str(), a.attr.as_str());
                            if let Some(c) = self.ctx.stdlib.consts.get(&qual).copied() {
                                return Ok(self.lower_stdlib_const(&c.value, span, true));
                            }
                            if let Some(attr) = self.ctx.stdlib.attrs.get(&qual).copied() {
                                return Ok(self.alloc(
                                    HirExprKind::CallRuntime {
                                        target: pyaot_hir::RuntimeCallTarget::Attr(attr),
                                        args: vec![],
                                        provided: 0,
                                    },
                                    SemTy::Dyn,
                                    span,
                                ));
                            }
                            return Err(parse_error(
                                format!(
                                    "stdlib module `{}` has no attribute `{}`",
                                    m.id.as_str(),
                                    a.attr.as_str()
                                ),
                                span,
                            ));
                        }
                    }
                }
                let value = self.lower_expr(a.value.as_ref())?;
                let name = self.intern(a.attr.as_str());
                Ok(self.alloc(HirExprKind::Attribute { value, name }, SemTy::Dyn, span))
            }
            Expr::ListComp(c) => self.lower_listcomp(c, span),
            Expr::SetComp(c) => self.lower_setcomp(c, span),
            Expr::DictComp(c) => self.lower_dictcomp(c, span),
            Expr::GeneratorExp(g) => self.lower_genexpr(g, span),
            // f-string interpolation (Phase 8B, minimal): each `{expr}` part
            // desugars to `str(expr)` and the parts concatenate left-to-right.
            // Format specs / conversions (`{x:.4f}`, `{x!r}`) are Phase 8E.
            Expr::JoinedStr(j) => self.lower_joined_str(j, span),
            // Walrus / named expression `(target := value)` (PEP 572, §2).
            Expr::NamedExpr(n) => self.lower_named_expr(n, span),
            other => Err(parse_error(
                "unsupported expression for this milestone",
                to_span(other.range()),
            )),
        }
    }

    /// Walrus / named expression `(target := value)` (PEP 572, §2): evaluate
    /// `value` ONCE, bind it to `target` (a bare `Name` per the grammar) in the
    /// containing scope, and evaluate to the assigned value. The binding routes
    /// through the ordinary write/read place machinery (a local, a captured cell,
    /// or a promoted module-global slot — `resolve_write_place`), so a name bound
    /// in an `if`/`while` test is visible after the statement, exactly as CPython.
    /// The write stmt is emitted before the enclosing expression reads the slot,
    /// so the assignment and the expression's value coincide (single evaluation).
    pub(super) fn lower_named_expr(&mut self, n: &ExprNamedExpr, span: Span) -> Result<Idx<HirExpr>> {
        let Expr::Name(target) = n.target.as_ref() else {
            // PEP 572 restricts the target to an identifier; the parser enforces
            // this, but guard defensively rather than mis-lower a non-name.
            return Err(parse_error("walrus target must be a name", span));
        };
        let value = self.lower_expr(n.value.as_ref())?;
        let name = self.intern(target.id.as_str());
        let place = self.resolve_write_place(name, SemTy::Dyn);
        self.write_place(place, value);
        Ok(self.read_place(place, span))
    }

    /// Lower a display (`[…]` / `{…}` / `(…)`) that contains at least one `*`
    /// spread element, by the same machinery as a `*`-spread call argv
    /// ([`Self::build_spread_argv`]): materialize into a fresh container,
    /// `ContainerPush` each plain element, and iterate each `*seq` spread (the
    /// iterator protocol — any iterable works) pushing its elements. List/Set
    /// return the populated local directly; a tuple is built as a list, then
    /// frozen via `Iter` + `TupleFromIter`.
    fn lower_display_with_spread(
        &mut self,
        elts: &[Expr],
        kind: DisplayKind,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let (local, empty) = match kind {
            DisplayKind::Set => {
                let l = self.fresh_local(SemTy::set_of(SemTy::Dyn));
                let e = self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span);
                (l, e)
            }
            // List and Tuple both accumulate into a list first.
            DisplayKind::List | DisplayKind::Tuple => {
                let l = self.fresh_local(SemTy::list_of(SemTy::Dyn));
                let e = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
                (l, e)
            }
        };
        self.push_stmt(HirStmt::Assign {
            target: local,
            value: empty,
        });
        for elt in elts {
            match elt {
                Expr::Starred(s) => {
                    let src = self.lower_expr(s.value.as_ref())?;
                    let lp = self.begin_iter_loop(src, span)?;
                    let elem = self.local_ref(lp.elem, span);
                    self.push_stmt(HirStmt::ContainerPush {
                        container: local,
                        value: elem,
                    });
                    self.end_iter_loop(lp);
                }
                _ => {
                    let v = self.lower_expr(elt)?;
                    self.push_stmt(HirStmt::ContainerPush {
                        container: local,
                        value: v,
                    });
                }
            }
        }
        match kind {
            DisplayKind::List | DisplayKind::Set => Ok(self.local_ref(local, span)),
            DisplayKind::Tuple => {
                let list_ref = self.local_ref(local, span);
                let it = self.alloc(
                    HirExprKind::ContainerExpr {
                        op: ContainerOp::Iter,
                        args: vec![list_ref],
                    },
                    SemTy::Dyn,
                    span,
                );
                Ok(self.alloc(
                    HirExprKind::ContainerExpr {
                        op: ContainerOp::TupleFromIter,
                        args: vec![it],
                    },
                    SemTy::tuple_var_of(SemTy::Dyn),
                    span,
                ))
            }
        }
    }

    /// Lower a dict display containing a `**spread` (`{**a, "k": v, **b}`),
    /// mirroring [`build_indirect_kwargs`]: start from an empty dict, then walk
    /// the entries in source order — a literal `key: value` is a
    /// [`HirStmt::ContainerInsert`], a `**other` spread a `DictUpdate` of
    /// `other`'s entries. Left-to-right order gives CPython's "later keys win".
    fn lower_dict_with_spread(&mut self, d: &ExprDict, span: Span) -> Result<Idx<HirExpr>> {
        let local = self.fresh_local(SemTy::dict_of(SemTy::Dyn, SemTy::Dyn));
        let empty = self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: local,
            value: empty,
        });
        for (k, v) in d.keys.iter().zip(d.values.iter()) {
            match k {
                // `key: value` → insert a single entry.
                Some(key) => {
                    let key = self.lower_expr(key)?;
                    let value = self.lower_expr(v)?;
                    self.push_stmt(HirStmt::ContainerInsert {
                        container: local,
                        key,
                        value,
                    });
                }
                // `**other` → merge `other`'s entries (CPython update order).
                None => {
                    let other = self.lower_expr(v)?;
                    let local_ref = self.local_ref(local, span);
                    let upd = self.alloc(
                        HirExprKind::ContainerExpr {
                            op: ContainerOp::DictUpdate,
                            args: vec![local_ref, other],
                        },
                        SemTy::NoneTy,
                        span,
                    );
                    self.push_stmt(HirStmt::Expr(upd));
                }
            }
        }
        Ok(self.local_ref(local, span))
    }

    /// Lower a list of expressions (literal elements).
    pub(super) fn lower_expr_list(&mut self, exprs: &[Expr]) -> Result<Vec<Idx<HirExpr>>> {
        exprs.iter().map(|e| self.lower_expr(e)).collect()
    }

    /// Lower a subscript read `value[index]`, or a slice `value[a:b:c]`
    /// (Phase 8E) when the index is a slice expression.
    pub(super) fn lower_subscript_expr(&mut self, s: &ExprSubscript, span: Span) -> Result<Idx<HirExpr>> {
        if let Expr::Slice(sl) = s.slice.as_ref() {
            let base = self.lower_expr(s.value.as_ref())?;
            let lower_opt =
                |this: &mut Self, e: &Option<Box<Expr>>| -> Result<Option<Idx<HirExpr>>> {
                    match e {
                        Some(x) => Ok(Some(this.lower_expr(x.as_ref())?)),
                        None => Ok(None),
                    }
                };
            let start = lower_opt(self, &sl.lower)?;
            let end = lower_opt(self, &sl.upper)?;
            let step = lower_opt(self, &sl.step)?;
            // The result kind mirrors the base's static type; typeck assigns it.
            return Ok(self.alloc(
                HirExprKind::Slice {
                    base,
                    start,
                    end,
                    step,
                },
                SemTy::Dyn,
                span,
            ));
        }
        let base = self.lower_expr(s.value.as_ref())?;
        let index = self.lower_expr(s.slice.as_ref())?;
        Ok(self.alloc(HirExprKind::Subscript { base, index }, SemTy::Dyn, span))
    }

    // ── comprehensions (Phase 4C) ──────────────────────────────────────────────

    /// f-string lowering (§13). Literal parts are `StrLit`s; each `{expr[!conv][:spec]}`
    /// field becomes a `FormatValue { value, spec }` (CPython `f"{x:spec}"` ≡
    /// `format(x, "spec")`); parts fold left-to-right with string `+`. Every
    /// field — even a bare `{x}` — routes through `FormatValue` so a class
    /// instance reaches its `__format__`/`__str__` (an empty spec degrades to
    /// `str(x)` for non-instances). A `:spec` is itself a `JoinedStr`, so a
    /// dynamic spec (`f"{x:.{n}f}"`) lowers through this same path; a static one
    /// collapses to a single `StrLit`. `!s`/`!r`/`!a` wraps the value in
    /// `str(...)`/`repr(...)`/`ascii(...)` FIRST (CPython applies the conversion,
    /// then `__format__`).
    pub(super) fn lower_joined_str(
        &mut self,
        j: &rustpython_parser::ast::ExprJoinedStr,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let mut parts: Vec<Idx<HirExpr>> = Vec::with_capacity(j.values.len());
        for part in &j.values {
            match part {
                Expr::Constant(c) => {
                    let Constant::Str(s) = &c.value else {
                        return Err(parse_error("unsupported f-string literal part", span));
                    };
                    let id = self.intern(s);
                    parts.push(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span));
                }
                Expr::FormattedValue(fv) => {
                    let raw = self.lower_expr(fv.value.as_ref())?;
                    // The field's `:spec` is itself a `JoinedStr` — a literal spec
                    // collapses to a `StrLit`, a dynamic one (`{x:.{n}f}`) lowers
                    // through the normal f-string concat. No spec ⇒ empty string.
                    let spec = match &fv.format_spec {
                        Some(spec_expr) => self.lower_format_spec_expr(spec_expr.as_ref(), span)?,
                        None => {
                            let id = self.intern("");
                            self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
                        }
                    };
                    parts.push(self.emit_format_field(raw, fv.conversion, spec, span));
                }
                _ => return Err(parse_error("unsupported f-string part", span)),
            }
        }
        Ok(self.concat_str_parts(parts, span))
    }

    /// Lower a format field's `:spec` (an f-string `JoinedStr`) to a string-valued
    /// expr. A purely-literal spec (the common static case, `f"{x:.4f}"`) collapses
    /// to one `StrLit`; a spec with a nested `{}` (`f"{x:.{n}f}"`) lowers through
    /// the ordinary f-string concat so the embedded value is `format()`-ed and
    /// spliced in.
    pub(super) fn lower_format_spec_expr(&mut self, spec: &Expr, span: Span) -> Result<Idx<HirExpr>> {
        if let Some(lit) = static_spec_literal(spec) {
            let id = self.intern(&lit);
            return Ok(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span));
        }
        self.lower_expr(spec)
    }

    /// Apply an f-string / `str.format` conversion (`!s`/`!r`/`!a`) to `value`,
    /// then wrap it in a `FormatValue` with `spec` (a string-valued expr). The
    /// shared field-builder for f-strings (`lower_joined_str`) and `str.format`
    /// (`lower_str_format`). CPython applies the conversion FIRST, then formats
    /// the (string) result with the spec.
    pub(super) fn emit_format_field(
        &mut self,
        value: Idx<HirExpr>,
        conv: rustpython_parser::ast::ConversionFlag,
        spec: Idx<HirExpr>,
        span: Span,
    ) -> Idx<HirExpr> {
        use rustpython_parser::ast::ConversionFlag;
        let converted = match conv {
            ConversionFlag::Str => self.call_builtin1("str", value, span),
            ConversionFlag::Repr => self.call_builtin1("repr", value, span),
            ConversionFlag::Ascii => self.call_builtin1("ascii", value, span),
            ConversionFlag::None => value,
        };
        self.alloc(
            HirExprKind::FormatValue {
                value: converted,
                spec,
            },
            SemTy::Str,
            span,
        )
    }

    /// Build a one-argument call to an unshadowed builtin by name
    /// (`str`/`repr`/`ascii`), resolved by `semantics` to `Symbol::Builtin`.
    pub(super) fn call_builtin1(&mut self, name: &str, arg: Idx<HirExpr>, span: Span) -> Idx<HirExpr> {
        let fn_name = self.intern(name);
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(fn_name)),
            SemTy::Dyn,
            span,
        );
        self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![arg],
            },
            SemTy::Str,
            span,
        )
    }

    /// Fold string parts left-to-right with `+` (the f-string / `str.format`
    /// tail). An empty part list yields the empty `StrLit`.
    pub(super) fn concat_str_parts(&mut self, parts: Vec<Idx<HirExpr>>, span: Span) -> Idx<HirExpr> {
        let mut iter = parts.into_iter();
        let Some(mut acc) = iter.next() else {
            let id = self.intern("");
            return self.alloc(HirExprKind::StrLit(id), SemTy::Str, span);
        };
        for p in iter {
            acc = self.alloc(
                HirExprKind::BinOp {
                    op: BinOp::Add,
                    l: acc,
                    r: p,
                },
                SemTy::Dyn,
                span,
            );
        }
        acc
    }

}

impl<'a> FnLowerer<'a> {
    pub(super) fn local_ref(&mut self, lid: LocalId, span: Span) -> Idx<HirExpr> {
        let ty = self.locals[lid.index()].ty.clone();
        self.alloc(HirExprKind::Local(lid), ty, span)
    }

    pub(super) fn lower_unary(&mut self, u: &ExprUnaryOp, span: Span) -> Result<Idx<HirExpr>> {
        // Fold `+`/`-` over a numeric literal into a signed literal (so e.g.
        // `-5` is a single `IntLit`, and negative bignum literals work).
        if matches!(u.op, PyUnaryOp::USub | PyUnaryOp::UAdd) {
            if let Expr::Constant(c) = u.operand.as_ref() {
                if let Some(idx) = self.try_fold_numeric(&u.op, &c.value, span) {
                    return Ok(idx);
                }
            }
        }
        let op = match u.op {
            PyUnaryOp::USub => UnaryOp::Neg,
            PyUnaryOp::UAdd => UnaryOp::Pos,
            PyUnaryOp::Invert => UnaryOp::Invert,
            PyUnaryOp::Not => UnaryOp::Not,
        };
        let operand = self.lower_expr(u.operand.as_ref())?;
        let ty = if op == UnaryOp::Not {
            SemTy::Bool
        } else {
            SemTy::Dyn
        };
        Ok(self.alloc(HirExprKind::Unary { op, operand }, ty, span))
    }

    /// Try to fold a unary `+`/`-` applied to a numeric constant.
    pub(super) fn try_fold_numeric(
        &mut self,
        op: &PyUnaryOp,
        c: &Constant,
        span: Span,
    ) -> Option<Idx<HirExpr>> {
        let negative = matches!(op, PyUnaryOp::USub);
        match c {
            Constant::Int(big) => {
                let kind = self.int_literal(&big.to_string(), negative);
                Some(self.alloc(kind, SemTy::Int, span))
            }
            Constant::Float(f) => {
                let v = if negative { -*f } else { *f };
                Some(self.alloc(HirExprKind::FloatLit(v), SemTy::Float, span))
            }
            _ => None,
        }
    }

    pub(super) fn lower_binop(&mut self, b: &ExprBinOp, span: Span) -> Result<Idx<HirExpr>> {
        let op = binop_from_ast(&b.op);
        let l = self.lower_expr(b.left.as_ref())?;
        let r = self.lower_expr(b.right.as_ref())?;
        Ok(self.alloc(HirExprKind::BinOp { op, l, r }, SemTy::Dyn, span))
    }

    pub(super) fn map_cmp(&self, op: &PyCmpOp, span: Span) -> Result<CmpOp> {
        Ok(match op {
            PyCmpOp::Eq => CmpOp::Eq,
            PyCmpOp::NotEq => CmpOp::NotEq,
            PyCmpOp::Lt => CmpOp::Lt,
            PyCmpOp::LtE => CmpOp::LtE,
            PyCmpOp::Gt => CmpOp::Gt,
            PyCmpOp::GtE => CmpOp::GtE,
            PyCmpOp::Is | PyCmpOp::IsNot | PyCmpOp::In | PyCmpOp::NotIn => {
                return Err(parse_error("`is`/`in` comparisons are out of scope", span))
            }
        })
    }

    pub(super) fn lower_compare(&mut self, c: &ExprCompare, span: Span) -> Result<Idx<HirExpr>> {
        if c.ops.len() != c.comparators.len() || c.ops.is_empty() {
            return Err(parse_error("malformed comparison", span));
        }
        // Single comparison: a plain `Compare` value node.
        if c.ops.len() == 1 {
            // `x in y` / `x not in y` → a container membership op (`Contains` reads
            // `container, elem`, so the operand order flips). `not in` negates it.
            if matches!(c.ops[0], PyCmpOp::In | PyCmpOp::NotIn) {
                let container = self.lower_expr(&c.comparators[0])?;
                let elem = self.lower_expr(c.left.as_ref())?;
                let contains = self.alloc(
                    HirExprKind::ContainerExpr {
                        op: ContainerOp::Contains,
                        args: vec![container, elem],
                    },
                    SemTy::Bool,
                    span,
                );
                if matches!(c.ops[0], PyCmpOp::NotIn) {
                    return Ok(self.alloc(
                        HirExprKind::Unary {
                            op: UnaryOp::Not,
                            operand: contains,
                        },
                        SemTy::Bool,
                        span,
                    ));
                }
                return Ok(contains);
            }
            // `x is …` / `x is not …` (Phase 8D + backlog §2). The `None` form is
            // the dedicated null-aware `IsNone` test (it recognizes both the
            // immediate `None` tag and a heap `None` object, which `==` does
            // not). Any other operand pair is general object identity, lowered
            // to `Is` → `rt_is` (bit-identity; never `__eq__`, which is the
            // `Compare` path). `is not` negates either form. `in`/`not in` were
            // handled above; chained `a is b is c` falls through to `map_cmp`
            // below, which still rejects it (out of scope).
            if matches!(c.ops[0], PyCmpOp::Is | PyCmpOp::IsNot) {
                let l_none = is_none_lit(c.left.as_ref());
                let r_none = is_none_lit(&c.comparators[0]);
                let negate = matches!(c.ops[0], PyCmpOp::IsNot);
                let ident = if l_none || r_none {
                    let operand = if r_none {
                        c.left.as_ref()
                    } else {
                        &c.comparators[0]
                    };
                    let v = self.lower_expr(operand)?;
                    self.alloc(HirExprKind::IsNone { value: v }, SemTy::Bool, span)
                } else {
                    let l = self.lower_expr(c.left.as_ref())?;
                    let r = self.lower_expr(&c.comparators[0])?;
                    self.alloc(HirExprKind::Is { l, r }, SemTy::Bool, span)
                };
                if negate {
                    return Ok(self.alloc(
                        HirExprKind::Unary {
                            op: UnaryOp::Not,
                            operand: ident,
                        },
                        SemTy::Bool,
                        span,
                    ));
                }
                return Ok(ident);
            }
            let op = self.map_cmp(&c.ops[0], span)?;
            let l = self.lower_expr(c.left.as_ref())?;
            let r = self.lower_expr(&c.comparators[0])?;
            return Ok(self.alloc(HirExprKind::Compare { op, l, r }, SemTy::Bool, span));
        }
        // Chained comparison `a < b < c`: short-circuit branch CFG with each
        // interior operand evaluated exactly once (single-eval), lazily.
        let res = self.fresh_local(SemTy::Bool);
        let false_b = self.new_block();
        let true_b = self.new_block();
        let join = self.new_block();

        let lv = self.lower_expr(c.left.as_ref())?;
        let mut prev = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: prev,
            value: lv,
        });

        for (i, comp) in c.comparators.iter().enumerate() {
            let op = self.map_cmp(&c.ops[i], span)?;
            let cv = self.lower_expr(comp)?;
            let cur = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: cur,
                value: cv,
            });
            let lref = self.local_ref(prev, span);
            let rref = self.local_ref(cur, span);
            let cmp = self.alloc(
                HirExprKind::Compare {
                    op,
                    l: lref,
                    r: rref,
                },
                SemTy::Bool,
                span,
            );
            let next = self.new_block();
            self.seal(HirTerminator::Branch {
                cond: cmp,
                then: next,
                else_: false_b,
            });
            self.switch(next);
            prev = cur;
        }
        self.seal(HirTerminator::Jump(true_b));

        self.switch(true_b);
        let t = self.alloc(HirExprKind::BoolLit(true), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: t,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(false_b);
        let fb = self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: fb,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    /// Short-circuit `and`/`or` over `values` (≥2), into branch CFG + result local.
    pub(super) fn lower_boolop(&mut self, b: &ExprBoolOp) -> Result<Idx<HirExpr>> {
        let span = to_span(b.range());
        let res = self.fresh_local(SemTy::Dyn);
        let join = self.new_block();
        let n = b.values.len();
        for (i, val) in b.values.iter().enumerate() {
            let v = self.lower_expr(val)?;
            self.push_stmt(HirStmt::Assign {
                target: res,
                value: v,
            });
            if i + 1 < n {
                let next = self.new_block();
                let cond = self.local_ref(res, span);
                match b.op {
                    // `and`: keep going while truthy; short-circuit (res = falsy) to join.
                    PyBoolOp::And => self.seal(HirTerminator::Branch {
                        cond,
                        then: next,
                        else_: join,
                    }),
                    // `or`: short-circuit (res = truthy) to join; else keep going.
                    PyBoolOp::Or => self.seal(HirTerminator::Branch {
                        cond,
                        then: join,
                        else_: next,
                    }),
                }
                self.switch(next);
            } else {
                self.seal(HirTerminator::Jump(join));
            }
        }
        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    pub(super) fn lower_ifexp(&mut self, e: &ExprIfExp) -> Result<Idx<HirExpr>> {
        let span = to_span(e.range());
        let res = self.fresh_local(SemTy::Dyn);
        let cond = self.lower_expr(e.test.as_ref())?;
        let then_b = self.new_block();
        let else_b = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: then_b,
            else_: else_b,
        });

        self.switch(then_b);
        let bv = self.lower_expr(e.body.as_ref())?;
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: bv,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(else_b);
        let ev = self.lower_expr(e.orelse.as_ref())?;
        self.push_stmt(HirStmt::Assign {
            target: res,
            value: ev,
        });
        self.seal(HirTerminator::Jump(join));

        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    /// Collect per-element `isinstance` checks for a type-tuple
    /// `isinstance(x, (A, B, ...))`. Each check reads the singly-staged receiver
    /// `recv`. Nested tuples flatten (CPython semantics); a `Name` resolves to a
    /// user class (`IsInstance`) or a builtin-type target (`IsInstanceBuiltin`);
    /// any other element is a clean error (matching the single-type form's
    /// strictness — the second arg must be a class / builtin-type name).
    pub(super) fn collect_isinstance_checks(
        &mut self,
        elts: &[Expr],
        recv: LocalId,
        checks: &mut Vec<Idx<HirExpr>>,
        span: Span,
    ) -> Result<()> {
        for elt in elts {
            match elt {
                Expr::Tuple(t) => {
                    self.collect_isinstance_checks(&t.elts, recv, checks, span)?;
                }
                Expr::Name(cls) => {
                    if let Some((class_id, _)) = self.ctx.class_map.get(cls.id.as_str()).copied() {
                        let value = self.local_ref(recv, span);
                        checks.push(self.alloc(
                            HirExprKind::IsInstance { value, class_id },
                            SemTy::Bool,
                            span,
                        ));
                    } else if let Some(target) = isinstance_builtin_target(cls.id.as_str()) {
                        let value = self.local_ref(recv, span);
                        checks.push(self.alloc(
                            HirExprKind::IsInstanceBuiltin { value, target },
                            SemTy::Bool,
                            span,
                        ));
                    } else {
                        return Err(parse_error(
                            format!(
                                "isinstance() type-tuple element `{}` is not a known class \
                                 or builtin type",
                                cls.id.as_str()
                            ),
                            span,
                        ));
                    }
                }
                _ => {
                    return Err(parse_error(
                        "isinstance() type-tuple elements must be class / builtin-type names",
                        span,
                    ));
                }
            }
        }
        Ok(())
    }

    /// Combine `checks` into a short-circuit `or` (mirrors `lower_boolop`'s `or`
    /// arm): empty ⇒ `False`; else a `Bool` result local + `join` block, each
    /// check assigned to `res` and (non-last) branching to `join` when truthy.
    /// Every check is side-effect-free (it reads the staged receiver), so this is
    /// observationally an eager OR but stays uniform with `lower_boolop`.
    pub(super) fn or_combine_checks(&mut self, checks: Vec<Idx<HirExpr>>, span: Span) -> Idx<HirExpr> {
        if checks.is_empty() {
            return self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span);
        }
        let res = self.fresh_local(SemTy::Bool);
        let join = self.new_block();
        let n = checks.len();
        for (i, check) in checks.into_iter().enumerate() {
            self.push_stmt(HirStmt::Assign {
                target: res,
                value: check,
            });
            if i + 1 < n {
                let next = self.new_block();
                let cond = self.local_ref(res, span);
                self.seal(HirTerminator::Branch {
                    cond,
                    then: join,
                    else_: next,
                });
                self.switch(next);
            } else {
                self.seal(HirTerminator::Jump(join));
            }
        }
        self.switch(join);
        self.local_ref(res, span)
    }

}

/// If a format-spec (modeled by rustpython as a `JoinedStr` of literal parts) is
/// purely literal text (`:.4f`, `:4d`), return it as a plain string — the static
/// fast-path that keeps a constant spec a `Const::Str`. Returns `None` when the
/// spec carries a nested `{}` interpolation (`f"{x:.{n}f}"`), which the caller
/// then lowers dynamically through the f-string concat.
pub(super) fn static_spec_literal(spec: &Expr) -> Option<String> {
    match spec {
        Expr::JoinedStr(j) => {
            let mut out = String::new();
            for part in &j.values {
                match part {
                    Expr::Constant(c) => match &c.value {
                        Constant::Str(s) => out.push_str(s),
                        _ => return None,
                    },
                    _ => return None,
                }
            }
            Some(out)
        }
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}


/// Parse a `str.format` template into literal-text / replacement-field segments.
/// Handles `{{`/`}}` escapes; rejects a nested `{}` inside a field (a dynamic
/// `.format` spec — deferred), an unmatched / stray brace, and (in
/// [`parse_format_field`]) `{0.attr}`/`{0[k]}` field access.
pub(super) fn parse_format_template(s: &str, span: Span) -> Result<Vec<FmtSeg>> {
    let mut segs = Vec::new();
    let mut lit = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    lit.push('{');
                    continue;
                }
                if !lit.is_empty() {
                    segs.push(FmtSeg::Lit(std::mem::take(&mut lit)));
                }
                // Read the field body up to the matching '}'.
                let mut body = String::new();
                let mut closed = false;
                while let Some(&nc) = chars.peek() {
                    if nc == '}' {
                        chars.next();
                        closed = true;
                        break;
                    }
                    if nc == '{' {
                        return Err(parse_error(
                            "a nested `{}` inside a .format() field/spec is out of scope",
                            span,
                        ));
                    }
                    body.push(nc);
                    chars.next();
                }
                if !closed {
                    return Err(parse_error("unmatched '{' in format string", span));
                }
                segs.push(parse_format_field(&body, span)?);
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    lit.push('}');
                    continue;
                }
                return Err(parse_error("single '}' encountered in format string", span));
            }
            _ => lit.push(ch),
        }
    }
    if !lit.is_empty() {
        segs.push(FmtSeg::Lit(lit));
    }
    Ok(segs)
}

/// Parse a single `str.format` field body `[name][!conv][:spec]` (the braces
/// already stripped). The spec is static text (a nested `{}` was rejected by
/// the caller).
pub(super) fn parse_format_field(body: &str, span: Span) -> Result<FmtSeg> {
    use rustpython_parser::ast::ConversionFlag;
    // Split off `:spec` at the first colon, then `!conv` from the remaining head.
    let (head, spec) = match body.find(':') {
        Some(i) => (&body[..i], body[i + 1..].to_string()),
        None => (body, String::new()),
    };
    let (name, conv) = match head.find('!') {
        Some(i) => {
            let flag = match &head[i + 1..] {
                "r" => ConversionFlag::Repr,
                "s" => ConversionFlag::Str,
                "a" => ConversionFlag::Ascii,
                other => {
                    return Err(parse_error(
                        format!("unknown conversion specifier '{other}' in format string"),
                        span,
                    ))
                }
            };
            (&head[..i], flag)
        }
        None => (head, ConversionFlag::None),
    };
    if name.contains('.') || name.contains('[') {
        return Err(parse_error(
            "`{0.attr}` / `{0[key]}` field access in .format() is out of scope",
            span,
        ));
    }
    let field = if name.is_empty() {
        FmtFieldRef::Auto
    } else if name.bytes().all(|b| b.is_ascii_digit()) {
        let idx = name.parse::<usize>().map_err(|_| {
            parse_error(
                format!("invalid field index '{name}' in format string"),
                span,
            )
        })?;
        FmtFieldRef::Index(idx)
    } else {
        FmtFieldRef::Keyword(name.to_string())
    };
    Ok(FmtSeg::Field { field, conv, spec })
}

/// If `expr` is a direct `print(...)` call, return it.
pub(super) fn as_print_call(expr: &Expr) -> Option<&rustpython_parser::ast::ExprCall> {
    if let Expr::Call(call) = expr {
        if let Expr::Name(n) = call.func.as_ref() {
            if n.id.as_str() == "print" {
                return Some(call);
            }
        }
    }
    None
}

/// Parse a `print(..., file=…)` keyword value into a [`PrintTarget`]. Only the
/// canonical `sys.stdout` / `sys.stderr` attribute forms are supported (the
/// streams the runtime can target); anything else is a compile error. The
/// stream objects themselves are never materialized — `file=` only selects which
/// runtime print target lowering toggles.
pub(super) fn parse_print_target(value: &Expr) -> Result<PrintTarget> {
    if let Expr::Attribute(a) = value {
        if let Expr::Name(base) = a.value.as_ref() {
            if base.id.as_str() == "sys" {
                match a.attr.as_str() {
                    "stdout" => return Ok(PrintTarget::Stdout),
                    "stderr" => return Ok(PrintTarget::Stderr),
                    _ => {}
                }
            }
        }
    }
    Err(parse_error(
        "print() file= must be sys.stdout or sys.stderr",
        to_span(value.range()),
    ))
}

/// Parse a `print(..., flush=…)` keyword value. Like `sep`/`end` (string
/// literals), `flush` is a compile-time literal in our subset: a bare `True` /
/// `False`. A non-literal flush flag is a compile error.
pub(super) fn parse_flush_flag(value: &Expr) -> Result<bool> {
    if let Expr::Constant(c) = value {
        if let Constant::Bool(b) = &c.value {
            return Ok(*b);
        }
    }
    Err(parse_error(
        "print() flush= must be True or False",
        to_span(value.range()),
    ))
}

