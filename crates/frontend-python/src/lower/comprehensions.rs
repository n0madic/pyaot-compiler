use super::*;

impl<'a> FnLowerer<'a> {
    pub(super) fn lower_listcomp(&mut self, c: &ExprListComp, span: Span) -> Result<Idx<HirExpr>> {
        // `Dyn` (non-authoritative) so typeck infers the ELEMENT type from the
        // desugared pushes (Phase 8H, D1) instead of pinning `list[Dyn]`.
        let result = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let kind = CompKind::List {
            result,
            elt: c.elt.as_ref(),
        };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// `{elt for … if …}` → an empty set filled the same way.
    pub(super) fn lower_setcomp(&mut self, c: &ExprSetComp, span: Span) -> Result<Idx<HirExpr>> {
        // `Dyn` for push-driven element inference (Phase 8H, D1).
        let result = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let kind = CompKind::Set {
            result,
            elt: c.elt.as_ref(),
        };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// `{k: v for … if …}` → an empty dict filled key/value-wise.
    pub(super) fn lower_dictcomp(&mut self, c: &ExprDictComp, span: Span) -> Result<Idx<HirExpr>> {
        // `Dyn` for insert-driven key/value inference (Phase 8H, D1).
        let result = self.fresh_local(SemTy::Dyn);
        let empty = self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let kind = CompKind::Dict {
            result,
            key: c.key.as_ref(),
            val: c.value.as_ref(),
        };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// Nest the comprehension's `for`/`if` clauses (one iterator loop per `for`),
    /// emitting the element action at the innermost point.
    pub(super) fn lower_comp_clauses(
        &mut self,
        generators: &[Comprehension],
        idx: usize,
        kind: &CompKind,
        span: Span,
    ) -> Result<()> {
        if idx == generators.len() {
            return self.emit_comp_elem(kind, span);
        }
        let gen = &generators[idx];
        if gen.is_async {
            return Err(parse_error("async comprehensions are out of scope", span));
        }

        // Comprehension loop variables are scoped to the comprehension (CPython 3
        // runs each comprehension in its own function). List/set/dict comps lower
        // inline here, so shadow every target name with a fresh local for the
        // duration and restore the outer binding afterward — otherwise
        // `[x for x in xs]` would clobber an enclosing `x` (genexprs already get
        // their own nested-function scope, so they need no shadowing). Pre-inserting
        // a fresh `Direct` binding (rather than removing the outer one) keeps writes
        // off a promoted global slot too.
        //
        // CPython evaluates ONLY the outermost (first) clause's iterable in the
        // *enclosing* scope, before the comprehension scope exists. So at idx==0
        // lower that iterable FIRST, then install the shadows; otherwise the outer
        // iterable in `[x for x in x]` would resolve to the not-yet-assigned shadow
        // and read uninitialized memory (a SIGSEGV). Inner clauses (idx>=1) lower
        // their iterable AFTER the shadows so they can see earlier loop variables.
        let (saved_targets, iterable): (Vec<(InternedString, Option<Binding>)>, Idx<HirExpr>) =
            if idx == 0 {
                let iterable = self.lower_iterable_expr(&gen.iter, span)?;
                let mut saved = Vec::new();
                let mut raw_names = Vec::new();
                for g in generators {
                    collect_target_names(&g.target, &mut raw_names);
                }
                for raw in raw_names {
                    let name = self.intern(raw);
                    let prev = self.scope.get(&name).copied();
                    let fresh = self.fresh_local(SemTy::Dyn);
                    self.scope.insert(name, Binding::Direct(fresh));
                    saved.push((name, prev));
                }
                (saved, iterable)
            } else {
                (Vec::new(), self.lower_iterable_expr(&gen.iter, span)?)
            };

        // it = iter(iterable)
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
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref1],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next,
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
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        self.switch(body_b);
        let elem_ref = self.local_ref(elem, span);
        self.bind_for_target(&gen.target, elem_ref, span)?;
        // Filters: a false `if` skips to the next element (jump back to header).
        for cond_expr in &gen.ifs {
            let cond = self.lower_expr(cond_expr)?;
            let cont = self.new_block();
            self.seal(HirTerminator::Branch {
                cond,
                then: cont,
                else_: header,
            });
            self.switch(cont);
        }
        // Recurse into the next clause (or emit the element at the innermost).
        self.lower_comp_clauses(generators, idx + 1, kind, span)?;
        self.seal(HirTerminator::Jump(header));
        self.switch(exit);
        // Restore the outer bindings the comprehension's loop variables shadowed.
        for (name, prev) in saved_targets {
            match prev {
                Some(b) => {
                    self.scope.insert(name, b);
                }
                None => {
                    self.scope.remove(&name);
                }
            }
        }
        Ok(())
    }

    // ── reduce/loop builtins: sum / min / max / set (Phase 4C) ─────────────────

    /// Emit the iterator-protocol prologue for a simple loop over an
    /// already-lowered iterable, switching to the loop body and returning the
    /// per-iteration element local plus the header/exit blocks. Pair with
    /// [`Self::end_iter_loop`]. (Used by `sum`/`min`/`max`/`set` — no target
    /// binding, filters, or `break`/`continue`, unlike the `for`/comprehension
    /// paths.)
    pub(super) fn begin_iter_loop(&mut self, iterable: Idx<HirExpr>, span: Span) -> Result<IterLoop> {
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
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref1],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next,
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
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });
        self.switch(body_b);
        Ok(IterLoop { header, exit, elem })
    }

    /// Close a [`Self::begin_iter_loop`] loop: jump back to the header and switch
    /// to the exit block.
    pub(super) fn end_iter_loop(&mut self, lp: IterLoop) {
        self.seal(HirTerminator::Jump(lp.header));
        self.switch(lp.exit);
    }

}
