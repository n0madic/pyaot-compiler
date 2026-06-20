use super::*;

impl<'a> FnLowerer<'a> {
    // ── statements ──────────────────────────────────────────────────────────

    /// Lower a statement list, stopping after a statement that terminates the
    /// current block (so trailing dead code is not emitted into a sealed block).
    pub(super) fn lower_body(&mut self, stmts: &[Stmt]) -> Result<()> {
        for stmt in stmts {
            if self.lower_stmt(stmt)? {
                break;
            }
        }
        Ok(())
    }

    /// Lower one statement. Returns `true` if it terminated the current block
    /// (`break` / `continue` / `return`).
    pub(super) fn lower_stmt(&mut self, stmt: &Stmt) -> Result<bool> {
        // Real tracebacks: establish this statement's source line in the
        // current block before any of its code is emitted.
        self.mark_line(to_span(stmt.range()));
        match stmt {
            Stmt::Expr(s) => {
                // `print(...)` is the one special statement (it carries sep/end).
                if let Some(call) = as_print_call(s.value.as_ref()) {
                    self.lower_print(call)?;
                } else if self.gen.is_some() && is_yield_expr(s.value.as_ref()) {
                    // A bare `yield e` / `yield from it` statement (Phase 6E).
                    self.lower_yield_stmt(s.value.as_ref())?;
                } else {
                    let idx = self.lower_expr(s.value.as_ref())?;
                    self.push_stmt(HirStmt::Expr(idx));
                }
                Ok(false)
            }
            Stmt::Assign(a) => {
                self.lower_assign(a)?;
                Ok(false)
            }
            Stmt::AugAssign(a) => {
                self.lower_augassign(a)?;
                Ok(false)
            }
            Stmt::AnnAssign(a) => {
                self.lower_annassign(a)?;
                Ok(false)
            }
            Stmt::If(s) => {
                self.lower_if(s)?;
                Ok(false)
            }
            Stmt::While(s) => self.lower_while(s),
            Stmt::For(s) => self.lower_for(s),
            Stmt::Assert(s) => {
                // `assert cond, msg` desugars to `if not cond: raise
                // AssertionError(msg)` so the message survives (Phase 7);
                // a bare `assert cond` keeps the lean AssertFail path.
                if let Some(msg) = &s.msg {
                    let span = to_span(s.range());
                    let cond = self.lower_expr(s.test.as_ref())?;
                    let fail_b = self.new_block();
                    let ok_b = self.new_block();
                    self.seal(HirTerminator::Branch {
                        cond,
                        then: ok_b,
                        else_: fail_b,
                    });
                    self.switch(fail_b);
                    let m = self.lower_expr(msg.as_ref())?;
                    self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
                        tag: pyaot_core_defs::BuiltinExceptionKind::AssertionError.tag(),
                        msg: Some(m),
                    }));
                    self.seal(HirTerminator::Unreachable);
                    self.switch(ok_b);
                    let _ = span;
                } else {
                    let cond = self.lower_expr(s.test.as_ref())?;
                    self.push_stmt(HirStmt::Assert { cond });
                }
                Ok(false)
            }
            // ── exceptions / with / match (Phase 7) ──
            Stmt::Try(t) => self.lower_try(t),
            Stmt::Raise(r) => self.lower_raise(r),
            Stmt::With(w) => self.lower_with(w),
            Stmt::Match(m) => self.lower_match(m),
            Stmt::Pass(_) => Ok(false),
            // `from typing import ...` / `from __future__ import ...` are
            // type-level only (no runtime effect in our subset) — accept as no-ops
            // so generics (TypeVar/Generic) compile. Real imports are processed at
            // module top level (`lower_module_into`'s import scan); reaching here
            // means the import is nested — inside a function body or a top-level
            // `if`/`try` block. Those are rejected: the load DFS precomputes each
            // module's `<init>` order in source order, so a conditionally-executed
            // import has no place in that schedule yet (Phase 8 limitation — a
            // top-level guarded `import` / optional-dependency pattern must be
            // hoisted to an unconditional top-level import).
            Stmt::ImportFrom(i) => {
                let module = i.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                if matches!(module, "typing" | "__future__" | "typing_extensions") {
                    Ok(false)
                } else {
                    Err(parse_error(
                        "only module-top-level imports are supported (an import inside \
                         a function or a conditional block is out of scope)",
                        to_span(i.range()),
                    ))
                }
            }
            Stmt::Import(i) => {
                if i.names
                    .iter()
                    .all(|n| matches!(n.name.as_str(), "typing" | "typing_extensions"))
                {
                    Ok(false)
                } else {
                    Err(parse_error(
                        "only module-top-level imports are supported (an import inside \
                         a function or a conditional block is out of scope)",
                        to_span(i.range()),
                    ))
                }
            }
            Stmt::Break(b) => {
                let span = to_span(b.range());
                let loop_idx = self
                    .innermost_loop()
                    .ok_or_else(|| parse_error("'break' outside loop", span))?;
                let ScopeCtx::Loop { break_to, .. } = self.scope_stack[loop_idx] else {
                    unreachable!()
                };
                self.with_exit_cleanups(loop_idx + 1, span, |this| {
                    this.seal(HirTerminator::Jump(break_to));
                    Ok(())
                })?;
                Ok(true)
            }
            Stmt::Continue(c) => {
                let span = to_span(c.range());
                let loop_idx = self
                    .innermost_loop()
                    .ok_or_else(|| parse_error("'continue' outside loop", span))?;
                let ScopeCtx::Loop { continue_to, .. } = self.scope_stack[loop_idx] else {
                    unreachable!()
                };
                self.with_exit_cleanups(loop_idx + 1, span, |this| {
                    this.seal(HirTerminator::Jump(continue_to));
                    Ok(())
                })?;
                Ok(true)
            }
            Stmt::Return(r) => {
                let span = to_span(r.range());
                // In a generator, `return` ends the generator (exhaust). The
                // returned value (StopIteration.value) is out of scope (6E).
                if self.gen.is_some() {
                    if let Some(e) = &r.value {
                        let _ = self.lower_expr(e.as_ref())?;
                    }
                    self.with_exit_cleanups(0, span, |this| {
                        this.emit_gen_exhaust(span);
                        Ok(())
                    })?;
                    return Ok(true);
                }
                if self
                    .scope_stack
                    .iter()
                    .all(|s| matches!(s, ScopeCtx::Loop { .. }))
                {
                    // Fast path: no protected regions to clean up.
                    let val = match &r.value {
                        Some(e) => Some(self.lower_expr(e.as_ref())?),
                        None => None,
                    };
                    self.seal(HirTerminator::Return(val));
                    return Ok(true);
                }
                // Evaluate the return value BEFORE the cleanups (CPython order),
                // snapshotting it to a temp the cleanups cannot disturb.
                let val = match &r.value {
                    Some(e) => {
                        let v = self.lower_expr(e.as_ref())?;
                        let tmp = self.fresh_local(SemTy::Dyn);
                        self.push_stmt(HirStmt::Assign {
                            target: tmp,
                            value: v,
                        });
                        Some(tmp)
                    }
                    None => None,
                };
                self.with_exit_cleanups(0, span, |this| {
                    let val = val.map(|tmp| this.local_ref(tmp, span));
                    this.seal(HirTerminator::Return(val));
                    Ok(())
                })?;
                Ok(true)
            }
            // Nested `def` (Phase 6A): a flat synthetic function plus a closure
            // value bound to the def's name in this scope.
            Stmt::FunctionDef(d) => {
                self.lower_nested_def(d)?;
                Ok(false)
            }
            // Nested `class` (FIX 2): registered + lowered in the module pre-scan
            // and resolved at use sites through `class_map`, so this arm only
            // validates the decorations/defaults the nested path can't express
            // and emits no code (no local binding needed).
            Stmt::ClassDef(c) => {
                self.lower_nested_classdef(c)?;
                Ok(false)
            }
            // Binding-analysis inputs only (Phase 6B): the declarations were
            // consumed by `freevars` / the module pre-scan; nothing to emit.
            Stmt::Global(_) | Stmt::Nonlocal(_) => Ok(false),
            Stmt::Delete(d) => self.lower_delete(d),
            // A PEP 695 `type X = T` alias is a compile-time annotation binding
            // (collected in the module pre-scan into `type_aliases`); it emits no
            // runtime code.
            Stmt::TypeAlias(_) => Ok(false),
            other => Err(parse_error(
                "unsupported statement for this milestone",
                to_span(other.range()),
            )),
        }
    }

    /// `a = b = value` — evaluate `value` once, assign to each target (a `Name` or
    /// a subscript `base[index]`).
    pub(super) fn lower_assign(&mut self, a: &rustpython_parser::ast::StmtAssign) -> Result<()> {
        // `x = yield e` inside a generator (Phase 6E): suspend, then bind the
        // sent value resuming here. Only a single simple-name target is in scope.
        if self.gen.is_some() && is_yield_expr(a.value.as_ref()) && a.targets.len() == 1 {
            if let Expr::Name(n) = &a.targets[0] {
                let span = to_span(a.range());
                let sent = self.lower_yield_value(a.value.as_ref(), true)?;
                let sent = sent.expect("x = yield yields a sent value");
                let name = self.intern(n.id.as_str());
                self.write_named(name, SemTy::Dyn, sent);
                let _ = span;
                return Ok(());
            }
        }
        // Tuple/list unpacking target: `a, b = …` / `a, b = c, d`.
        if a.targets.len() == 1 {
            if let Some(targets) = seq_target_elts(&a.targets[0]) {
                let span = to_span(a.range());
                // A literal sequence RHS unpacks element-wise with a static arity
                // check and no intermediate tuple; anything else — including a
                // starred target over a literal (`a, *rest = [1, 2, 3]`) —
                // stages a value and reads it back positionally.
                let has_star = targets.iter().any(|t| matches!(t, Expr::Starred(_)));
                if !has_star {
                    if let Some(values) = seq_target_elts(a.value.as_ref()) {
                        if targets.len() != values.len() {
                            return Err(parse_error(
                                format!(
                                    "cannot unpack: expected {} value(s), got {}",
                                    targets.len(),
                                    values.len()
                                ),
                                span,
                            ));
                        }
                        return self.lower_unpack_literal(targets, values, span);
                    }
                }
                let value = self.lower_expr(a.value.as_ref())?;
                return self.lower_unpack_subscript(targets, value, span);
            }
        }
        let value = self.lower_expr(a.value.as_ref())?;
        if a.targets.len() == 1 {
            return self.assign_to_target(&a.targets[0], value);
        }
        // Multiple targets: stage the value once, then fan out.
        let span = to_span(a.value.range());
        let tmp = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign { target: tmp, value });
        for target in &a.targets {
            let v = self.local_ref(tmp, span);
            self.assign_to_target(target, v)?;
        }
        Ok(())
    }

    /// Bind `value` to one assignment target: a simple name (`x = …`) or a
    /// subscript write (`a[i] = …` → [`HirStmt::SetItem`]).
    pub(super) fn assign_to_target(&mut self, target: &Expr, value: Idx<HirExpr>) -> Result<()> {
        match target {
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                self.write_named(name, SemTy::Dyn, value);
                Ok(())
            }
            Expr::Subscript(s) => {
                let span = to_span(s.range());
                if matches!(s.slice.as_ref(), Expr::Slice(_)) {
                    return Err(parse_error("slice assignment is not yet supported", span));
                }
                // `os.environ[k] = v` (Phase 8H): a SetItem into the environ
                // attr would write into a FRESH dict snapshot (the getter
                // rebuilds it on every read) and be silently lost. Route to the
                // `rt_os_environ_set` setter, which mutates the real process
                // environment.
                // TODO: `del os.environ[k]` needs a delete-subscript statement
                // path (none exists yet) plus an `rt_os_environ_del` (remove_var).
                if let Some((leftmost, dotted)) = flatten_attr_chain(s.value.as_ref()) {
                    let lname = self.intern(leftmost);
                    if dotted == "os.environ"
                        && self.ctx.stdlib.aliases.contains(leftmost)
                        && !self.scope.contains_key(&lname)
                    {
                        let key = self.lower_expr(s.slice.as_ref())?;
                        let call = self.alloc(
                            HirExprKind::CallRuntime {
                                target: pyaot_hir::RuntimeCallTarget::Func(
                                    &pyaot_stdlib_defs::modules::os::OS_ENVIRON_SET,
                                ),
                                args: vec![Some(key), Some(value)],
                                provided: 2,
                            },
                            SemTy::NoneTy,
                            span,
                        );
                        self.push_stmt(HirStmt::Expr(call));
                        return Ok(());
                    }
                }
                let base = self.lower_expr(s.value.as_ref())?;
                let index = self.lower_expr(s.slice.as_ref())?;
                self.push_stmt(HirStmt::SetItem { base, index, value });
                Ok(())
            }
            Expr::Attribute(attr) => {
                let base = self.lower_expr(attr.value.as_ref())?;
                let name = self.intern(attr.attr.as_str());
                self.push_stmt(HirStmt::SetAttr { base, name, value });
                Ok(())
            }
            // Nested sequence target (`a, (b, c) = …`): stage this element and
            // re-subscript it positionally, recursing for deeper nesting. Routes
            // through the same unpacker as the top level, so for-loop and
            // comprehension targets get nested support for free (backlog §4).
            Expr::Tuple(t) => {
                let span = to_span(target.range());
                self.lower_unpack_subscript(&t.elts, value, span)
            }
            Expr::List(l) => {
                let span = to_span(target.range());
                self.lower_unpack_subscript(&l.elts, value, span)
            }
            other => Err(parse_error(
                "unsupported assignment target",
                to_span(other.range()),
            )),
        }
    }

    /// Lower a `del` statement (`del d[k]`, `del li[i]`, `del name`,
    /// `del obj.attr`, and multi-target `del a, b`). Each target is unbound
    /// independently, mirroring [`Self::assign_to_target`]:
    /// - a subscript → [`HirStmt::DelItem`] (a runtime element delete);
    /// - a name → stores the `Value::UNBOUND` sentinel into the slot (marking a
    ///   local deletable + pinned-tagged, or recording a global), so any later
    ///   read raises `UnboundLocalError`/`NameError` via the read-guard;
    /// - an attribute → stores `UNBOUND` into the field slot (recording the
    ///   field name deletable), so a later read raises `AttributeError`.
    pub(super) fn lower_delete(&mut self, d: &StmtDelete) -> Result<bool> {
        for target in &d.targets {
            self.delete_target(target)?;
        }
        Ok(false)
    }

    /// Unbind one `del` target. See [`Self::lower_delete`].
    pub(super) fn delete_target(&mut self, target: &Expr) -> Result<()> {
        match target {
            Expr::Subscript(s) => {
                let span = to_span(s.range());
                if matches!(s.slice.as_ref(), Expr::Slice(_)) {
                    return Err(parse_error("slice deletion is not supported", span));
                }
                let base = self.lower_expr(s.value.as_ref())?;
                let index = self.lower_expr(s.slice.as_ref())?;
                self.push_stmt(HirStmt::DelItem { base, index });
                Ok(())
            }
            Expr::Name(n) => {
                let span = to_span(n.range());
                let name = self.intern(n.id.as_str());
                let unbound = self.alloc(HirExprKind::Unbound, SemTy::Never, span);
                match self.resolve_write_place(name, SemTy::Dyn) {
                    Place::Bind(Binding::Direct(lid)) => {
                        // Keep the name bound (CPython keeps it in co_varnames);
                        // store the sentinel and pin the slot Tagged so the
                        // immediate fits regardless of the inferred type.
                        let l = &mut self.locals[lid.index()];
                        l.deletable = true;
                        l.pin_tagged = true;
                        self.push_stmt(HirStmt::Assign {
                            target: lid,
                            value: unbound,
                        });
                        Ok(())
                    }
                    Place::Bind(Binding::Cell(_)) => Err(parse_error(
                        "del of a captured (nonlocal/closure) variable is not supported",
                        span,
                    )),
                    Place::Global(var_id) => {
                        self.shared.deletable_globals.insert(var_id, name);
                        self.push_stmt(HirStmt::GlobalSet {
                            var_id,
                            value: unbound,
                        });
                        Ok(())
                    }
                }
            }
            Expr::Attribute(attr) => {
                let span = to_span(attr.range());
                let base = self.lower_expr(attr.value.as_ref())?;
                let name = self.intern(attr.attr.as_str());
                let unbound = self.alloc(HirExprKind::Unbound, SemTy::Never, span);
                self.shared.deletable_fields.insert(name);
                self.push_stmt(HirStmt::SetAttr {
                    base,
                    name,
                    value: unbound,
                });
                Ok(())
            }
            // `del (a, b)` / `del [a, b]` — parenthesized/bracketed multi-target
            // (the bare `del a, b` form is split into separate targets by the
            // parser and handled in `lower_delete`).
            Expr::Tuple(t) => {
                for elt in &t.elts {
                    self.delete_target(elt)?;
                }
                Ok(())
            }
            Expr::List(l) => {
                for elt in &l.elts {
                    self.delete_target(elt)?;
                }
                Ok(())
            }
            other => Err(parse_error(
                "unsupported del target",
                to_span(other.range()),
            )),
        }
    }

    /// Unpack a literal-sequence RHS (`a, b = e0, e1`): stage every RHS value
    /// first (so `a, b = b, a` swaps correctly), then bind each target — no
    /// intermediate tuple allocation.
    pub(super) fn lower_unpack_literal(
        &mut self,
        targets: &[Expr],
        values: &[Expr],
        span: Span,
    ) -> Result<()> {
        reject_starred(targets, span)?;
        let mut staged = Vec::with_capacity(values.len());
        for v in values {
            let vv = self.lower_expr(v)?;
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: vv,
            });
            staged.push(tmp);
        }
        for (target, tmp) in targets.iter().zip(staged) {
            let v = self.local_ref(tmp, span);
            self.assign_to_target(target, v)?;
        }
        Ok(())
    }

    /// Unpack an arbitrary iterable RHS (`a, b = expr`, `for k, v in pairs`): stage
    /// the value once, validate its arity, then bind `target_i = tmp[i]` via
    /// positional subscripts. One starred target (`a, *rest = …`) captures a fresh
    /// list of the middle slice. A nested sequence target recurses here via
    /// [`Self::assign_to_target`] (`a, (b, c) = …`, backlog §4). Arity is checked
    /// against `len(tmp)` up front by [`Self::emit_unpack_guard`], raising the exact
    /// CPython `ValueError` ("too many values to unpack" / "not enough values to
    /// unpack") — a non-starred pattern requires an exact count, a starred one a
    /// minimum.
    pub(super) fn lower_unpack_subscript(
        &mut self,
        targets: &[Expr],
        value: Idx<HirExpr>,
        span: Span,
    ) -> Result<()> {
        let star_pos = targets.iter().position(|t| matches!(t, Expr::Starred(_)));
        if targets
            .iter()
            .enumerate()
            .any(|(i, t)| matches!(t, Expr::Starred(_)) && Some(i) != star_pos)
        {
            return Err(parse_error("multiple starred targets in unpacking", span));
        }
        let tmp = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign { target: tmp, value });

        let (prefix, suffix): (&[Expr], &[Expr]) = match star_pos {
            Some(p) => (&targets[..p], &targets[p + 1..]),
            None => (targets, &[]),
        };

        // n = len(tmp), staged once: the arity guard, the star slice, and the
        // suffix back-indices all read it.
        let tmp_ref = self.local_ref(tmp, span);
        let len_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Len,
                args: vec![tmp_ref],
            },
            SemTy::Int,
            span,
        );
        let len_l = self.fresh_local(SemTy::Int);
        self.push_stmt(HirStmt::Assign {
            target: len_l,
            value: len_e,
        });
        self.emit_unpack_guard(len_l, prefix.len(), suffix.len(), star_pos.is_some(), span);

        for (i, target) in prefix.iter().enumerate() {
            let tmp_ref = self.local_ref(tmp, span);
            let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
            let sub = self.alloc(
                HirExprKind::Subscript {
                    base: tmp_ref,
                    index: idx,
                },
                SemTy::Dyn,
                span,
            );
            self.assign_to_target(target, sub)?;
        }
        let Some(p) = star_pos else { return Ok(()) };

        // *rest = tmp[p .. n - m] as a fresh list (len_l already staged above).
        let Expr::Starred(st) = &targets[p] else {
            unreachable!()
        };
        let lo = self.alloc(HirExprKind::IntLit(p as i64), SemTy::Int, span);
        let len_ref = self.local_ref(len_l, span);
        let m_lit = self.alloc(HirExprKind::IntLit(suffix.len() as i64), SemTy::Int, span);
        let hi = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Sub,
                l: len_ref,
                r: m_lit,
            },
            SemTy::Dyn,
            span,
        );
        let rest = self.build_sublist(tmp, lo, hi, span)?;
        let rest_ref = self.local_ref(rest, span);
        self.assign_to_target(st.value.as_ref(), rest_ref)?;

        // Suffix targets: tmp[n - (m - j)].
        for (j, target) in suffix.iter().enumerate() {
            let len_ref = self.local_ref(len_l, span);
            let back = self.alloc(
                HirExprKind::IntLit((suffix.len() - j) as i64),
                SemTy::Int,
                span,
            );
            let idx = self.alloc(
                HirExprKind::BinOp {
                    op: BinOp::Sub,
                    l: len_ref,
                    r: back,
                },
                SemTy::Dyn,
                span,
            );
            let tmp_ref = self.local_ref(tmp, span);
            let sub = self.alloc(
                HirExprKind::Subscript {
                    base: tmp_ref,
                    index: idx,
                },
                SemTy::Dyn,
                span,
            );
            self.assign_to_target(target, sub)?;
        }
        Ok(())
    }

    /// Emit the CPython arity check for a runtime-value unpack: compare the staged
    /// `len(tmp)` (`len_l`) against the target pattern, raising the exact
    /// `ValueError` on a mismatch and falling through to a fresh block on success.
    /// A non-starred pattern requires exactly `prefix` values; a starred one
    /// requires at least `prefix + suffix` (the star absorbs any excess, so there
    /// is no upper bound). Mirrors the match-pattern length guard in `patterns.rs`.
    pub(super) fn emit_unpack_guard(
        &mut self,
        len_l: LocalId,
        prefix: usize,
        suffix: usize,
        has_star: bool,
        span: Span,
    ) {
        let expected = prefix + suffix;
        if has_star {
            // n < expected → "not enough values to unpack (expected at least E, got N)".
            let len_ref = self.local_ref(len_l, span);
            let need = self.alloc(HirExprKind::IntLit(expected as i64), SemTy::Int, span);
            let lt = self.alloc(
                HirExprKind::Compare {
                    op: CmpOp::Lt,
                    l: len_ref,
                    r: need,
                },
                SemTy::Bool,
                span,
            );
            let fail = self.new_block();
            let cont = self.new_block();
            self.seal(HirTerminator::Branch {
                cond: lt,
                then: fail,
                else_: cont,
            });
            self.switch(fail);
            self.raise_unpack_value_error(
                &format!("not enough values to unpack (expected at least {expected}, got "),
                len_l,
                span,
            );
            self.switch(cont);
            return;
        }
        // Non-starred: n > expected → too many; n < expected → not enough.
        let len_ref = self.local_ref(len_l, span);
        let exp = self.alloc(HirExprKind::IntLit(expected as i64), SemTy::Int, span);
        let gt = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Gt,
                l: len_ref,
                r: exp,
            },
            SemTy::Bool,
            span,
        );
        let too_many = self.new_block();
        let check_few = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: gt,
            then: too_many,
            else_: check_few,
        });

        // too many: "too many values to unpack (expected E, got N)".
        self.switch(too_many);
        self.raise_unpack_value_error(
            &format!("too many values to unpack (expected {expected}, got "),
            len_l,
            span,
        );

        self.switch(check_few);
        let len_ref = self.local_ref(len_l, span);
        let exp = self.alloc(HirExprKind::IntLit(expected as i64), SemTy::Int, span);
        let lt = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Lt,
                l: len_ref,
                r: exp,
            },
            SemTy::Bool,
            span,
        );
        let too_few = self.new_block();
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: lt,
            then: too_few,
            else_: cont,
        });
        self.switch(too_few);
        self.raise_unpack_value_error(
            &format!("not enough values to unpack (expected {expected}, got "),
            len_l,
            span,
        );
        self.switch(cont);
    }

    /// Raise `ValueError` with the runtime message `prefix + str(len_l) + ")"`,
    /// embedding the actual length so the wording matches CPython byte-for-byte.
    /// Seals the current block as unreachable; the caller switches to the success
    /// edge afterwards.
    fn raise_unpack_value_error(&mut self, prefix: &str, len_l: LocalId, span: Span) {
        let pre_id = self.intern(prefix);
        let pre = self.alloc(HirExprKind::StrLit(pre_id), SemTy::Str, span);
        let len_ref = self.local_ref(len_l, span);
        let got = self.call_builtin1("str", len_ref, span);
        let close_id = self.intern(")");
        let close = self.alloc(HirExprKind::StrLit(close_id), SemTy::Str, span);
        let msg = self.concat_str_parts(vec![pre, got, close], span);
        self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
            tag: pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            msg: Some(msg),
        }));
        self.seal(HirTerminator::Unreachable);
    }

    pub(super) fn lower_augassign(&mut self, a: &rustpython_parser::ast::StmtAugAssign) -> Result<()> {
        let span = to_span(a.range());
        // `x |= y` desugars to `x = x | y`, but `|=` must mutate in place for
        // `dict`/`set` (alias semantics) — route it to the in-place `IOr` op so
        // the runtime can return the same object (binary `|` keeps `BitOr` =
        // new object). `&=` / `-=` / `^=` get the same in-place treatment for
        // `set` operands (`IAnd`/`ISub`/`IXor`) so an alias observes the mutation;
        // every non-set (numeric) operand still delegates to the new-object path
        // inside the runtime. Type-blind: the runtime decides per operand tag.
        let op = match binop_from_ast(&a.op) {
            BinOp::BitOr => BinOp::IOr,
            BinOp::BitAnd => BinOp::IAnd,
            BinOp::Sub => BinOp::ISub,
            BinOp::BitXor => BinOp::IXor,
            other => other,
        };
        match a.target.as_ref() {
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                let place = self.resolve_write_place(name, SemTy::Dyn);
                let l = self.read_place(place, span);
                let r = self.lower_expr(a.value.as_ref())?;
                let combined = self.alloc(HirExprKind::BinOp { op, l, r }, SemTy::Dyn, span);
                self.write_place(place, combined);
                Ok(())
            }
            // `base.attr op= value` — evaluate `base` once, then read/modify/write.
            Expr::Attribute(attr) => {
                let name = self.intern(attr.attr.as_str());
                // A bare-`Name` base (a local, parameter, class name, or module)
                // has NO side effects, so evaluate it twice (read + write) rather
                // than binding it to a temp. The temp path lowers the base as a
                // VALUE — which rejects a class name (`ClassName.attr op= v`:
                // `Symbol::Class` is not a value, the L72 `Tracker.total += 1`
                // blocker). Embedding the `Name` in `Attribute`/`SetAttr` instead
                // routes a class-name base through the class-attribute path
                // (`GetClassAttr`/`SetClassAttr`), exactly as the plain
                // `ClassName.attr` read / `ClassName.attr = v` write already do.
                if matches!(attr.value.as_ref(), Expr::Name(_)) {
                    let read_base = self.lower_expr(attr.value.as_ref())?;
                    let cur = self.alloc(
                        HirExprKind::Attribute {
                            value: read_base,
                            name,
                        },
                        SemTy::Dyn,
                        span,
                    );
                    let r = self.lower_expr(a.value.as_ref())?;
                    let combined =
                        self.alloc(HirExprKind::BinOp { op, l: cur, r }, SemTy::Dyn, span);
                    let write_base = self.lower_expr(attr.value.as_ref())?;
                    self.push_stmt(HirStmt::SetAttr {
                        base: write_base,
                        name,
                        value: combined,
                    });
                    return Ok(());
                }
                let base_e = self.lower_expr(attr.value.as_ref())?;
                let base_tmp = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: base_tmp,
                    value: base_e,
                });
                let read_base = self.local_ref(base_tmp, span);
                let cur = self.alloc(
                    HirExprKind::Attribute {
                        value: read_base,
                        name,
                    },
                    SemTy::Dyn,
                    span,
                );
                let r = self.lower_expr(a.value.as_ref())?;
                let combined = self.alloc(HirExprKind::BinOp { op, l: cur, r }, SemTy::Dyn, span);
                let write_base = self.local_ref(base_tmp, span);
                self.push_stmt(HirStmt::SetAttr {
                    base: write_base,
                    name,
                    value: combined,
                });
                Ok(())
            }
            // `base[index] op= value` — evaluate `base` and `index` once.
            Expr::Subscript(s) => {
                if matches!(s.slice.as_ref(), Expr::Slice(_)) {
                    return Err(parse_error(
                        "slice augmented assignment is not supported",
                        span,
                    ));
                }
                let base_e = self.lower_expr(s.value.as_ref())?;
                let base_tmp = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: base_tmp,
                    value: base_e,
                });
                let idx_e = self.lower_expr(s.slice.as_ref())?;
                let idx_tmp = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: idx_tmp,
                    value: idx_e,
                });
                let read_base = self.local_ref(base_tmp, span);
                let read_idx = self.local_ref(idx_tmp, span);
                let cur = self.alloc(
                    HirExprKind::Subscript {
                        base: read_base,
                        index: read_idx,
                    },
                    SemTy::Dyn,
                    span,
                );
                let r = self.lower_expr(a.value.as_ref())?;
                let combined = self.alloc(HirExprKind::BinOp { op, l: cur, r }, SemTy::Dyn, span);
                let write_base = self.local_ref(base_tmp, span);
                let write_idx = self.local_ref(idx_tmp, span);
                self.push_stmt(HirStmt::SetItem {
                    base: write_base,
                    index: write_idx,
                    value: combined,
                });
                Ok(())
            }
            other => Err(parse_error(
                "unsupported augmented-assignment target",
                to_span(other.range()),
            )),
        }
    }

    pub(super) fn lower_annassign(&mut self, a: &rustpython_parser::ast::StmtAnnAssign) -> Result<()> {
        // `X: TypeAlias = T` (PEP 613) is a compile-time alias binding (collected
        // in the module pre-scan into `type_aliases`), NOT a value assignment: the
        // RHS `T` is a type, not a runtime value, so do not lower it.
        if matches!(a.annotation.as_ref(), Expr::Name(n) if n.id.as_str() == "TypeAlias") {
            return Ok(());
        }
        match a.target.as_ref() {
            Expr::Name(n) => {
                let ty = annotation_to_semty(a.annotation.as_ref(), self.ctx);
                let name = self.intern(n.id.as_str());
                let place = self.resolve_write_place(name, ty);
                if let Some(value) = &a.value {
                    let v = self.lower_expr(value.as_ref())?;
                    self.write_place(place, v);
                }
                Ok(())
            }
            // `self.x: T = v` (and any `obj.attr: T = v` / `d[k]: T = v`): the
            // annotation on an attribute/subscript target is decorative at the
            // statement level — only the underlying store is emitted here (the
            // same `SetAttr`/`SetItem` as the unannotated form). A `self.<name>:
            // T` field-type contract is collected separately by
            // `scan_self_field_annotations` (consumed in `lower_class` ahead of
            // method lowering, so the field type is known before `typeck`). A bare
            // declaration with no value (`self.x: T`) is a no-op store, exactly as
            // in CPython.
            target => {
                if let Some(value) = &a.value {
                    let v = self.lower_expr(value.as_ref())?;
                    self.assign_to_target(target, v)?;
                }
                Ok(())
            }
        }
    }

    /// Look up or allocate a named binding. A new (non-celled) name takes a
    /// direct local of type `ty`; an existing one keeps its slot (flat
    /// per-function scope). Celled names are pre-created by [`Self::init_cells`]
    /// in the entry block — they are always already in scope here.
    pub(super) fn ensure_binding(&mut self, name: InternedString, ty: SemTy) -> Binding {
        if let Some(b) = self.scope.get(&name).copied() {
            return b;
        }
        debug_assert!(
            !self.celled.contains(&name),
            "celled name must be pre-created by init_cells"
        );
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        self.scope.insert(name, Binding::Direct(id));
        Binding::Direct(id)
    }

    /// Read a bound name: a direct local read, or a `CellGet` through its cell.
    pub(super) fn read_binding(&mut self, b: Binding, span: Span) -> Idx<HirExpr> {
        match b {
            Binding::Direct(lid) => self.local_ref(lid, span),
            Binding::Cell(lid) => self.alloc(HirExprKind::CellGet { cell: lid }, SemTy::Dyn, span),
        }
    }

    /// Write `value` to a bound name: a direct assignment, or a `CellSet`.
    pub(super) fn write_binding(&mut self, b: Binding, value: Idx<HirExpr>) {
        match b {
            Binding::Direct(lid) => self.push_stmt(HirStmt::Assign { target: lid, value }),
            Binding::Cell(lid) => self.push_stmt(HirStmt::CellSet { cell: lid, value }),
        }
    }

    /// Declare-and-write in one step (the common assignment path), routing a
    /// promoted module-global write to its slot (Phase 6B).
    pub(super) fn write_named(&mut self, name: InternedString, ty: SemTy, value: Idx<HirExpr>) {
        match self.resolve_write_place(name, ty) {
            Place::Bind(b) => self.write_binding(b, value),
            Place::Global(var_id) => self.push_stmt(HirStmt::GlobalSet { var_id, value }),
        }
    }

    /// Where a WRITE to `name` lands (Phase 6B): an existing binding; the global
    /// slot (in `__main__` for any promoted name, in a function only under a
    /// `global` declaration — an undeclared assignment binds locally, as in
    /// CPython); else a fresh local.
    pub(super) fn resolve_write_place(&mut self, name: InternedString, ty: SemTy) -> Place {
        if let Some(b) = self.scope.get(&name).copied() {
            return Place::Bind(b);
        }
        if self.is_main || self.global_decls.contains(&name) {
            if let Some(vid) = self.promoted_id(name) {
                return Place::Global(vid);
            }
        }
        Place::Bind(self.ensure_binding(name, ty))
    }

    /// The global slot a READ of `name` (not in scope) resolves to, if any:
    /// any promoted name in `__main__`; in a function a `global`-declared name,
    /// or a promoted name the function never binds locally.
    pub(super) fn global_read_slot(&self, name: InternedString) -> Option<u32> {
        let vid = self.promoted_id(name)?;
        if self.is_main || self.global_decls.contains(&name) || !self.bound_names.contains(&name) {
            Some(vid)
        } else {
            None
        }
    }

    /// The promoted-global `var_id` of `name`, if it has one.
    pub(super) fn promoted_id(&self, name: InternedString) -> Option<u32> {
        self.ctx.promoted.get(self.interner.resolve(name)).copied()
    }

    /// Read through a [`Place`].
    pub(super) fn read_place(&mut self, p: Place, span: Span) -> Idx<HirExpr> {
        match p {
            Place::Bind(b) => self.read_binding(b, span),
            Place::Global(var_id) => {
                self.alloc(HirExprKind::GlobalGet { var_id }, SemTy::Dyn, span)
            }
        }
    }

    /// Write through a [`Place`].
    pub(super) fn write_place(&mut self, p: Place, value: Idx<HirExpr>) {
        match p {
            Place::Bind(b) => self.write_binding(b, value),
            Place::Global(var_id) => self.push_stmt(HirStmt::GlobalSet { var_id, value }),
        }
    }

    pub(super) fn lower_if(&mut self, s: &rustpython_parser::ast::StmtIf) -> Result<()> {
        let cond = self.lower_expr(s.test.as_ref())?;
        let then_b = self.new_block();
        let join = self.new_block();
        let else_b = if s.orelse.is_empty() {
            join
        } else {
            self.new_block()
        };
        self.seal(HirTerminator::Branch {
            cond,
            then: then_b,
            else_: else_b,
        });

        self.switch(then_b);
        self.lower_body(&s.body)?;
        self.seal(HirTerminator::Jump(join));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(join));
        }

        self.switch(join);
        Ok(())
    }

    pub(super) fn lower_while(&mut self, s: &rustpython_parser::ast::StmtWhile) -> Result<bool> {
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let cond = self.lower_expr(s.test.as_ref())?;
        let body_b = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() {
            exit
        } else {
            self.new_block()
        };
        self.seal(HirTerminator::Branch {
            cond,
            then: body_b,
            else_: else_b,
        });

        self.switch(body_b);
        self.scope_stack.push(ScopeCtx::Loop {
            continue_to: header,
            break_to: exit,
        });
        self.lower_body(&s.body)?;
        self.scope_stack.pop();
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    pub(super) fn lower_for(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        // The Phase-3c `range(...)` fast path (raw-i64 cursors) bakes in two
        // assumptions: a compile-time-literal step (so the loop direction is
        // fixed statically) and a simple-`Name` target. Use it ONLY when both
        // hold; everything else — a non-literal/computed step, or an attribute/
        // subscript/tuple target — takes the general iterator path, which drives
        // the runtime `RangeIter` (correct direction + step=0 `ValueError`) and
        // binds an arbitrary target via `bind_for_target`. This is a strict
        // superset of the old behavior: the only loops newly diverted are exactly
        // the ones `lower_for_range` rejected, so gated raw-int loops keep the
        // fast path with no perf regression.
        if is_range_call(s.iter.as_ref())
            && matches!(s.target.as_ref(), Expr::Name(_))
            && range_step_is_literal(s.iter.as_ref())
        {
            self.lower_for_range(s)
        } else {
            self.lower_for_iter(s)
        }
    }

    /// General `for target in <iterable>`: drive the runtime iterator protocol
    /// (`iter` → `next` → `is_exhausted`), binding the target (a name or a tuple
    /// pattern) each iteration. `for`-else / `break` / `continue` reuse the loop
    /// stack exactly as the `while`/range paths do.
    /// Lower a for-loop / comprehension iterable. A File iterable (syntactic
    /// `open(...)` and File variables alike) is handled at lowering: the frozen
    /// runtime cannot iterate a File object (PITFALLS), so `lowering` expands
    /// `Iter(file)` to `rt_file_readlines` + list iteration (Phase 8H) —
    /// line-for-line identical to CPython's lazy file iteration on the small
    /// corpus inputs.
    pub(super) fn lower_iterable_expr(&mut self, e: &Expr, _span: Span) -> Result<Idx<HirExpr>> {
        self.lower_expr(e)
    }

    pub(super) fn lower_for_iter(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        let span = to_span(s.range());

        // it = iter(iterable)  — a Heap(Iterator) local, live across the loop.
        let iterable = self.lower_iterable_expr(s.iter.as_ref(), span)?;
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![iterable],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: it,
            value: iter_expr,
        });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);

        // elem = next(it)   then   done = is_exhausted(it)  (this call order is the
        // runtime contract: `next` advances and sets the exhausted flag).
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next_expr = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref1],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next_expr,
        });
        let it_ref2 = self.local_ref(it, span);
        let done = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterExhausted,
                args: vec![it_ref2],
            },
            SemTy::Bool,
            span,
        );

        let body_b = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() {
            exit
        } else {
            self.new_block()
        };
        // done == true → exit (or the for-else); else run the body.
        self.seal(HirTerminator::Branch {
            cond: done,
            then: else_b,
            else_: body_b,
        });

        self.switch(body_b);
        let elem_ref = self.local_ref(elem, span);
        self.bind_for_target(s.target.as_ref(), elem_ref, span)?;
        self.scope_stack.push(ScopeCtx::Loop {
            continue_to: header,
            break_to: exit,
        });
        self.lower_body(&s.body)?;
        self.scope_stack.pop();
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    /// Bind a `for`-loop target. Delegates the supported assignment shapes to
    /// [`Self::assign_to_target`] — byte-identical on `Name`/`Tuple`/`List`
    /// (same `write_named` / `lower_unpack_subscript`), and additionally lowers
    /// an attribute (`for obj.attr in …` → `SetAttr`) or subscript
    /// (`for lst[i] in …` → `SetItem`) leaf each iteration (backlog §4). Keeps a
    /// precise for-loop diagnostic for everything else.
    pub(super) fn bind_for_target(&mut self, target: &Expr, value: Idx<HirExpr>, span: Span) -> Result<()> {
        match target {
            Expr::Name(_)
            | Expr::Tuple(_)
            | Expr::List(_)
            | Expr::Attribute(_)
            | Expr::Subscript(_) => self.assign_to_target(target, value),
            _ => Err(parse_error("unsupported for-loop target", span)),
        }
    }

    /// The preserved Phase-3 `range(...)` loop with proof-gated raw-i64 cursors.
    pub(super) fn lower_for_range(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        let span = to_span(s.range());
        let (start, stop, step) = parse_range(s.iter.as_ref(), span)?;
        if step == 0 {
            return Err(parse_error("range() step argument must not be zero", span));
        }
        let Expr::Name(n) = s.target.as_ref() else {
            return Err(parse_error("for-loop target must be a simple name", span));
        };
        let i_name = self.intern(n.id.as_str());
        let i_b = self.resolve_write_place(i_name, SemTy::Dyn);
        let cursor = self.fresh_local(SemTy::Dyn);
        let stop_l = self.fresh_local(SemTy::Dyn);

        // Phase 3c: the cursor / stop slot / induction variable `i` / derived
        // body expressions are all left as plain tagged locals here. typeck's
        // interval pass (`narrow_raw_ints`) runs a sound forward range analysis
        // over the materialized CFG and flags every `int` slot — and every
        // derived `int` BinOp — that provably stays within `±RAW_I64_NARROW_BOUND`
        // with no i64 overflow, subsuming the old literal-`range()` heuristic and
        // additionally narrowing `i` itself and body expressions like `i * 3 % k`.

        // cursor = start; stop_l = stop  (range args evaluated once).
        let s_idx = self.lower_range_arg(&start, span)?;
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: s_idx,
        });
        let stop_idx = self.lower_range_arg(&stop, span)?;
        self.push_stmt(HirStmt::Assign {
            target: stop_l,
            value: stop_idx,
        });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let cursor_ref = self.local_ref(cursor, span);
        let stop_ref = self.local_ref(stop_l, span);
        let cmp_op = if step > 0 { CmpOp::Lt } else { CmpOp::Gt };
        let cond = self.alloc(
            HirExprKind::Compare {
                op: cmp_op,
                l: cursor_ref,
                r: stop_ref,
            },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let incr = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() {
            exit
        } else {
            self.new_block()
        };
        self.seal(HirTerminator::Branch {
            cond,
            then: body_b,
            else_: else_b,
        });

        self.switch(body_b);
        // i = cursor
        let cref = self.local_ref(cursor, span);
        self.write_place(i_b, cref);
        self.scope_stack.push(ScopeCtx::Loop {
            continue_to: incr,
            break_to: exit,
        });
        self.lower_body(&s.body)?;
        self.scope_stack.pop();
        self.seal(HirTerminator::Jump(incr));

        // incr: cursor = cursor + step
        self.switch(incr);
        let cref2 = self.local_ref(cursor, span);
        let step_kind = self.int_literal_const(step);
        let step_lit = self.alloc(step_kind, SemTy::Int, span);
        let inc = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Add,
                l: cref2,
                r: step_lit,
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: inc,
        });
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    /// Lower a range() bound argument (start/stop) — an arbitrary expression.
    pub(super) fn lower_range_arg(&mut self, arg: &RangeArg, span: Span) -> Result<Idx<HirExpr>> {
        match arg {
            RangeArg::Zero => Ok(self.alloc(HirExprKind::IntLit(0), SemTy::Int, span)),
            RangeArg::Expr(e) => self.lower_expr(e),
        }
    }

    /// A fixnum/bignum int-literal expr kind (used for the loop step).
    pub(super) fn int_literal_const(&mut self, v: i64) -> HirExprKind {
        if pyaot_core_defs::int_fits(v) {
            HirExprKind::IntLit(v)
        } else {
            HirExprKind::BigIntLit(self.intern(&v.to_string()))
        }
    }

    /// `print(args, sep=…, end=…)` → [`HirStmt::Print`].
    pub(super) fn lower_print(&mut self, call: &rustpython_parser::ast::ExprCall) -> Result<()> {
        let mut sep: Option<InternedString> = None;
        let mut end: Option<InternedString> = None;
        for kw in &call.keywords {
            let key = kw.arg.as_ref().map(|i| i.as_str());
            match key {
                Some("sep") => sep = Some(self.kw_str_literal(kw, "sep")?),
                Some("end") => end = Some(self.kw_str_literal(kw, "end")?),
                Some(other) => {
                    return Err(parse_error(
                        format!("print() got an unexpected keyword argument '{other}'"),
                        to_span(call.range()),
                    ))
                }
                None => {
                    return Err(parse_error(
                        "print() does not support **kwargs",
                        to_span(call.range()),
                    ))
                }
            }
        }

        let mut args = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            args.push(self.lower_expr(arg)?);
        }
        self.push_stmt(HirStmt::Print { args, sep, end });
        Ok(())
    }

    /// Extract a string-literal keyword value (`sep=`/`end=`).
    pub(super) fn kw_str_literal(&mut self, kw: &Keyword, name: &str) -> Result<InternedString> {
        if let Expr::Constant(c) = &kw.value {
            if let Constant::Str(s) = &c.value {
                return Ok(self.intern(s));
            }
        }
        Err(parse_error(
            format!("print() {name}= must be a string literal"),
            to_span(kw.range()),
        ))
    }

}

