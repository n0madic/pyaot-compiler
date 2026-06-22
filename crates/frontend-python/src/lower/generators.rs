use super::*;

impl<'a> FnLowerer<'a> {
    // ── generators (Phase 6E) ────────────────────────────────────────────────

    /// Lower a generator expression `(elt for t in it …)` (Phase 6E): a
    /// synthetic generator whose OUTERMOST iterable is an eager parameter
    /// (CPython semantics); inner clauses/elt must be free-var-free (captures in
    /// generators are out of scope), so the gate keeps genexprs self-contained.
    pub(super) fn lower_genexpr(&mut self, g: &ExprGeneratorExp, span: Span) -> Result<Idx<HirExpr>> {
        if g.generators.is_empty() {
            return Err(parse_error("malformed generator expression", span));
        }
        if g.generators.iter().any(|c| c.is_async) {
            return Err(parse_error(
                "async generator expressions are out of scope",
                span,
            ));
        }
        // The outermost iterable, evaluated eagerly in THIS scope.
        let outer = self.lower_expr(&g.generators[0].iter)?;

        let synth = self.synth_name("<genexpr>");
        let name = self.interner.intern(&synth);
        let wrapper_fid = self.shared.reserve();
        let resume_fid = self.shared.reserve();
        let gen_id = self.shared.generators.len() as u32;
        self.shared.generators.push(resume_fid);

        // ── resume function ──
        let resume_name = self.interner.intern(&format!("{synth}.<resume>"));
        {
            let mut rl = FnLowerer::new(
                self.interner,
                self.ctx,
                self.shared,
                resume_name,
                &synth,
                SemTy::Dyn,
                None,
            );
            let gen_name = rl.intern("__gen__");
            rl.add_param(gen_name, SemTy::Dyn);
            let iter0_name = rl.intern("__iter0__");
            let iter0 = rl.add_logical_local(iter0_name, SemTy::Dyn);
            rl.gen = Some(GenCtx {
                gen_local: LocalId::new(0),
                next_state: 1,
                resume_targets: Vec::new(),
            });
            let start = rl.new_block();
            rl.switch(start);
            rl.lower_genexpr_clauses(g, 0, iter0, span)?;
            if rl.cur_open() {
                rl.emit_gen_exhaust(span);
            }
            rl.gen_rewrite_locals();
            let num_locals = rl.locals.len() as u32 - 1;
            rl.build_gen_dispatch(start);
            let resume_fn = rl.finish(HirTerminator::Return(None));
            self.shared.fill(resume_fid, resume_fn);

            // ── wrapper(iter0) ──
            let mut wl = FnLowerer::new(
                self.interner,
                self.ctx,
                self.shared,
                name,
                &synth,
                SemTy::Dyn,
                None,
            );
            let p = wl.intern("__iter0__");
            wl.add_param(p, SemTy::Dyn);
            let g_local = wl.fresh_local(SemTy::Dyn);
            let mg = wl.alloc(
                HirExprKind::MakeGenerator { gen_id, num_locals },
                SemTy::Dyn,
                span,
            );
            wl.push_stmt(HirStmt::Assign {
                target: g_local,
                value: mg,
            });
            let gen = wl.local_ref(g_local, span);
            let p0 = wl.local_ref(LocalId::new(0), span);
            wl.push_stmt(HirStmt::GenSetLocal {
                gen,
                slot: 0,
                value: p0,
            });
            let g_ret = wl.local_ref(g_local, span);
            wl.seal(HirTerminator::Return(Some(g_ret)));
            let wrapper_fn = wl.finish(HirTerminator::Return(None));
            self.shared.fill(wrapper_fid, wrapper_fn);
        }

        // Call the synthetic wrapper with the eager iterable → the generator.
        let callee = self.alloc(
            HirExprKind::Name(SymbolRef::Unresolved(name)),
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![outer],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// Recurse over a genexpr's clauses building nested iterator loops; the
    /// innermost point yields the element (Phase 6E). The first clause iterates
    /// the eager `iter0` parameter; deeper iterables are lowered in place.
    pub(super) fn lower_genexpr_clauses(
        &mut self,
        g: &ExprGeneratorExp,
        idx: usize,
        iter0: LocalId,
        span: Span,
    ) -> Result<()> {
        if idx == g.generators.len() {
            let elt = self.lower_expr(g.elt.as_ref())?;
            self.suspend(Some(elt), false, span)?;
            return Ok(());
        }
        let comp = &g.generators[idx];
        let iterable = if idx == 0 {
            self.local_ref(iter0, span)
        } else {
            self.lower_expr(&comp.iter)?
        };
        let lp = self.begin_iter_loop(iterable, span)?;
        let elem = self.local_ref(lp.elem, span);
        self.bind_for_target(&comp.target, elem, span)?;
        for cond_expr in &comp.ifs {
            let cond = self.lower_expr(cond_expr)?;
            let cont = self.new_block();
            self.seal(HirTerminator::Branch {
                cond,
                then: cont,
                else_: lp.header,
            });
            self.switch(cont);
        }
        self.lower_genexpr_clauses(g, idx + 1, iter0, span)?;
        self.end_iter_loop(lp);
        Ok(())
    }

    /// A read of the generator object (the resume function's param 0).
    pub(super) fn gen_ref(&mut self, span: Span) -> Idx<HirExpr> {
        let g = self.gen.as_ref().expect("generator mode").gen_local;
        self.local_ref(g, span)
    }

    /// Lower a `yield e` / `yield from it` statement (the value is discarded on
    /// resume — Phase 6E).
    pub(super) fn lower_yield_stmt(&mut self, expr: &Expr) -> Result<()> {
        self.lower_yield_value(expr, false)?;
        Ok(())
    }

    /// Lower a yield expression as a suspend point. Returns the resumed sent
    /// value when `want_sent`. `yield from it` desugars to a for-loop of plain
    /// yields. `yield` / `yield e` suspend: evaluate the value, `SetState(k)`,
    /// return it; the resume block checks `IsClosing` (→ exhaust) then continues.
    pub(super) fn lower_yield_value(&mut self, expr: &Expr, want_sent: bool) -> Result<Option<Idx<HirExpr>>> {
        let span = to_span(expr.range());
        if let Expr::YieldFrom(yf) = expr {
            // `yield from sub` → `for __yf in sub: yield __yf` (StopIteration.value
            // and send-forwarding are out of scope — documented).
            let iterable = self.lower_expr(yf.value.as_ref())?;
            let lp = self.begin_iter_loop(iterable, span)?;
            let elem = self.local_ref(lp.elem, span);
            self.suspend(Some(elem), false, span)?;
            self.end_iter_loop(lp);
            return Ok(None);
        }
        let Expr::Yield(y) = expr else {
            return Err(parse_error("expected a yield expression", span));
        };
        let value = match &y.value {
            Some(e) => Some(self.lower_expr(e.as_ref())?),
            None => None,
        };
        self.suspend(value, want_sent, span)
    }

    /// Emit a suspend point: `SetState(k); Return(value)`, then a resume block
    /// that checks `IsClosing` and (if `want_sent`) reads the sent value.
    pub(super) fn suspend(
        &mut self,
        value: Option<Idx<HirExpr>>,
        want_sent: bool,
        span: Span,
    ) -> Result<Option<Idx<HirExpr>>> {
        // A `yield` inside a `try`/`with` is supported: the project unwinds via
        // table-based metadata (PC→handler), not a per-frame stack structure, so
        // a suspended frame has no live try-region state for a cross-stack resume
        // to dangle. Handler coverage falls out for free — the resume/`cont`
        // blocks below are created (and stamped) while `cur_handler` still names
        // the enclosing try handler, so an exception raised after resume unwinds
        // to it. A `yield` lexically inside an `except`/`finally` body is the one
        // rejected case (see `yield_in_except_or_finally`).
        let value = value.unwrap_or_else(|| self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span));
        let k = {
            let g = self.gen.as_mut().expect("generator mode");
            let k = g.next_state;
            g.next_state += 1;
            k
        };
        let gen = self.gen_ref(span);
        self.push_stmt(HirStmt::GenSetState { gen, state: k });
        self.seal(HirTerminator::Return(Some(value)));

        let resume = self.new_block();
        self.gen.as_mut().unwrap().resume_targets.push((k, resume));
        self.switch(resume);

        // `close()` resumes with `closing` set: unwind via GeneratorExit, then
        // exhaust and return None. The unwind runs the enclosing try-finally /
        // with cleanups (finally bodies, `__exit__`) before exhausting — see the
        // close path below.
        let gen2 = self.gen_ref(span);
        let closing = self.alloc(
            HirExprKind::GenQuery {
                op: GenOp::IsClosing,
                gen: gen2,
                imm: 0,
                value: None,
            },
            SemTy::Bool,
            span,
        );
        let close_b = self.new_block();
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: closing,
            then: close_b,
            else_: cont,
        });
        self.switch(close_b);
        // close() unwinds via GeneratorExit: run enclosing try-finally / with
        // cleanups (finally bodies, __exit__), then exhaust — mirrors the
        // generator `return` path. With no protected scopes this is exactly
        // emit_gen_exhaust, so plain generators are unaffected.
        self.with_exit_cleanups(0, span, |this| {
            this.emit_gen_exhaust(span);
            Ok(())
        })?;
        self.switch(cont);

