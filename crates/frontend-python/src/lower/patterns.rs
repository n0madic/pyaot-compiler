use super::*;

impl<'a> FnLowerer<'a> {
    // ── match (Phase 7E) ─────────────────────────────────────────────────────

    /// Lower a `match` statement: pure desugar to an if/elif CFG on a subject
    /// temp. Captures are ordinary function-scope locals (CPython leak
    /// semantics); binds happen on the partial-match path before the guard.
    pub(super) fn lower_match(&mut self, m: &rustpython_parser::ast::StmtMatch) -> Result<bool> {
        let span = to_span(m.range());
        let subj_e = self.lower_expr(m.subject.as_ref())?;
        let subj = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: subj,
            value: subj_e,
        });
        let join = self.new_block();

        for case in &m.cases {
            let fail_b = self.new_block();
            self.lower_pattern(&case.pattern, subj, fail_b, span)?;
            if let Some(g) = &case.guard {
                let cond = self.lower_expr(g.as_ref())?;
                let body_b = self.new_block();
                self.seal(HirTerminator::Branch {
                    cond,
                    then: body_b,
                    else_: fail_b,
                });
                self.switch(body_b);
            }
            self.lower_body(&case.body)?;
            self.seal(HirTerminator::Jump(join));
            self.switch(fail_b);
        }
        // No case matched: a match statement just falls through.
        self.seal(HirTerminator::Jump(join));
        self.switch(join);
        Ok(false)
    }

    /// Emit the tests for `pat` against the value in local `scr`. On mismatch
    /// control jumps to `fail`; on fall-through the pattern matched and its
    /// captures are bound.
    pub(super) fn lower_pattern(
        &mut self,
        pat: &rustpython_parser::ast::Pattern,
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        use rustpython_parser::ast::Pattern;
        match pat {
            // Literal: `subject == literal` (the documented `==`-vs-`is`
            // divergence for interned singletons is corpus-clean).
            Pattern::MatchValue(v) => {
                let lit = self.lower_expr(&v.value)?;
                self.emit_pattern_eq(scr, lit, fail, span)
            }
            Pattern::MatchSingleton(s) => {
                let lit = self.lower_constant(&s.value, span)?;
                self.emit_pattern_eq(scr, lit, fail, span)
            }
            Pattern::MatchAs(a) => {
                if let Some(sub) = &a.pattern {
                    self.lower_pattern(sub, scr, fail, span)?;
                }
                if let Some(name) = &a.name {
                    let iname = self.intern(name.as_str());
                    let v = self.local_ref(scr, span);
                    self.write_named(iname, SemTy::Dyn, v);
                }
                Ok(())
            }
            Pattern::MatchOr(o) => {
                // CPython requires every alternative to bind the SAME set of
                // names (`case A(x) | B(x):` is fine, `A(x) | B(y)` is a
                // SyntaxError). Each alternative binds its captures on its own
                // success path before jumping to the shared `ok` block, so the
                // names resolve to the same function-scope locals and the merge
                // at `ok` holds whichever alternative matched.
                if let Some((first, rest)) = o.patterns.split_first() {
                    let names0 = sorted_bound_names(first);
                    for sub in rest {
                        if sorted_bound_names(sub) != names0 {
                            return Err(parse_error(
                                "alternative patterns bind different names",
                                span,
                            ));
                        }
                    }
                }
                let ok = self.new_block();
                let n = o.patterns.len();
                for (i, sub) in o.patterns.iter().enumerate() {
                    let alt_fail = if i + 1 == n { fail } else { self.new_block() };
                    self.lower_pattern(sub, scr, alt_fail, span)?;
                    self.seal(HirTerminator::Jump(ok));
                    if i + 1 != n {
                        self.switch(alt_fail);
                    }
                }
                self.switch(ok);
                Ok(())
            }
            Pattern::MatchSequence(s) => self.lower_seq_pattern(&s.patterns, scr, fail, span),
            Pattern::MatchMapping(mp) => self.lower_mapping_pattern(
                &mp.keys,
                &mp.patterns,
                mp.rest.as_ref(),
                scr,
                fail,
                span,
            ),
            Pattern::MatchClass(c) => self.lower_class_pattern(c, scr, fail, span),
            Pattern::MatchStar(_) => Err(parse_error(
                "a star pattern is only valid inside a sequence pattern",
                span,
            )),
        }
    }

    /// `subject == lit` → continue, else jump to `fail`.
    pub(super) fn emit_pattern_eq(
        &mut self,
        scr: LocalId,
        lit: Idx<HirExpr>,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        let s = self.local_ref(scr, span);
        let cmp = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Eq,
                l: s,
                r: lit,
            },
            SemTy::Bool,
            span,
        );
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: cmp,
            then: cont,
            else_: fail,
        });
        self.switch(cont);
        Ok(())
    }

    /// A sequence pattern `[p0, …, *star, …, pn]`: length test (`== n`, star:
    /// `>= n-1`), positional subscripts for prefix/suffix, star capture as a
    /// fresh list.
    pub(super) fn lower_seq_pattern(
        &mut self,
        pats: &[rustpython_parser::ast::Pattern],
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        use rustpython_parser::ast::Pattern;
        let star_pos = pats.iter().position(|p| matches!(p, Pattern::MatchStar(_)));
        if pats
            .iter()
            .enumerate()
            .any(|(i, p)| matches!(p, Pattern::MatchStar(_)) && Some(i) != star_pos)
        {
            return Err(parse_error("multiple star patterns in a sequence", span));
        }

        // n = len(subject), staged once.
        let s = self.local_ref(scr, span);
        let len_e = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Len,
                args: vec![s],
            },
            SemTy::Int,
            span,
        );
        let len_l = self.fresh_local(SemTy::Int);
        self.push_stmt(HirStmt::Assign {
            target: len_l,
            value: len_e,
        });

        let (prefix, suffix): (&[Pattern], &[Pattern]) = match star_pos {
            Some(p) => (&pats[..p], &pats[p + 1..]),
            None => (pats, &[]),
        };
        let need = (prefix.len() + suffix.len()) as i64;
        let len_ref = self.local_ref(len_l, span);
        let need_lit = self.alloc(HirExprKind::IntLit(need), SemTy::Int, span);
        let cmp_op = if star_pos.is_some() {
            CmpOp::GtE
        } else {
            CmpOp::Eq
        };
        let cmp = self.alloc(
            HirExprKind::Compare {
                op: cmp_op,
                l: len_ref,
                r: need_lit,
            },
            SemTy::Bool,
            span,
        );
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: cmp,
            then: cont,
            else_: fail,
        });
        self.switch(cont);

        // Prefix elements: subject[i].
        for (i, sub) in prefix.iter().enumerate() {
            let base = self.local_ref(scr, span);
            let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
            let elem = self.alloc(
                HirExprKind::Subscript { base, index: idx },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: elem,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        // Star capture: subject[p .. n-m] as a fresh list.
        if let Some(p) = star_pos {
            let Pattern::MatchStar(st) = &pats[p] else {
                unreachable!()
            };
            if let Some(name) = &st.name {
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
                let rest = self.build_sublist(scr, lo, hi, span)?;
                let iname = self.intern(name.as_str());
                let v = self.local_ref(rest, span);
                self.write_named(iname, SemTy::Dyn, v);
            }
        }
        // Suffix elements: subject[n - (m - j)].
        for (j, sub) in suffix.iter().enumerate() {
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
            let base = self.local_ref(scr, span);
            let elem = self.alloc(
                HirExprKind::Subscript { base, index: idx },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: elem,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        Ok(())
    }

    /// Build a fresh list of `src[lo..hi]` (both bounds already lowered,
    /// evaluated exactly once).
    pub(super) fn build_sublist(
        &mut self,
        src: LocalId,
        lo: Idx<HirExpr>,
        hi: Idx<HirExpr>,
        span: Span,
    ) -> Result<LocalId> {
        let result = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let cursor = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: lo,
        });
        let hi_l = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: hi_l,
            value: hi,
        });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let c1 = self.local_ref(cursor, span);
        let h1 = self.local_ref(hi_l, span);
        let cond = self.alloc(
            HirExprKind::Compare {
                op: CmpOp::Lt,
                l: c1,
                r: h1,
            },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: body_b,
            else_: exit,
        });

        self.switch(body_b);
        let base = self.local_ref(src, span);
        let c2 = self.local_ref(cursor, span);
        let elem = self.alloc(HirExprKind::Subscript { base, index: c2 }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: elem,
        });
        let c3 = self.local_ref(cursor, span);
        let one = self.alloc(HirExprKind::IntLit(1), SemTy::Int, span);
        let inc = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Add,
                l: c3,
                r: one,
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: cursor,
            value: inc,
        });
        self.seal(HirTerminator::Jump(header));

        self.switch(exit);
        Ok(result)
    }

    /// A mapping pattern `{k: p, …, **rest}`: per-key `Contains` → branch,
    /// bind via `DictGet`; `**rest` is a copy with the matched keys popped
    /// (the original is untouched).
    pub(super) fn lower_mapping_pattern(
        &mut self,
        keys: &[Expr],
        pats: &[rustpython_parser::ast::Pattern],
        rest: Option<&rustpython_parser::ast::Identifier>,
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        // Stage the keys once (used by Contains, DictGet, and DictPopM).
        let mut key_locals = Vec::with_capacity(keys.len());
        for k in keys {
            let ke = self.lower_expr(k)?;
            let kl = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: kl,
                value: ke,
            });
            key_locals.push(kl);
        }
        // Membership tests, then sub-pattern binds.
        for (kl, sub) in key_locals.iter().zip(pats) {
            let c = self.local_ref(scr, span);
            let k = self.local_ref(*kl, span);
            let has = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::Contains,
                    args: vec![c, k],
                },
                SemTy::Bool,
                span,
            );
            let cont = self.new_block();
            self.seal(HirTerminator::Branch {
                cond: has,
                then: cont,
                else_: fail,
            });
            self.switch(cont);

            let c2 = self.local_ref(scr, span);
            let k2 = self.local_ref(*kl, span);
            let got = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::DictGet,
                    args: vec![c2, k2],
                },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: got,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        // `**rest` = copy minus the matched keys (copy semantics).
        if let Some(rest_name) = rest {
            let c = self.local_ref(scr, span);
            let copy = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::DictCopy,
                    args: vec![c],
                },
                SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
                span,
            );
            let copy_l = self.fresh_local(SemTy::dict_of(SemTy::Dyn, SemTy::Dyn));
            self.push_stmt(HirStmt::Assign {
                target: copy_l,
                value: copy,
            });
            for kl in &key_locals {
                let d = self.local_ref(copy_l, span);
                let k = self.local_ref(*kl, span);
                let popped = self.alloc(
                    HirExprKind::ContainerExpr {
                        op: ContainerOp::DictPopM,
                        args: vec![d, k],
                    },
                    SemTy::Dyn,
                    span,
                );
                self.push_stmt(HirStmt::Expr(popped));
            }
            let iname = self.intern(rest_name.as_str());
            let v = self.local_ref(copy_l, span);
            self.write_named(iname, SemTy::Dyn, v);
        }
        Ok(())
    }

    /// A class pattern `Cls(attr=p, …)` (keyword-only): `IsInstance` → branch,
    /// then per-kwarg attribute reads feeding sub-patterns.
    pub(super) fn lower_class_pattern(
        &mut self,
        c: &rustpython_parser::ast::PatternMatchClass,
        scr: LocalId,
        fail: Idx<HirBlock>,
        span: Span,
    ) -> Result<()> {
        let Expr::Name(n) = c.cls.as_ref() else {
            return Err(parse_error("class pattern must name a user class", span));
        };
        let Some((cid, iname)) = self.ctx.class_map.get(n.id.as_str()).copied() else {
            return Err(parse_error(
                format!("unknown class `{}` in class pattern", n.id.as_str()),
                span,
            ));
        };
        if !c.patterns.is_empty() {
            return Err(parse_error(
                "positional class patterns (`__match_args__`) are out of scope; \
                 use keyword patterns (`Cls(attr=…)`)",
                span,
            ));
        }
        let v = self.local_ref(scr, span);
        let isinst = self.alloc(
            HirExprKind::IsInstance {
                value: v,
                class_id: cid,
            },
            SemTy::Bool,
            span,
        );
        let cont = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: isinst,
            then: cont,
            else_: fail,
        });
        self.switch(cont);

        // Narrow the subject to the class type so attribute reads resolve. The
        // narrowing is runtime-guarded by the `IsInstance` branch above, but
        // inference would still see `Class{subject} → Class{pattern}` and the
        // annotation-contract check would reject it — so the value is
        // type-erased through a shared cell first (a `cell_shared` `CellGet`
        // is always `Dyn`, the gradual seam the contract check admits).
        let erase_name = self.intern("<match-subject>");
        let cell_lid = self.alloc_cell_local(erase_name, SemTy::Dyn, true);
        let init = self.local_ref(scr, span);
        let mc = self.alloc(HirExprKind::MakeCell { init: Some(init) }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: cell_lid,
            value: mc,
        });
        let erased = self.alloc(HirExprKind::CellGet { cell: cell_lid }, SemTy::Dyn, span);
        let cls_local = self.fresh_local(SemTy::Class {
            class_id: cid,
            name: iname,
        });
        self.push_stmt(HirStmt::Assign {
            target: cls_local,
            value: erased,
        });

        for (attr, sub) in c.kwd_attrs.iter().zip(&c.kwd_patterns) {
            let base = self.local_ref(cls_local, span);
            let aname = self.intern(attr.as_str());
            let read = self.alloc(
                HirExprKind::Attribute {
                    value: base,
                    name: aname,
                },
                SemTy::Dyn,
                span,
            );
            let tmp = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign {
                target: tmp,
                value: read,
            });
            self.lower_pattern(sub, tmp, fail, span)?;
        }
        Ok(())
    }

}

