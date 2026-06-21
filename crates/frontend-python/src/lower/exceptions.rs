use super::*;

impl<'a> FnLowerer<'a> {
    // ── exceptions: try / raise (Phase 7A/7B) ───────────────────────────────

    /// Lower a `try` statement. `try/except/finally` nests as
    /// `try { try/except } finally` (two frames).
    pub(super) fn lower_try(&mut self, t: &rustpython_parser::ast::StmtTry) -> Result<bool> {
        let span = to_span(t.range());
        if !t.finalbody.is_empty() {
            self.lower_try_finally(t, span)?;
        } else {
            self.lower_try_except(&t.body, &t.handlers, &t.orelse, span)?;
        }
        Ok(false)
    }

    /// `try X finally F`: normal edge exits the region then runs `<F>`;
    /// exceptional edge `StartHandling; <F>; Reraise`. Early exits re-lower
    /// `<F>` via the [`ScopeCtx::Finally`] entry.
    pub(super) fn lower_try_finally(&mut self, t: &rustpython_parser::ast::StmtTry, span: Span) -> Result<()> {
        let try_b = self.new_block();
        let exc_b = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Jump(try_b));

        self.switch(try_b);
        let outer = self.cur_handler;
        self.cur_handler = Some(exc_b);
        self.scope_stack.push(ScopeCtx::Finally {
            outer,
            stmts: t.finalbody.clone(),
        });
        if t.handlers.is_empty() {
            debug_assert!(
                t.orelse.is_empty(),
                "orelse without handlers is a SyntaxError"
            );
            self.lower_body(&t.body)?;
        } else {
            self.lower_try_except(&t.body, &t.handlers, &t.orelse, span)?;
        }
        self.scope_stack.pop();
        if self.cur_open() {
            // The finalbody runs OUTSIDE the region it guards (its own raise
            // propagates outward, and `finally` must not re-run): exit to a
            // fresh block under the outer handler.
            self.exit_protected(outer);
            self.lower_body(&t.finalbody)?;
            self.seal(HirTerminator::Jump(join));
        }
        self.cur_handler = outer;

        // Exceptional edge (runs under the OUTER handler). Park the in-flight
        // exception (so a nested raise chains it as __context__), run the
        // finalbody, then re-raise it.
        self.switch(exc_b);
        self.push_stmt(HirStmt::ExcOp(ExcOp::StartHandling));
        self.lower_body(&t.finalbody)?;
        if self.cur_open() {
            self.push_stmt(HirStmt::Raise(HirRaise::Reraise));
            self.seal(HirTerminator::Unreachable);
        }