        if want_sent {
            let gen3 = self.gen_ref(span);
            let sent = self.alloc(
                HirExprKind::GenQuery {
                    op: GenOp::GetSentValue,
                    gen: gen3,
                    imm: 0,
                    value: None,
                },
                SemTy::Dyn,
                span,
            );
            Ok(Some(sent))
        } else {
            Ok(None)
        }
    }

    /// Emit the generator's exhaust sequence: `SetExhausted; SetState(MAX);
    /// Return None`. Used at fallthrough / `return` / `close()`.
    pub(super) fn emit_gen_exhaust(&mut self, span: Span) {
        let gen = self.gen_ref(span);
        self.push_stmt(HirStmt::GenSetExhausted { gen });
        let gen2 = self.gen_ref(span);
        self.push_stmt(HirStmt::GenSetState {
            gen: gen2,
            state: u32::MAX,
        });
        let none = self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span);
        self.seal(HirTerminator::Return(Some(none)));
    }

    /// Rewrite every named/synthetic local access to generator-slot storage
    /// (P6-3): `Local(lid)` → `GenQuery(GetLocal, slot)`, `Assign{target}` →
    /// `GenSetLocal{slot}`. Local 0 (the generator param) is left untouched.
    /// Slot index = `lid - 1`; so `num_locals = locals.len() - 1`.
    pub(super) fn gen_rewrite_locals(&mut self) {
        let span = Span::dummy();
        let gen_local = self.gen.as_ref().unwrap().gen_local;
        debug_assert_eq!(gen_local.index(), 0);
        // Rewrite reads (`Local`) in place.
        let read_rewrites: Vec<(Idx<HirExpr>, u32)> = self
            .exprs
            .iter()
            .filter_map(|(idx, e)| match e.kind {
                HirExprKind::Local(lid) if lid.index() != 0 => Some((idx, lid.index() as u32 - 1)),
                _ => None,
            })
            .collect();
        for (idx, slot) in read_rewrites {
            let gen = self.alloc(HirExprKind::Local(gen_local), SemTy::Dyn, span);
            self.exprs[idx].kind = HirExprKind::GenQuery {
                op: GenOp::GetLocal,
                gen,
                imm: slot,
                value: None,
            };
        }
        // Rewrite writes (`Assign`) in place across every block.
        let block_ids: Vec<Idx<HirBlock>> = self.blocks.iter().map(|(b, _)| b).collect();
        for b in block_ids {
            let n = self.blocks[b].stmts.len();
            for i in 0..n {
                if let HirStmt::Assign { target, value } = self.blocks[b].stmts[i] {
                    if target.index() != 0 {
                        let slot = target.index() as u32 - 1;
                        let gen = self.alloc(HirExprKind::Local(gen_local), SemTy::Dyn, span);
                        self.blocks[b].stmts[i] = HirStmt::GenSetLocal { gen, slot, value };
                    }
                }
            }
        }
    }

    /// Build the entry dispatch (Phase 6E): a compare-chain on `GetState` routing
    /// state 0 → `start`, state k → its resume block, anything else → exhaust.
    /// Built AFTER `gen_rewrite_locals`, so its fresh `Local(gen)` reads survive.
    pub(super) fn build_gen_dispatch(&mut self, start: Idx<HirBlock>) {
        let span = Span::dummy();
        let mut chain: Vec<(u32, Idx<HirBlock>)> = vec![(0, start)];
        chain.extend(self.gen.as_ref().unwrap().resume_targets.iter().copied());
        let default_b = self.new_block();
        let mut block = self.entry;
        let len = chain.len();
        for (i, (state, target)) in chain.into_iter().enumerate() {
            self.switch(block);
            let gen = self.gen_ref(span);
            let s = self.alloc(
                HirExprKind::GenQuery {
                    op: GenOp::GetState,
                    gen,
                    imm: 0,
                    value: None,
                },
                SemTy::Int,
                span,
            );
            let k = self.alloc(HirExprKind::IntLit(state as i64), SemTy::Int, span);
            let cmp = self.alloc(
                HirExprKind::Compare {
                    op: CmpOp::Eq,
                    l: s,
                    r: k,
                },
                SemTy::Bool,
                span,
            );
            let next = if i + 1 < len {
                self.new_block()
            } else {
                default_b
            };
            self.seal(HirTerminator::Branch {
                cond: cmp,
                then: target,
                else_: next,
            });
            block = next;
        }
        self.switch(default_b);
        self.emit_gen_exhaust(span);
    }

}

