use super::*;

impl<'a> FnLowerer<'a> {
    pub(crate) fn new(
        interner: &'a mut StringInterner,
        ctx: &'a AnnCtx<'a>,
        shared: &'a mut Shared,
        name: InternedString,
        base_name: &str,
        ret_ty: SemTy,
        enclosing_class: Option<ClassId>,
    ) -> Self {
        let mut blocks = Arena::new();
        let entry = blocks.alloc(HirBlock {
            stmts: Vec::new(),
            term: HirTerminator::Unreachable,
            handler: None,
        });
        Self {
            interner,
            ctx,
            shared,
            name,
            base_name: base_name.to_string(),
            enclosing_class,
            cls_ref: None,
            params: Vec::new(),
            ret_ty,
            exprs: Arena::new(),
            blocks,
            locals: Vec::new(),
            scope: HashMap::new(),
            celled: HashSet::new(),
            shared_writes: HashSet::new(),
            global_decls: HashSet::new(),
            bound_names: HashSet::new(),
            is_main: false,
            entry,
            cur: entry,
            sealed: HashSet::new(),
            cur_handler: None,
            stamped: HashSet::new(),
            cur_line: None,
            scope_stack: Vec::new(),
            synth_counter: 0,
            self_capture: None,
            gen: None,
        }
    }

    /// Adopt the scope's free-variable facts (interning the name sets). A
    /// promoted module-global is never celled in `__main__` — its single
    /// storage is the global slot, which nested functions read directly.
    pub(super) fn set_scope_facts(&mut self, facts: &ScopeFacts) {
        self.celled = facts
            .celled
            .iter()
            .filter(|n| !(self.is_main && self.ctx.promoted.contains_key(*n)))
            .map(|n| self.interner.intern(n))
            .collect();
        self.shared_writes = facts
            .shared_writes
            .iter()
            .map(|n| self.interner.intern(n))
            .collect();
        self.global_decls = facts
            .globals
            .iter()
            .map(|n| self.interner.intern(n))
            .collect();
        self.bound_names = facts
            .bound
            .iter()
            .map(|n| self.interner.intern(n))
            .collect();
    }

    /// Register a parameter as the next local (params occupy locals `0..nparams`).
    pub(super) fn add_param(&mut self, name: InternedString, ty: SemTy) {
        self.add_param_default(name, ty, None);
    }

    /// Register a parameter carrying a default (Phase 6C; literal `Const` or a
    /// `Slot` for a mutable/computed top-level default).
    pub(super) fn add_param_default(
        &mut self,
        name: InternedString,
        ty: SemTy,
        default: Option<ParamDefault>,
    ) {
        let id = LocalId::new(self.locals.len() as u32);
        self.params.push(HirParam {
            name,
            ty: ty.clone(),
            default,
        });
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        self.scope.insert(name, Binding::Direct(id));
    }

    /// Allocate a named *logical* local (not a MIR parameter) bound `Direct` —
    /// used for a generator resume function's Python params, which live in gen
    /// slots rather than the ABI (Phase 6E).
    pub(super) fn add_logical_local(&mut self, name: InternedString, ty: SemTy) -> LocalId {
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
        id
    }

    /// Install the parsed params in MIR order: fixed positional, keyword-only,
    /// `*args` tuple, `**kwargs` dict (Phase 6C).
    pub(super) fn install_params(&mut self, parsed: &ParsedParams) {
        for p in parsed.fixed.iter().chain(&parsed.kwonly) {
            self.add_param_default(p.name, p.ty.clone(), p.default.clone());
        }
        if let Some(name) = parsed.varargs {
            self.add_param(name, SemTy::tuple_var_of(SemTy::Dyn));
        }
        if let Some(name) = parsed.kwargs {
            self.add_param(name, SemTy::dict_of(SemTy::Str, SemTy::Dyn));
        }
    }