/// Collect every name a `match` pattern binds (capture / `as` / star / `**rest`),
/// recursing into sub-patterns. Mapping keys are expressions, not patterns, so
/// they bind nothing.
fn pattern_bound_names(p: &rustpython_parser::ast::Pattern, out: &mut Vec<String>) {
    use rustpython_parser::ast::Pattern;
    match p {
        Pattern::MatchValue(_) | Pattern::MatchSingleton(_) => {}
        Pattern::MatchAs(a) => {
            if let Some(n) = &a.name {
                out.push(n.to_string());
            }
            if let Some(sub) = &a.pattern {
                pattern_bound_names(sub, out);
            }
        }
        Pattern::MatchStar(s) => {
            if let Some(n) = &s.name {
                out.push(n.to_string());
            }
        }
        Pattern::MatchOr(o) => {
            for sub in &o.patterns {
                pattern_bound_names(sub, out);
            }
        }
        Pattern::MatchSequence(s) => {
            for sub in &s.patterns {
                pattern_bound_names(sub, out);
            }
        }
        Pattern::MatchMapping(m) => {
            for sub in &m.patterns {
                pattern_bound_names(sub, out);
            }
            if let Some(r) = &m.rest {
                out.push(r.to_string());
            }
        }
        Pattern::MatchClass(c) => {
            for sub in &c.patterns {
                pattern_bound_names(sub, out);
            }
            for sub in &c.kwd_patterns {
                pattern_bound_names(sub, out);
            }
        }
    }
}

/// The sorted, de-duplicated set of names a pattern binds — used to enforce
/// CPython's "alternative patterns bind different names" rule on or-patterns.
fn sorted_bound_names(p: &rustpython_parser::ast::Pattern) -> Vec<String> {
    let mut names = Vec::new();
    pattern_bound_names(p, &mut names);
    names.sort();
    names.dedup();
    names
}