/// True iff `expr` is a `yield` / `yield from` expression (Phase 6E).
pub(super) fn is_yield_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Yield(_) | Expr::YieldFrom(_))
}

/// True iff a statement list contains a `yield` not nested inside another
/// function / lambda scope (Phase 6E generator detection).
pub(super) fn body_has_yield(body: &[Stmt]) -> bool {
    body.iter().any(stmt_has_yield)
}

pub(super) fn stmt_has_yield(s: &Stmt) -> bool {
    match s {
        Stmt::Expr(e) => is_yield_expr(&e.value),
        Stmt::Assign(a) => is_yield_expr(&a.value),
        Stmt::AugAssign(a) => is_yield_expr(&a.value),
        Stmt::AnnAssign(a) => a.value.as_ref().is_some_and(|v| is_yield_expr(v)),
        Stmt::Return(r) => r.value.as_ref().is_some_and(|v| is_yield_expr(v)),
        Stmt::If(s) => body_has_yield(&s.body) || body_has_yield(&s.orelse),
        Stmt::While(s) => body_has_yield(&s.body) || body_has_yield(&s.orelse),
        Stmt::For(s) => body_has_yield(&s.body) || body_has_yield(&s.orelse),
        // Phase 7: a yield lexically inside try/with/match still makes the def
        // a generator (the suspend path then rejects try/with with a clear
        // message instead of "unsupported expression").
        Stmt::Try(t) => {
            body_has_yield(&t.body)
                || try_handlers(&t.handlers).any(|h| body_has_yield(&h.body))
                || body_has_yield(&t.orelse)
                || body_has_yield(&t.finalbody)
        }
        Stmt::With(w) => body_has_yield(&w.body),
        Stmt::Match(m) => m.cases.iter().any(|c| body_has_yield(&c.body)),
        // A nested def/lambda/class is its own scope — its yields don't count.
        _ => false,
    }
}