/// True iff `iter` is a direct `range(...)` call — selects the Phase-3 fast path.
pub(super) fn is_range_call(iter: &Expr) -> bool {
    matches!(iter, Expr::Call(c)
        if matches!(c.func.as_ref(), Expr::Name(n) if n.id.as_str() == "range"))
}

/// True when a `range(...)` call's step is a compile-time integer literal — the
/// precondition for the Phase-3c raw-i64 fast path (which decides the loop
/// direction statically). `range(stop)` / `range(start, stop)` have an implicit
/// step of `1` (literal); `range(start, stop, step)` qualifies only when
/// `step` is an int literal (incl. unary sign). A non-literal/computed step
/// routes to the general iterator path (runtime `RangeIter`). Callers gate this
/// behind [`is_range_call`], so a non-`range` expr conservatively returns false.
pub(super) fn range_step_is_literal(iter: &Expr) -> bool {
    let Expr::Call(call) = iter else {
        return false;
    };
    match call.args.len() {
        0..=2 => true,
        3 => literal_int(&call.args[2]).is_some(),
        _ => false,
    }
}

/// Flatten an attribute chain `a.b.c` rooted at a `Name` into its leftmost name
/// (`"a"`) and full dotted path (`"a.b.c"`), or `None` if the base is not a bare
/// name. Used to fold stdlib qualified calls of any depth (Phase 8D).
pub(super) fn flatten_attr_chain(e: &Expr) -> Option<(&str, String)> {
    let mut parts: Vec<&str> = Vec::new();
    let mut cur = e;
    loop {
        match cur {
            Expr::Attribute(a) => {
                parts.push(a.attr.as_str());
                cur = a.value.as_ref();
            }
            Expr::Name(n) => {
                parts.push(n.id.as_str());
                break;
            }
            _ => return None,
        }
    }
    parts.reverse();
    let leftmost = parts[0];
    Some((leftmost, parts.join(".")))
}