    /// Allocate one cell per celled name in the entry block (P6-2: one cell per
    /// variable per *activation*, so loops over closures get CPython
    /// late-binding and repeated calls get independent cells). A celled
    /// parameter is copied into its fresh cell (its annotation becoming the
    /// cell's content type); capture bindings installed by the prologue are
    /// already cells and are skipped.
    pub(super) fn init_cells(&mut self) {
        let mut names: Vec<InternedString> = self.celled.iter().copied().collect();
        names.sort_by_key(|n| n.index());
        for name in names {
            let (init, content_ty) = match self.scope.get(&name).copied() {
                Some(Binding::Cell(_)) => continue,
                Some(Binding::Direct(param_lid)) => {
                    let ty = self.locals[param_lid.index()].ty.clone();
                    (Some(self.local_ref(param_lid, Span::dummy())), ty)
                }
                None => (None, SemTy::Dyn),
            };
            let cell_lid =
                self.alloc_cell_local(name, content_ty, self.shared_writes.contains(&name));
            let mc = self.alloc(HirExprKind::MakeCell { init }, SemTy::Dyn, Span::dummy());
            self.push_stmt(HirStmt::Assign {
                target: cell_lid,
                value: mc,
            });
            self.scope.insert(name, Binding::Cell(cell_lid));
        }
    }