/// True if a `yield` appears lexically inside an `except` handler body or a
/// `finally` block (not descending into nested def/lambda/class). Out of scope:
/// a yield in an `except` body would suspend with the runtime's
/// `handling_exception` thread-local still set; a yield in a `finally` body
/// would be duplicated across the per-edge re-lowering of the finalbody. A
/// yield in a `try` body, `else` clause, or `with` body IS supported.
pub(super) fn yield_in_except_or_finally(body: &[Stmt]) -> bool {
    body.iter().any(stmt_yield_in_except_or_finally)
}

fn stmt_yield_in_except_or_finally(s: &Stmt) -> bool {
    match s {
        Stmt::Try(t) => {
            // A `yield` ANYWHERE inside a handler body or the finalbody is the
            // rejected case; the try body and `else` clause descend normally.
            try_handlers(&t.handlers).any(|h| body_has_yield(&h.body))
                || body_has_yield(&t.finalbody)
                || yield_in_except_or_finally(&t.body)
                || yield_in_except_or_finally(&t.orelse)
        }
        Stmt::If(s) => {
            yield_in_except_or_finally(&s.body) || yield_in_except_or_finally(&s.orelse)
        }
        Stmt::While(s) => {
            yield_in_except_or_finally(&s.body) || yield_in_except_or_finally(&s.orelse)
        }
        Stmt::For(s) => {
            yield_in_except_or_finally(&s.body) || yield_in_except_or_finally(&s.orelse)
        }
        Stmt::With(w) => yield_in_except_or_finally(&w.body),
        Stmt::Match(m) => m.cases.iter().any(|c| yield_in_except_or_finally(&c.body)),
        // A nested def/lambda/class is its own scope — its yields don't count.
        _ => false,
    }
}