/// True if `e` is the `None` literal (the only RHS supported for `is`/`is not`,
/// Phase 8D).
pub(super) fn is_none_lit(e: &Expr) -> bool {
    matches!(e, Expr::Constant(c) if matches!(c.value, Constant::None))
}

/// The string value of `e` if it is a plain string-literal constant (the
/// attribute-name argument to `getattr`/`setattr`/`hasattr`, §5).
pub(super) fn string_literal_arg(e: &Expr) -> Option<&str> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// The element expressions of a tuple/list target or literal-sequence value, used
/// for unpacking (`a, b = …`). `None` for any other expression.
pub(super) fn seq_target_elts(e: &Expr) -> Option<&[Expr]> {
    match e {
        Expr::Tuple(t) => Some(&t.elts),
        Expr::List(l) => Some(&l.elts),
        _ => None,
    }
}

/// Reject starred unpacking targets (`a, *rest = …`) — deferred to Phase 6.
pub(super) fn reject_starred(targets: &[Expr], span: Span) -> Result<()> {
    if targets.iter().any(|t| matches!(t, Expr::Starred(_))) {
        return Err(parse_error(
            "starred unpacking targets are out of scope",
            span,
        ));
    }
    Ok(())
}

/// Parse `range(...)` from a `for` iterable into `(start, stop, step)`. `step`
/// must be an integer literal (the loop direction is decided at compile time).
pub(super) fn parse_range(iter: &Expr, span: Span) -> Result<(RangeArg<'_>, RangeArg<'_>, i64)> {
    let Expr::Call(call) = iter else {
        return Err(parse_error("for-loop iterable must be range(...)", span));
    };
    let is_range = matches!(call.func.as_ref(), Expr::Name(n) if n.id.as_str() == "range");
    if !is_range {
        return Err(parse_error("for-loop iterable must be range(...)", span));
    }
    if !call.keywords.is_empty() {
        return Err(parse_error("range() takes no keyword arguments", span));
    }
    match call.args.len() {
        1 => Ok((RangeArg::Zero, RangeArg::Expr(&call.args[0]), 1)),
        2 => Ok((
            RangeArg::Expr(&call.args[0]),
            RangeArg::Expr(&call.args[1]),
            1,
        )),
        3 => {
            let step = literal_int(&call.args[2])
                .ok_or_else(|| parse_error("range() step must be an integer literal", span))?;
            Ok((
                RangeArg::Expr(&call.args[0]),
                RangeArg::Expr(&call.args[1]),
                step,
            ))
        }
        _ => Err(parse_error("range() takes 1 to 3 arguments", span)),
    }
}

/// Extract an `i64` from an integer-literal expression (possibly unary-signed).
pub(super) fn literal_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Constant(c) => match &c.value {
            Constant::Int(b) => b.to_string().parse::<i64>().ok(),
            _ => None,
        },
        Expr::UnaryOp(u) => {
            if let Expr::Constant(c) = u.operand.as_ref() {
                if let Constant::Int(b) = &c.value {
                    let v = b.to_string().parse::<i64>().ok()?;
                    return match u.op {
                        PyUnaryOp::USub => Some(-v),
                        PyUnaryOp::UAdd => Some(v),
                        _ => None,
                    };
                }
            }
            None
        }
        _ => None,
    }
}