    /// Allocate the local slot that holds a cell for `name`. The slot gets a
    /// distinct `.cell`-suffixed name so `semantics`' name→local map never
    /// aliases it with the original (celled-parameter) slot.
    ///
    /// `content_ty` is the cell's authoritative CONTENT type (an enclosing
    /// annotation carried across the capture boundary; `Dyn` when unknown) —
    /// `typeck` types `CellGet` from it. The slot itself always holds a tagged
    /// cell pointer, so its representation is pinned `Tagged` regardless.
    pub(super) fn alloc_cell_local(
        &mut self,
        name: InternedString,
        content_ty: SemTy,
        cell_shared: bool,
    ) -> LocalId {
        let cell_name = format!("{}.cell", self.interner.resolve(name));
        let cname = self.interner.intern(&cell_name);
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name: cname,
            ty: content_ty,
            raw_int_ok: false,
            pin_tagged: true,
            cell_shared,
            deletable: false,
        });
        id
    }

    /// Seal the current block with `default_term` if it is still open, then
    /// assemble the [`HirFunction`].
    pub(crate) fn finish(mut self, default_term: HirTerminator) -> HirFunction {
        if !self.sealed.contains(&self.cur) {
            self.blocks[self.cur].term = default_term;
        }
        HirFunction {
            name: self.name,
            file: self
                .shared
                .cur_file
                .expect("cur_file is set before any function is lowered"),
            params: self.params,
            varargs: false,
            kwargs: false,
            ret_ty: self.ret_ty,
            // `ret_raw_int` defaults to the always-correct tagged baseline;
            // typeck's interprocedural interval pass sets it where a range proof
            // holds (mirrors `HirLocal::raw_int_ok`).
            ret_raw_int: false,
            locals: self.locals,
            blocks: self.blocks,
            entry: self.entry,
            exprs: self.exprs,
        }
    }

    // ── block builder ──────────────────────────────────────────────────────

    pub(super) fn new_block(&mut self) -> Idx<HirBlock> {
        self.blocks.alloc(HirBlock {
            stmts: Vec::new(),
            term: HirTerminator::Unreachable,
            handler: None,
        })
    }

    /// Stamp the current block with the active handler context, first fill
    /// wins. A block must only ever be filled under one context — the
    /// structural lowerers split blocks whenever `cur_handler` changes — so a
    /// re-stamp under a different context is a frontend bug (dead statements
    /// pushed into an already-sealed block are exempt: they never run).
    pub(super) fn stamp_handler(&mut self) {
        if !self.cur_open() {
            return;
        }
        if self.stamped.insert(self.cur) {
            self.blocks[self.cur].handler = self.cur_handler;
        } else {
            debug_assert_eq!(
                self.blocks[self.cur].handler, self.cur_handler,
                "block filled under two different handler contexts"
            );
        }
    }

    pub(super) fn push_stmt(&mut self, stmt: HirStmt) {
        self.stamp_handler();
        self.blocks[self.cur].stmts.push(stmt);
    }

    /// Seal the current block with `term` (only if still open) and leave `cur`
    /// pointing at it; the caller must `switch` to a fresh block next.
    /// Open-ness is tracked explicitly (not by inspecting the placeholder
    /// terminator) because an explicit `Unreachable` seal — the Phase-7 `raise`
    /// shape — must not be overwritten by a later structural seal.
    pub(super) fn seal(&mut self, term: HirTerminator) {
        self.stamp_handler();
        if self.sealed.insert(self.cur) {
            self.blocks[self.cur].term = term;
        }
    }

    pub(super) fn switch(&mut self, block: Idx<HirBlock>) {
        self.cur = block;
        self.cur_line = None;
    }

    /// Emit a `HirStmt::Line` marker for `span`'s source line if the current
    /// block has not already established it (real tracebacks).
    pub(super) fn mark_line(&mut self, span: Span) {
        let line = self.shared.line_map.line_number(span.start);
        if self.cur_line != Some(line) {
            self.push_stmt(HirStmt::Line(line));
            self.cur_line = Some(line);
        }
    }

    pub(super) fn alloc(&mut self, kind: HirExprKind, ty: SemTy, span: Span) -> Idx<HirExpr> {
        // `raw_int_ok` defaults to the always-correct tagged baseline; typeck's
        // interval pass proves and sets it where sound (Phase 3c).
        self.exprs.alloc(HirExpr {
            kind,
            ty,
            span,
            raw_int_ok: false,
        })
    }

    /// Synthesize `lit0 + str(e0) + lit1 + str(e1) + ... + tail` — the
    /// left-folded string concatenation used for stdlib-exception messages.
    /// Each expression is wrapped in `str(...)` (resolved by `semantics`),
    /// matching the f-string lowering idiom.
    pub(super) fn synth_concat_str(
        &mut self,
        parts: &[(&str, Idx<HirExpr>)],
        tail: &str,
        span: Span,
    ) -> Idx<HirExpr> {
        let mut acc: Option<Idx<HirExpr>> = None;
        let mut push = |this: &mut Self, e: Idx<HirExpr>| {
            acc = Some(match acc {
                Some(a) => this.alloc(
                    HirExprKind::BinOp {
                        op: BinOp::Add,
                        l: a,
                        r: e,
                    },
                    SemTy::Dyn,
                    span,
                ),
                None => e,
            });
        };
        for (lit, expr) in parts {
            if !lit.is_empty() {
                let id = self.intern(lit);
                let lit_e = self.alloc(HirExprKind::StrLit(id), SemTy::Str, span);
                push(self, lit_e);
            }
            let fn_name = self.intern("str");
            let callee = self.alloc(
                HirExprKind::Name(SymbolRef::Unresolved(fn_name)),
                SemTy::Dyn,
                span,
            );
            let wrapped = self.alloc(
                HirExprKind::Call {
                    callee,
                    args: vec![*expr],
                },
                SemTy::Str,
                span,
            );
            push(self, wrapped);
        }
        if !tail.is_empty() {
            let id = self.intern(tail);
            let tail_e = self.alloc(HirExprKind::StrLit(id), SemTy::Str, span);
            push(self, tail_e);
        }
        acc.unwrap_or_else(|| {
            let id = self.intern("");
            self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
        })
    }

    pub(super) fn intern(&mut self, s: &str) -> InternedString {
        self.interner.intern(s)
    }

    /// True iff `dotted` (`"module.attr"`) names ANY known stdlib surface: a
    /// function, const, module attr, class, or a submodule (a prefix of a
    /// longer registered name, e.g. `os.path` for `os.path.join`).
    pub(super) fn stdlib_module_attr_exists(&self, dotted: &str) -> bool {
        let s = &self.ctx.stdlib;
        if s.funcs.contains_key(dotted)
            || s.consts.contains_key(dotted)
            || s.attrs.contains_key(dotted)
            || s.classes.contains_key(dotted)
        {
            return true;
        }
        let prefix = format!("{dotted}.");
        s.funcs.keys().any(|k| k.starts_with(&prefix))
            || s.consts.keys().any(|k| k.starts_with(&prefix))
            || s.attrs.keys().any(|k| k.starts_with(&prefix))
            || s.classes.keys().any(|k| k.starts_with(&prefix))
    }

    /// True iff the current block is still open (no terminator emitted yet).
    pub(super) fn cur_open(&self) -> bool {
        !self.sealed.contains(&self.cur)
    }

    // ── control scopes / early-exit cleanups (Phase 7) ──────────────────────

    /// Index of the innermost `Loop` scope, if any.
    pub(super) fn innermost_loop(&self) -> Option<usize> {
        self.scope_stack
            .iter()
            .rposition(|s| matches!(s, ScopeCtx::Loop { .. }))
    }

    /// Emit the cleanup sequence for an early exit (`return` / `break` /
    /// `continue`) leaving every scope at index `down_to..`, innermost first.
    /// The stack itself is not popped — control statements elsewhere in the
    /// same scopes still need the entries.
    ///
    /// `cur_handler` is deliberately LEFT at the exit edge's final (outer)
    /// context: the caller must seal the exit terminator in that context,
    /// then restore `cur_handler` itself (lowering continues with dead-or-
    /// live code in the original context). Use [`Self::with_exit_cleanups`].
    pub(super) fn emit_exit_cleanups(&mut self, down_to: usize, span: Span) -> Result<()> {
        for i in (down_to..self.scope_stack.len()).rev() {
            match self.scope_stack[i].clone() {
                ScopeCtx::Loop { .. } => {}
                ScopeCtx::TryFrame { outer } => {
                    self.exit_protected(outer);
                }
                ScopeCtx::Handler => {
                    self.push_stmt(HirStmt::ExcOp(ExcOp::EndHandling));
                }
                ScopeCtx::Finally { outer, stmts } => {
                    self.exit_protected(outer);
                    // Re-lower the finalbody on this exit edge. The scopes above
                    // `i` are already cleaned up, so the finalbody must see only
                    // the scopes BELOW this entry (a nested `return` inside it
                    // must not re-run these cleanups).
                    let saved = self.scope_stack.split_off(i);
                    self.lower_body(&stmts)?;
                    self.scope_stack.extend(saved);
                }
                ScopeCtx::WithCleanup { outer, mgr } => {
                    self.exit_protected(outer);
                    self.emit_exit_none_call(mgr, span);
                }
            }
        }
        Ok(())
    }

    /// Run [`Self::emit_exit_cleanups`] plus the caller's exit-edge seal
    /// under the exit context, then restore `cur_handler`.
    pub(super) fn with_exit_cleanups(
        &mut self,
        down_to: usize,
        span: Span,
        seal_exit: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        let saved = self.cur_handler;
        self.emit_exit_cleanups(down_to, span)?;
        seal_exit(self)?;
        self.cur_handler = saved;
        Ok(())
    }

    /// Leave a protected region on an exit path: the code that follows (the
    /// region's cleanup, the rest of the exit edge) runs under the region's
    /// OUTER handler, in a fresh block — the current block is already stamped
    /// with the inner handler.
    pub(super) fn exit_protected(&mut self, outer: Option<Idx<HirBlock>>) {
        if self.cur_open() && self.cur_handler != outer {
            let b = self.new_block();
            self.seal(HirTerminator::Jump(b));
            self.switch(b);
        }
        self.cur_handler = outer;
    }

    /// Emit `mgr.__exit__(None, None, None)` as a statement (the normal-path
    /// context-manager epilogue; the result is ignored).
    pub(super) fn emit_exit_none_call(&mut self, mgr: LocalId, span: Span) {
        let recv = self.local_ref(mgr, span);
        let method_name = self.intern("__exit__");
        let args: Vec<Idx<HirExpr>> = (0..3)
            .map(|_| self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span))
            .collect();
        let call = self.alloc(
            HirExprKind::MethodCall {
                recv,
                method_name,
                args,
                kwargs: vec![],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Expr(call));
    }

}