/// Lower a generator `def` (Phase 6E): a wrapper (`fid`) building the generator
/// and storing its params/captures into slots, plus a `<resume>` state machine
/// registered in `shared.generators`.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_generator_def(
    interner: &mut StringInterner,
    ctx: &AnnCtx,
    shared: &mut Shared,
    body: &[Stmt],
    name_str: &str,
    name: InternedString,
    wrapper_fid: FuncId,
    parsed: &ParsedParams,
    ret_ty: SemTy,
    enclosing: Option<ClassId>,
    is_nested: bool,
) -> Result<()> {
    let span = Span::dummy();
    // A `yield` in the body of an `except` handler or a `finally` block is out
    // of scope (a try/with body or `else` clause is supported) — reject cleanly
    // rather than miscompile.
    if yield_in_except_or_finally(body) {
        return Err(parse_error(
            "yield inside an `except` or `finally` block of a generator is out of \
             scope (Phase 6E); yield in a `try` body or `with` body is supported",
            span,
        ));
    }
    let n_params = parsed.fixed.len() + parsed.kwonly.len();

    // ── resume function: the state machine ──
    let resume_fid = shared.reserve();
    let gen_id = shared.generators.len() as u32;
    shared.generators.push(resume_fid);

    let resume_name = interner.intern(&format!("{name_str}.<resume>"));
    let mut rl = FnLowerer::new(
        interner,
        ctx,
        shared,
        resume_name,
        name_str,
        SemTy::Dyn,
        enclosing,
    );
    // Param 0 = the generator object.
    let gen_name = rl.intern("__gen__");
    rl.add_param(gen_name, SemTy::Dyn);
    // The Python params become *logical locals* (gen slots), bound by name so
    // the body resolves to them (slots 0.. = locals 1..).
    for p in parsed.fixed.iter().chain(&parsed.kwonly) {
        rl.add_logical_local(p.name, p.ty.clone());
    }
    rl.gen = Some(GenCtx {
        gen_local: LocalId::new(0),
        next_state: 1,
        resume_targets: Vec::new(),
    });

    let start = rl.new_block();
    rl.switch(start);
    rl.lower_body(body)?;
    // Fallthrough → exhaust.
    if rl.cur_open() {
        rl.emit_gen_exhaust(span);
    }
    rl.gen_rewrite_locals();
    let num_locals = rl.locals.len() as u32 - 1;
    rl.build_gen_dispatch(start);
    let resume_fn = rl.finish(HirTerminator::Return(None));
    shared.fill(resume_fid, resume_fn);

    // ── wrapper: build the generator, seed param slots, return it ──
    let mut wl = FnLowerer::new(interner, ctx, shared, name, name_str, ret_ty, enclosing);
    // A nested generator wrapper crosses the ONE nested-call ABI (PITFALLS A4):
    // it gets a synthetic `__env__: Dyn` at param 0 (mirroring `lower_callable`'s
    // non-generator nested branch), so `closure_sem_ty` (`params[1..]`) and
    // `uniform_thunk_over_nested` (base = 1) are correct and the empty capture env
    // from `make_closure_expr` lands harmlessly in `__env__`. A top-level
    // generator (or a generator method) keeps base 0 (no env param).
    let base = if is_nested {
        let env = wl.intern("__env__");
        wl.add_param(env, SemTy::Dyn); // LocalId(0)
        1
    } else {
        0
    };
    wl.install_params(parsed); // Python params now at LocalId(base..)
    let g_local = wl.fresh_local(SemTy::Dyn);
    let mg = wl.alloc(
        HirExprKind::MakeGenerator { gen_id, num_locals },
        SemTy::Dyn,
        span,
    );
    wl.push_stmt(HirStmt::Assign {
        target: g_local,
        value: mg,
    });
    for i in 0..n_params {
        let gen = wl.local_ref(g_local, span);
        // The gen slot index is unshifted (the resume state machine numbers its
        // logical locals from 0); only the positional param *read* shifts past
        // the synthetic `__env__`.
        let p = wl.local_ref(LocalId::new((i + base) as u32), span);
        wl.push_stmt(HirStmt::GenSetLocal {
            gen,
            slot: i as u32,
            value: p,
        });
    }
    let g_ret = wl.local_ref(g_local, span);
    wl.seal(HirTerminator::Return(Some(g_ret)));
    let wrapper_fn = wl.finish(HirTerminator::Return(None));
    shared.fill(wrapper_fid, wrapper_fn);
    Ok(())
}