        self.switch(join);
        Ok(())
    }

    /// `try/except[/else]`: lower the body under the handler context, exit
    /// the region on the normal edge (`else` after the exit so its exceptions
    /// escape), then the handler chain (`Matches*` tests; tuple clause =
    /// OR-chain), with a no-match tail that re-raises.
    pub(super) fn lower_try_except(
        &mut self,
        body: &[Stmt],
        handlers: &[rustpython_parser::ast::ExceptHandler],
        orelse: &[Stmt],
        span: Span,
    ) -> Result<()> {
        debug_assert!(
            !handlers.is_empty(),
            "try without handlers or finally is a SyntaxError"
        );
        let try_b = self.new_block();
        let h_test = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Jump(try_b));

        // ── try body ──
        self.switch(try_b);
        let outer = self.cur_handler;
        self.cur_handler = Some(h_test);
        self.scope_stack.push(ScopeCtx::TryFrame { outer });
        self.lower_body(body)?;
        self.scope_stack.pop();
        if self.cur_open() {
            // `else` runs after the region exit: its exceptions are NOT
            // caught here.
            self.exit_protected(outer);
            self.lower_body(orelse)?;
            self.seal(HirTerminator::Jump(join));
        }
        self.cur_handler = outer;

        // ── handler chain (runs under the OUTER handler) ──
        self.switch(h_test);
        for (hi, handler) in handlers.iter().enumerate() {
            let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = handler;
            let hspan = to_span(h.range());
            let body_b = self.new_block();
            let next_test = self.new_block();
            match h.type_.as_deref() {
                // Bare `except:` catches everything (must be last in CPython).
                None => {
                    if hi + 1 != handlers.len() {
                        return Err(parse_error("default 'except:' must be last", hspan));
                    }
                    self.seal(HirTerminator::Jump(body_b));
                }
                Some(Expr::Tuple(tu)) => {
                    // OR-chain: any matching member enters the body.
                    for (i, te) in tu.elts.iter().enumerate() {
                        let q = self.exc_match_query(te)?;
                        if i + 1 == tu.elts.len() {
                            self.seal(HirTerminator::Branch {
                                cond: q,
                                then: body_b,
                                else_: next_test,
                            });
                        } else {
                            let more = self.new_block();
                            self.seal(HirTerminator::Branch {
                                cond: q,
                                then: body_b,
                                else_: more,
                            });
                            self.switch(more);
                        }
                    }
                }
                Some(single) => {
                    let q = self.exc_match_query(single)?;
                    self.seal(HirTerminator::Branch {
                        cond: q,
                        then: body_b,
                        else_: next_test,
                    });
                }
            }

            // ── handler body ──
            self.switch(body_b);
            if let Some(name) = &h.name {
                // Bind `as e` BEFORE StartHandling (rt_exc_get_current reads
                // the still-current exception). A fresh local per binding,
                // shadowing the name, with the clause's static type.
                let bind_ty = self.exc_clause_semty(h.type_.as_deref());
                let cur = self.alloc(
                    HirExprKind::ExcQuery(ExcQuery::Current),
                    bind_ty.clone(),
                    hspan,
                );
                self.bind_exc_name(name.as_str(), bind_ty, cur);
            }
            self.push_stmt(HirStmt::ExcOp(ExcOp::StartHandling));
            self.scope_stack.push(ScopeCtx::Handler);
            self.lower_body(&h.body)?;
            self.scope_stack.pop();
            if self.cur_open() {
                self.push_stmt(HirStmt::ExcOp(ExcOp::EndHandling));
                self.seal(HirTerminator::Jump(join));
            }
            self.switch(next_test);
        }

        // ── no handler matched: propagate outward ──
        self.push_stmt(HirStmt::Raise(HirRaise::Reraise));
        self.seal(HirTerminator::Unreachable);

        self.switch(join);
        let _ = span;
        Ok(())
    }

    /// The `Matches*` query for one `except` clause member: a user class from
    /// the class map, else a builtin exception name.
    pub(super) fn exc_match_query(&mut self, te: &Expr) -> Result<Idx<HirExpr>> {
        let span = to_span(te.range());
        let Expr::Name(n) = te else {
            return Err(parse_error(
                "except clause must name an exception class",
                span,
            ));
        };
        let q = if let Some((cid, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
            ExcQuery::MatchesClass(cid)
        } else if let Some((class_id, _)) = self.ctx.stdlib.exceptions.get(n.id.as_str()).copied() {
            // A stdlib exception (`except HTTPError:`, Phase 8D): match by its
            // reserved class id (the runtime self-matches the raised
            // `custom_class_id`).
            ExcQuery::MatchesClass(ClassId::new(class_id as u32))
        } else if let Some(tag) = pyaot_core_defs::exception_name_to_tag(n.id.as_str()) {
            ExcQuery::MatchesBuiltin(tag)
        } else {
            return Err(parse_error(
                format!(
                    "unknown exception type `{}` in except clause",
                    n.id.as_str()
                ),
                span,
            ));
        };
        Ok(self.alloc(HirExprKind::ExcQuery(q), SemTy::Bool, span))
    }

    /// Fold `value.__class__.__name__` to a string literal from `value`'s
    /// statically-known type (Phase 7B). Only a directly-bound name whose
    /// static type is a builtin exception or a user class folds; anything else
    /// is rejected with a clear error.
    pub(super) fn fold_class_name(&mut self, value: &Expr, span: Span) -> Result<Idx<HirExpr>> {
        let static_ty = match value {
            Expr::Name(n) => {
                let iname = self.intern(n.id.as_str());
                match self.scope.get(&iname).copied() {
                    Some(Binding::Direct(lid)) => Some(self.locals[lid.index()].ty.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        let name_str = match static_ty {
            Some(SemTy::BuiltinException(kind)) => kind.name().to_string(),
            Some(SemTy::Class { name, .. }) => self.interner.resolve(name).to_string(),
            _ => {
                return Err(parse_error(
                    "`.__class__.__name__` requires a variable with a statically-known \
                     exception/class type (bind it via `except SomeError as e`)",
                    span,
                ))
            }
        };
        let id = self.intern(&name_str);
        Ok(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span))
    }

    /// The static type an `except … as e` binding carries: a single builtin
    /// name → `BuiltinException`; a single user class → `Class`; a tuple
    /// clause → the `Union` of its members (NOT `Dyn` — `str(e)`/`print(e)`
    /// must still route to the exception-message surface, and the generic
    /// Dyn print renders the object repr; Principle 2 demands the imprecise
    /// type stays behaviorally correct). A bare clause stays `Dyn`.
    pub(super) fn exc_clause_semty(&mut self, ty: Option<&Expr>) -> SemTy {
        match ty {
            Some(e @ Expr::Name(_)) => self.exc_member_semty(e),
            Some(Expr::Tuple(t)) => {
                let mut members: Vec<SemTy> = Vec::new();
                for e in &t.elts {
                    let m = self.exc_member_semty(e);
                    if m == SemTy::Dyn {
                        return SemTy::Dyn;
                    }
                    if !members.contains(&m) {
                        members.push(m);
                    }
                }
                match members.len() {
                    0 => SemTy::Dyn,
                    1 => members.pop().expect("one member"),
                    _ => SemTy::Union(members),
                }
            }
            _ => SemTy::Dyn,
        }
    }

    /// One except-clause member's static type (builtin exception / user class).
    pub(super) fn exc_member_semty(&mut self, e: &Expr) -> SemTy {
        let Expr::Name(n) = e else { return SemTy::Dyn };
        if let Some((cid, iname)) = self.ctx.class_map.get(n.id.as_str()).copied() {
            return SemTy::Class {
                class_id: cid,
                name: iname,
            };
        }
        if let Some(kind) = pyaot_core_defs::BuiltinExceptionKind::from_name(n.id.as_str()) {
            return SemTy::BuiltinException(kind);
        }
        // A stdlib exception (`HTTPError`/`URLError`, …) caught by its own name:
        // model the bound `e` as its builtin PARENT so `print(e)` / `str(e)` route
        // through the deterministic exception-message path. Otherwise `e` is `Dyn`
        // and renders the default object repr — a non-deterministic heap ADDRESS in
        // stdout (Phase 8 follow-up; matches the `except <parent>` behaviour).
        if let Some((_cid, parent_tag)) = self.ctx.stdlib.exceptions.get(n.id.as_str()).copied() {
            if let Some(parent) = pyaot_core_defs::BuiltinExceptionKind::from_tag(parent_tag) {
                return SemTy::BuiltinException(parent);
            }
        }
        SemTy::Dyn
    }

    /// Bind an `except … as e` name to a FRESH typed local, shadowing any
    /// previous binding (CPython unbinds `e` after the handler; a fresh slot
    /// per handler keeps each binding's static type precise). Celled names
    /// keep their cell (uniform tagged content).
    pub(super) fn bind_exc_name(&mut self, name: &str, ty: SemTy, value: Idx<HirExpr>) {
        let iname = self.intern(name);
        if self.celled.contains(&iname) || self.global_decls.contains(&iname) {
            self.write_named(iname, SemTy::Dyn, value);
            return;
        }
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name: iname,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        self.scope.insert(iname, Binding::Direct(id));
        self.push_stmt(HirStmt::Assign { target: id, value });
    }

    /// Lower a `raise` statement. Always terminates the block.
    ///
    /// `raise TARGET from CAUSE` lowers to: stage the target's scalar
    /// message/value operand (so it evaluates before the cause, matching
    /// CPython's target-first order), emit an [`HirStmt::ArmCause`] that stashes
    /// the explicit cause, then the unchanged [`HirStmt::Raise`] for TARGET. The
    /// next raise builder consumes the pending cause — no per-target cause
    /// variant.
    pub(super) fn lower_raise(&mut self, r: &rustpython_parser::ast::StmtRaise) -> Result<bool> {
        let span = to_span(r.range());
        match &r.exc {
            // Bare `raise` — re-raise the exception being handled.
            None => {
                if r.cause.is_some() {
                    return Err(parse_error("bare raise cannot carry a cause", span));
                }
                self.push_stmt(HirStmt::Raise(HirRaise::Reraise));
            }
            Some(exc) => {
                let mut raise = self.classify_raise_target(exc, span)?;
                self.stage_raise_target_operands(&mut raise, span);
                self.emit_cause_arm(r.cause.as_deref(), span)?;
                self.push_stmt(HirStmt::Raise(raise));
            }
        }
        self.seal(HirTerminator::Unreachable);
        Ok(true)
    }

    /// Stage the target's scalar message/value operand into a temp so its side
    /// effects happen before the cause arm (CPython evaluates the raise target
    /// first). A `Custom` target's `__init__` body still runs at lowering after
    /// the arm — documented as a boundary; absent from realistic code.
    fn stage_raise_target_operands(&mut self, raise: &mut HirRaise, span: Span) {
        match raise {
            HirRaise::Builtin { msg, .. } | HirRaise::Stdlib { msg, .. } => {
                if let Some(m) = msg {
                    *m = self.stage_hir_temp(*m, span);
                }
            }
            HirRaise::Instance { value } => {
                *value = self.stage_hir_temp(*value, span);
            }
            HirRaise::Custom { .. } | HirRaise::Reraise => {}
        }
    }

    /// Assign an already-lowered HIR expression to a fresh temp (preserving its
    /// static type) and return a read of that temp. Pins the expression's side
    /// effects to the current statement position.
    fn stage_hir_temp(&mut self, e: Idx<HirExpr>, span: Span) -> Idx<HirExpr> {
        let ty = self.exprs[e].ty.clone();
        let tmp = self.fresh_local(ty);
        self.push_stmt(HirStmt::Assign {
            target: tmp,
            value: e,
        });
        self.local_ref(tmp, span)
    }

    /// Classify a `raise EXPR` target (the `from CAUSE`, if any, is handled
    /// separately by [`Self::emit_cause_arm`]). Builtin-exception name
    /// resolution is frontend-local: scope binding → `Instance`; class map →
    /// `Custom`; `exception_name_to_tag` → builtin; else an error.
    pub(super) fn classify_raise_target(
        &mut self,
        exc: &Expr,
        span: Span,
    ) -> Result<HirRaise> {
        // `raise Name(...)` — a constructed exception.
        if let Expr::Call(c) = exc {
            if let Expr::Name(n) = c.func.as_ref() {
                if !c.keywords.is_empty() {
                    return Err(parse_error(
                        "keyword arguments in a raise expression are out of scope",
                        span,
                    ));
                }
                let iname = self.intern(n.id.as_str());
                if !self.scope.contains_key(&iname) {
                    if let Some((cid, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
                        let args = self.lower_expr_list(&c.args)?;
                        return Ok(HirRaise::Custom {
                            class_id: cid,
                            args,
                        });
                    }
                    if let Some((class_id, parent_tag)) =
                        self.ctx.stdlib.exceptions.get(n.id.as_str()).copied()
                    {
                        // Synthesize the CPython __str__ for the exceptions
                        // whose message is not the first positional arg:
                        // HTTPError(url, code, msg, hdrs, fp) prints
                        // "HTTP Error {code}: {msg}"; URLError(reason) prints
                        // "<urlopen error {reason}>". Everything else keeps
                        // the first positional arg as the message.
                        let msg = match (n.id.as_str(), c.args.len()) {
                            ("HTTPError", 3..) => {
                                let code = self.lower_expr(&c.args[1])?;
                                let msg_arg = self.lower_expr(&c.args[2])?;
                                Some(self.synth_concat_str(
                                    &[("HTTP Error ", code), (": ", msg_arg)],
                                    "",
                                    span,
                                ))
                            }
                            ("URLError", 1..) => {
                                let reason = self.lower_expr(&c.args[0])?;
                                Some(self.synth_concat_str(
                                    &[("<urlopen error ", reason)],
                                    ">",
                                    span,
                                ))
                            }
                            _ => match c.args.first() {
                                Some(a) => Some(self.lower_expr(a)?),
                                None => None,
                            },
                        };
                        return Ok(HirRaise::Stdlib {
                            class_id,
                            exc_type_tag: parent_tag,
                            msg,
                        });
                    }
                    if let Some(tag) = pyaot_core_defs::exception_name_to_tag(n.id.as_str()) {
                        if c.args.len() > 1 {
                            return Err(parse_error(
                                "multi-argument builtin exceptions are out of scope",
                                span,
                            ));
                        }
                        let msg = match c.args.first() {
                            Some(a) => Some(self.lower_expr(a)?),
                            None => None,
                        };
                        return Ok(HirRaise::Builtin { tag, msg });
                    }
                }
            }
        }
        // `raise Name` — a bare class (builtin/custom) or a caught instance.
        if let Expr::Name(n) = exc {
            let iname = self.intern(n.id.as_str());
            if self.scope.contains_key(&iname) {
                let value = self.lower_expr(exc)?;
                return Ok(HirRaise::Instance { value });
            }
            if let Some((cid, _)) = self.ctx.class_map.get(n.id.as_str()).copied() {
                return Ok(HirRaise::Custom {
                    class_id: cid,
                    args: vec![],
                });
            }
            if let Some((class_id, parent_tag)) =
                self.ctx.stdlib.exceptions.get(n.id.as_str()).copied()
            {
                return Ok(HirRaise::Stdlib {
                    class_id,
                    exc_type_tag: parent_tag,
                    msg: None,
                });
            }
            if let Some(tag) = pyaot_core_defs::exception_name_to_tag(n.id.as_str()) {
                return Ok(HirRaise::Builtin { tag, msg: None });
            }
        }
        Err(parse_error(
            "raise target must be an exception class, a constructed exception, \
             or a caught exception variable",
            span,
        ))
    }

    /// Emit the `from CAUSE` arm for a `raise TARGET from CAUSE`, if present.
    /// The cause has exactly three shapes (mirroring CPython's accepted
    /// causes): `from None` → suppress; a builtin exception (bare or
    /// constructed) → a scalar `(tag, msg)`; any other value expression (a
    /// caught variable, a constructed custom/stdlib exception) → a Tagged
    /// instance value the runtime introspects. A bare custom/stdlib *class*
    /// cause is a clean compile error.
    fn emit_cause_arm(&mut self, cause: Option<&Expr>, span: Span) -> Result<()> {
        let Some(cause) = cause else { return Ok(()) };

        // `from None` → suppress the implicit `__context__` chain.
        if matches!(cause, Expr::Constant(c) if matches!(c.value, Constant::None)) {
            self.push_stmt(HirStmt::ArmCause(ArmCause::Suppress));
            return Ok(());
        }

        // Resolve the "head name" of a bare name or a `Name(...)` constructor.
        let head: Option<(&str, &[Expr])> = match cause {
            Expr::Name(n) => Some((n.id.as_str(), &[])),
            Expr::Call(c) => match c.func.as_ref() {
                Expr::Name(n) if c.keywords.is_empty() => (n.id.as_str(), c.args.as_slice()).into(),
                _ => None,
            },
            _ => None,
        };

        if let Some((name, cargs)) = head {
            let iname = self.intern(name);
            // An in-scope name is a runtime value (a caught variable / a
            // local holding an exception) → value path.
            if !self.scope.contains_key(&iname) {
                // A builtin exception name (bare class or constructor) → the
                // scalar builtin cause (no builtin-exception-as-value needed).
                if let Some(cause_tag) = pyaot_core_defs::exception_name_to_tag(name) {
                    if cargs.len() > 1 {
                        return Err(parse_error(
                            "multi-argument builtin exceptions are out of scope",
                            span,
                        ));
                    }
                    let cause_msg = match cargs.first() {
                        Some(a) => Some(self.lower_expr(a)?),
                        None => None,
                    };
                    self.push_stmt(HirStmt::ArmCause(ArmCause::Builtin {
                        cause_tag,
                        cause_msg,
                    }));
                    return Ok(());
                }
                // A bare custom/stdlib *class* cause (no parens) has no instance
                // to introspect — reject cleanly. A constructor call falls
                // through to the value path.
                let is_class = self.ctx.class_map.contains_key(name)
                    || self.ctx.stdlib.exceptions.contains_key(name);
                if is_class && matches!(cause, Expr::Name(_)) {
                    return Err(parse_error(
                        "a bare class cause (`raise ... from SomeError`) is out of scope; \
                         construct it (`from SomeError(...)`) or use a caught variable",
                        span,
                    ));
                }
            }
        }

        // Value path: a caught variable, a constructed custom/stdlib exception,
        // or any other value expression. A non-exception value raises TypeError
        // at runtime (`rt_exc_arm_cause_value`), matching CPython.
        let v = self.lower_expr(cause)?;
        self.push_stmt(HirStmt::ArmCause(ArmCause::Value(v)));
        Ok(())
    }

    // ── with (Phase 7D) ──────────────────────────────────────────────────────

    /// Lower a `with` statement: items nest left-to-right; each item desugars
    /// to `__enter__` + `TryEnter` + `__exit__` on both edges (a truthy
    /// exceptional `__exit__` swallows the exception).
    pub(super) fn lower_with(&mut self, w: &rustpython_parser::ast::StmtWith) -> Result<bool> {
        let span = to_span(w.range());
        self.lower_with_items(&w.items, &w.body, span)?;
        Ok(false)
    }

    pub(super) fn lower_with_items(
        &mut self,
        items: &[rustpython_parser::ast::WithItem],
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        let Some((first, rest)) = items.split_first() else {
            return self.lower_body(body);
        };

        // mgr = EXPR; val = mgr.__enter__(); [bind TARGET]
        let mgr_e = self.lower_expr(&first.context_expr)?;
        let mgr = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: mgr,
            value: mgr_e,
        });
        let recv = self.local_ref(mgr, span);
        let enter_name = self.intern("__enter__");
        let enter = self.alloc(
            HirExprKind::MethodCall {
                recv,
                method_name: enter_name,
                args: vec![],
                kwargs: vec![],
            },
            SemTy::Dyn,
            span,
        );
        match &first.optional_vars {
            Some(t) => self.bind_for_target(t.as_ref(), enter, span)?,
            None => self.push_stmt(HirStmt::Expr(enter)),
        }

        let body_b = self.new_block();
        let exit_exc = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Jump(body_b));

        // ── body (or the next nested item) ──
        self.switch(body_b);
        let outer = self.cur_handler;
        self.cur_handler = Some(exit_exc);
        self.scope_stack.push(ScopeCtx::WithCleanup { outer, mgr });
        self.lower_with_items(rest, body, span)?;
        self.scope_stack.pop();
        if self.cur_open() {
            // `__exit__` runs outside the region (its own raise propagates).
            self.exit_protected(outer);
            self.emit_exit_none_call(mgr, span);
            self.seal(HirTerminator::Jump(join));
        }
        self.cur_handler = outer;

        // ── exceptional edge (under the OUTER handler):
        //    r = mgr.__exit__(e, e, None); truthy swallows ──
        self.switch(exit_exc);
        let e_local = self.fresh_local_tagged();
        let cur = self.alloc(HirExprKind::ExcQuery(ExcQuery::Current), SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: e_local,
            value: cur,
        });
        self.push_stmt(HirStmt::ExcOp(ExcOp::StartHandling));
        let recv2 = self.local_ref(mgr, span);
        let e1 = self.local_ref(e_local, span);
        let e2 = self.local_ref(e_local, span);
        let none = self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span);
        let exit_name = self.intern("__exit__");
        let r = self.alloc(
            HirExprKind::MethodCall {
                recv: recv2,
                method_name: exit_name,
                args: vec![e1, e2, none],
                kwargs: vec![],
            },
            SemTy::Dyn,
            span,
        );
        let r_local = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: r_local,
            value: r,
        });
        let swallow_b = self.new_block();
        let reraise_b = self.new_block();
        let cond = self.local_ref(r_local, span);
        self.seal(HirTerminator::Branch {
            cond,
            then: swallow_b,
            else_: reraise_b,
        });
        self.switch(swallow_b);
        self.push_stmt(HirStmt::ExcOp(ExcOp::EndHandling));
        self.seal(HirTerminator::Jump(join));
        self.switch(reraise_b);
        self.push_stmt(HirStmt::Raise(HirRaise::Reraise));
        self.seal(HirTerminator::Unreachable);

        self.switch(join);
        Ok(())
    }

}
