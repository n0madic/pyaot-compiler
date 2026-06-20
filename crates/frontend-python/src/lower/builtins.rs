use super::*;

impl<'a> FnLowerer<'a> {
    /// `sum(iterable[, start])` → [`HirExprKind::Sum`] (Phase 8H, D2): typeck
    /// types the accumulator precisely (numeric promotion / inferred dunder
    /// returns), lowering expands the iterator loop. A generator-expression
    /// argument is MATERIALIZED into a list comprehension here — eager, not
    /// lazy, which is observationally identical for sum (the corpus inputs are
    /// finite and side-effect-free).
    pub(super) fn lower_sum(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.is_empty() || args.len() > 2 {
            return Err(parse_error("sum() takes 1 or 2 arguments", span));
        }
        let iterable = if let Expr::GeneratorExp(g) = &args[0] {
            // Same desugar as a list comprehension, driven by the genexpr's
            // elt/generators.
            let result = self.fresh_local(SemTy::Dyn);
            let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
            self.push_stmt(HirStmt::Assign {
                target: result,
                value: empty,
            });
            let kind = CompKind::List {
                result,
                elt: g.elt.as_ref(),
            };
            self.lower_comp_clauses(&g.generators, 0, &kind, span)?;
            self.local_ref(result, span)
        } else {
            self.lower_expr(&args[0])?
        };
        let start = match args.get(1) {
            Some(s) => Some(self.lower_expr(s)?),
            None => None,
        };
        Ok(self.alloc(HirExprKind::Sum { iterable, start }, SemTy::Dyn, span))
    }

    /// `min`/`max` over a single iterable, or over 2+ positional args (wrapped in a
    /// synthetic list), with optional `key=`. Compares with the tagged baseline
    /// (`rt_obj_cmp`), so heap elements order by value, not pointer (PITFALLS
    /// B13). An empty input raises `ValueError` (Phase 7, CPython semantics);
    /// the accumulator is seeded from the first element, so its inferred type
    /// is the element type — never a spurious `Optional`.
    pub(super) fn lower_minmax(
        &mut self,
        args: &[Expr],
        key: Option<&Expr>,
        span: Span,
        is_min: bool,
    ) -> Result<Idx<HirExpr>> {
        if args.is_empty() {
            return Err(parse_error(
                "min()/max() require at least one argument",
                span,
            ));
        }
        // 1 arg → iterate it; 2+ args → iterate a synthetic list of the args.
        let iterable = if args.len() == 1 {
            self.lower_expr(&args[0])?
        } else {
            let elems = self.lower_expr_list(args)?;
            self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span)
        };
        // The key callable: a bare out-of-scope name (`key=abs`, `key=len`, a
        // top-level def) is called DIRECTLY per element — builtins have no
        // value-position thunk, so staging would reject them. Anything else
        // (lambda, a local) is staged once and called indirectly (CPython
        // evaluates the key expression once; a bare name re-read is pure).
        let key_mode: Option<KeyMode> = match key {
            None => None,
            Some(k @ Expr::Name(n)) => {
                let iname = self.intern(n.id.as_str());
                if self.scope.contains_key(&iname) {
                    let kv = self.lower_expr(k)?;
                    let l = self.fresh_local(SemTy::Dyn);
                    self.push_stmt(HirStmt::Assign {
                        target: l,
                        value: kv,
                    });
                    Some(KeyMode::Staged(l))
                } else {
                    Some(KeyMode::ByName(k))
                }
            }
            Some(k) => {
                let kv = self.lower_expr(k)?;
                let l = self.fresh_local(SemTy::Dyn);
                self.push_stmt(HirStmt::Assign {
                    target: l,
                    value: kv,
                });
                Some(KeyMode::Staged(l))
            }
        };

        // it = iter(iterable); first probe decides empty-vs-seed.
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
        let elem0 = self.emit_iter_next(it, span);
        let done0 = self.emit_iter_exhausted(it, span);
        let empty_b = self.new_block();
        let first_b = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done0,
            then: empty_b,
            else_: first_b,
        });

        // empty: raise ValueError — the live-oracle (CPython ≥3.13) wording.
        self.switch(empty_b);
        let what = if is_min { "min" } else { "max" };
        let msg_id = self.intern(&format!("{what}() iterable argument is empty"));
        let msg = self.alloc(HirExprKind::StrLit(msg_id), SemTy::Str, span);
        self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
            tag: pyaot_core_defs::BuiltinExceptionKind::ValueError.tag(),
            msg: Some(msg),
        }));
        self.seal(HirTerminator::Unreachable);

        // seed: acc = elem0; acc_key = key(elem0) when keyed.
        self.switch(first_b);
        let acc = self.fresh_local(SemTy::Dyn);
        let e0 = self.local_ref(elem0, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: e0,
        });
        let acc_key = match &key_mode {
            Some(km) => {
                let l = self.fresh_local(SemTy::Dyn);
                let v = self.emit_key_call(km, elem0, span)?;
                self.push_stmt(HirStmt::Assign {
                    target: l,
                    value: v,
                });
                Some(l)
            }
            None => None,
        };

        // loop: elem = next(it); done → exit; cand </> best → replace.
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.emit_iter_next(it, span);
        let done = self.emit_iter_exhausted(it, span);
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        self.switch(body_b);
        let cand_key = match &key_mode {
            Some(km) => {
                let l = self.fresh_local(SemTy::Dyn);
                let v = self.emit_key_call(km, elem, span)?;
                self.push_stmt(HirStmt::Assign {
                    target: l,
                    value: v,
                });
                Some(l)
            }
            None => None,
        };
        let (cl, bl) = match (cand_key, acc_key) {
            (Some(c), Some(b)) => (c, b),
            _ => (elem, acc),
        };
        let cref = self.local_ref(cl, span);
        let bref = self.local_ref(bl, span);
        let op = if is_min { CmpOp::Lt } else { CmpOp::Gt };
        let cmp = self.alloc(
            HirExprKind::Compare {
                op,
                l: cref,
                r: bref,
            },
            SemTy::Bool,
            span,
        );
        let upd = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: cmp,
            then: upd,
            else_: header,
        });
        self.switch(upd);
        let e_ref = self.local_ref(elem, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: e_ref,
        });
        if let (Some(ck), Some(ak)) = (cand_key, acc_key) {
            let ck_ref = self.local_ref(ck, span);
            self.push_stmt(HirStmt::Assign {
                target: ak,
                value: ck_ref,
            });
        }
        self.seal(HirTerminator::Jump(header));

        self.switch(exit);
        Ok(self.local_ref(acc, span))
    }

    /// `pow(a, b)` → the `**` operator (`BinOp::Pow`), which is
    /// already end-to-end and bignum- / numeric-tower-correct via `rt_obj_pow`
    /// (a negative exponent yields a float, exactly like `a ** b`). 2-arg only:
    /// the 1-arg form and the 3-arg modular form `pow(a, b, m)` are out of scope.
    pub(super) fn lower_pow(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error(
                "pow() takes exactly two arguments (1-arg and 3-arg modular pow \
                 are out of scope)",
                span,
            ));
        }
        let l = self.lower_expr(&args[0])?;
        let r = self.lower_expr(&args[1])?;
        Ok(self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Pow,
                l,
                r,
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `divmod(a, b)` → the 2-tuple `(a // b, a % b)`. `a` and `b` are
    /// each staged into a fresh local ONCE, left-to-right (CPython
    /// evaluate-once / eval-order, §1); both binops apply CPython floor/sign
    /// semantics via `rt_obj_floordiv`/`rt_obj_mod` (PITFALLS B1), so the tuple
    /// is exact for negative operands too.
    pub(super) fn lower_divmod(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error("divmod() takes exactly two arguments", span));
        }
        let a_val = self.lower_expr(&args[0])?;
        let a = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: a,
            value: a_val,
        });
        let b_val = self.lower_expr(&args[1])?;
        let b = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign {
            target: b,
            value: b_val,
        });

        let aq = self.local_ref(a, span);
        let bq = self.local_ref(b, span);
        let q = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::FloorDiv,
                l: aq,
                r: bq,
            },
            SemTy::Dyn,
            span,
        );
        let ar = self.local_ref(a, span);
        let br = self.local_ref(b, span);
        let rem = self.alloc(
            HirExprKind::BinOp {
                op: BinOp::Mod,
                l: ar,
                r: br,
            },
            SemTy::Dyn,
            span,
        );
        Ok(self.alloc(
            HirExprKind::TupleLit {
                elems: vec![q, rem],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `all(iterable)` / `any(iterable)` — an iterator loop mirroring
    /// [`Self::lower_minmax`]. The accumulator seeds to the empty-input answer
    /// (`all([]) == True`, `any([]) == False`); each element is tested for
    /// truthiness (the same `Branch`-cond mechanism `if elem:` uses) and the
    /// loop short-circuits on the first falsy (`all`) / truthy (`any`) element,
    /// flipping the accumulator. The result is the `Bool` accumulator — zero new
    /// runtime (reuses `Iter`/`IterNext`/`IterExhausted` + existing truthiness).
    pub(super) fn lower_all_any(&mut self, args: &[Expr], span: Span, is_all: bool) -> Result<Idx<HirExpr>> {
        if args.len() != 1 {
            return Err(parse_error("all()/any() take exactly one argument", span));
        }
        let iterable = self.lower_expr(&args[0])?;

        // acc = empty-input answer (True for all, False for any).
        let acc = self.fresh_local(SemTy::Bool);
        let seed = self.alloc(HirExprKind::BoolLit(is_all), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: seed,
        });

        // it = iter(iterable).
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

        // loop: elem = next(it); done → exit; else test truthiness.
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.emit_iter_next(it, span);
        let done = self.emit_iter_exhausted(it, span);
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        // body: branch on element truthiness. `all`: falsy short-circuits;
        // `any`: truthy short-circuits. The short-circuit edge flips the acc.
        self.switch(body_b);
        let hit = self.new_block();
        let elem_ref = self.local_ref(elem, span);
        let (then_b, else_b) = if is_all { (header, hit) } else { (hit, header) };
        self.seal(HirTerminator::Branch {
            cond: elem_ref,
            then: then_b,
            else_: else_b,
        });

        self.switch(hit);
        let flipped = self.alloc(HirExprKind::BoolLit(!is_all), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: flipped,
        });
        self.seal(HirTerminator::Jump(exit));

        self.switch(exit);
        Ok(self.local_ref(acc, span))
    }

    /// `functools.reduce(function, iterable[, initial])` — a higher-order
    /// builtin (like `map`/`filter`) desugared to a compiled accumulator loop
    /// calling `function(acc, elem)` each iteration, mirroring
    /// [`Self::lower_minmax`]'s seed-from-first-element shape. This deliberately
    /// AVOIDS the raw-ABI `rt_reduce` callback path (the PITFALLS A4
    /// anti-pattern — a parallel HOF calling convention with hand-encoded
    /// captures): the reduction callable rides the ordinary indirect-call
    /// machinery (lambda / closure / named def alike), so its arguments and
    /// result stay on the uniform tagged ABI. Without an `initial` the
    /// accumulator seeds from the first element and an empty iterable raises
    /// `TypeError` (CPython); with one, the accumulator seeds from `initial` and
    /// an empty iterable returns it unchanged.
    pub(super) fn lower_reduce(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() < 2 || args.len() > 3 {
            return Err(parse_error("reduce() takes 2 or 3 arguments", span));
        }
        // The reduction callable, staged with the same discipline as the
        // `min`/`max` `key=` and evaluated FIRST (CPython left-to-right order).
        let func_mode = self.stage_callable(&args[0])?;

        let iterable = self.lower_expr(&args[1])?;
        let initial = match args.get(2) {
            Some(e) => Some(self.lower_expr(e)?),
            None => None,
        };

        // it = iter(iterable).
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

        // Seed the accumulator: from `initial` if given, else from the first
        // element (empty-without-initial raises TypeError, like CPython).
        let acc = self.fresh_local(SemTy::Dyn);
        match initial {
            Some(init) => {
                self.push_stmt(HirStmt::Assign {
                    target: acc,
                    value: init,
                });
            }
            None => {
                let elem0 = self.emit_iter_next(it, span);
                let done0 = self.emit_iter_exhausted(it, span);
                let empty_b = self.new_block();
                let seed_b = self.new_block();
                self.seal(HirTerminator::Branch {
                    cond: done0,
                    then: empty_b,
                    else_: seed_b,
                });

                self.switch(empty_b);
                let msg_id = self.intern("reduce() of empty iterable with no initial value");
                let msg = self.alloc(HirExprKind::StrLit(msg_id), SemTy::Str, span);
                self.push_stmt(HirStmt::Raise(HirRaise::Builtin {
                    tag: pyaot_core_defs::BuiltinExceptionKind::TypeError.tag(),
                    msg: Some(msg),
                }));
                self.seal(HirTerminator::Unreachable);

                self.switch(seed_b);
                let e0 = self.local_ref(elem0, span);
                self.push_stmt(HirStmt::Assign {
                    target: acc,
                    value: e0,
                });
            }
        }

        // loop: elem = next(it); done → exit; else acc = func(acc, elem).
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.emit_iter_next(it, span);
        let done = self.emit_iter_exhausted(it, span);
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch {
            cond: done,
            then: exit,
            else_: body_b,
        });

        self.switch(body_b);
        let call = self.emit_reduce_call(&func_mode, acc, elem, span)?;
        self.push_stmt(HirStmt::Assign {
            target: acc,
            value: call,
        });
        self.seal(HirTerminator::Jump(header));

        self.switch(exit);
        Ok(self.local_ref(acc, span))
    }

    /// `map(func, iterable)` — the next higher-order builtin after `reduce`,
    /// desugared to an EAGER compiled loop that calls `func(elem)` per element
    /// through the ordinary uniform-tagged indirect-call machinery, materializes
    /// the results into a `list`, and wraps it in an iterator (`ContainerOp::Iter`)
    /// so `for`/`list`/`next`/`sum` consume it like any other iterable.
    ///
    /// This deliberately AVOIDS the runtime `rt_map_new` / `IteratorKind::Map`
    /// lazy-iterator HOF machinery (the PITFALLS A4 anti-pattern — a parallel
    /// calling convention with hand-encoded captures, marker bits, and an `i8`
    /// predicate ABI). `func` is staged ONCE (CPython evaluates the callable a
    /// single time), and builtin callbacks (`map(str, …)` / `map(len, …)`) resolve
    /// through the normal `Symbol`-dispatch in `lowering::lower_call` with no extra
    /// code — they ride the same tagged `Call` a compiled lambda/closure does. The
    /// eager-vs-lazy side-effect timing is observationally identical on the finite,
    /// pure corpus (the `lower_sum`/`reduce` materialization precedent). Only the
    /// single-iterable form is supported; multi-iterable `map` needs `zip`
    /// (§12, out of scope).
    pub(super) fn lower_map(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        match args.len() {
            2 => {}
            n if n > 2 => {
                return Err(parse_error(
                    "single iterable only — multi-iterable map() needs zip (§12), out of scope",
                    span,
                ))
            }
            _ => return Err(parse_error("map() takes a function and one iterable", span)),
        }
        // Stage the callable FIRST (CPython evaluates `func` once, before the
        // iterable), with the `min`/`max` `key=` discipline.
        let func = self.stage_callable(&args[0])?;
        let iterable = self.lower_expr(&args[1])?;

        // result = [] — a heap ListObj of uniform-Tagged elements, GC-rooted as a
        // stack local (the same B5-safe shape as `set()`/`sum(genexpr)`).
        let result = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });

        // for elem in iterable: result.append(func(elem)).
        let lp = self.begin_iter_loop(iterable, span)?;
        let mapped = self.emit_key_call(&func, lp.elem, span)?;
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: mapped,
        });
        self.end_iter_loop(lp);

        let list_ref = self.local_ref(result, span);
        Ok(self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![list_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `filter(func, iterable)` — the conditional sibling of [`Self::lower_map`]:
    /// an EAGER loop that pushes `elem` only when `func(elem)` is truthy. The
    /// special `filter(None, xs)` form (the predicate is the `None` literal)
    /// filters on the element's own truthiness instead. The survivors are
    /// materialized into a `list` wrapped in an iterator. Same A4 avoidance as
    /// `map` — the predicate rides the ordinary tagged `Call` (lowering
    /// truthiness-tests the result), never the `rt_filter_new` / `i8`-predicate-ABI
    /// HOF path. `func` is staged once (CPython single evaluation).
    pub(super) fn lower_filter(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error(
                "filter() takes a predicate (or None) and one iterable",
                span,
            ));
        }
        // `filter(None, xs)` keeps truthy elements directly; otherwise stage the
        // predicate once.
        let pred_is_none = is_none_lit(&args[0]);
        let func = if pred_is_none {
            None
        } else {
            Some(self.stage_callable(&args[0])?)
        };
        let iterable = self.lower_expr(&args[1])?;

        let result = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });

        // for elem in iterable: if <pred>: result.append(elem). A falsy test
        // branches straight back to the loop header (skip), mirroring the
        // comprehension `if`-filter.
        let lp = self.begin_iter_loop(iterable, span)?;
        let cond = match &func {
            Some(mode) => self.emit_key_call(mode, lp.elem, span)?,
            None => self.local_ref(lp.elem, span),
        };
        let push_b = self.new_block();
        self.seal(HirTerminator::Branch {
            cond,
            then: push_b,
            else_: lp.header,
        });
        self.switch(push_b);
        let elem_ref = self.local_ref(lp.elem, span);
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: elem_ref,
        });
        self.end_iter_loop(lp);

        let list_ref = self.local_ref(result, span);
        Ok(self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![list_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `format(value[, spec])` (§5) — the value/spec sibling of an f-string field
    /// and `str.format`. Desugars to the same `FormatValue { value, spec }`
    /// (`rt_format`) node, with the spec defaulting to the empty string (which
    /// routes a class instance to its `__format__`). Unshadowed-gated by the
    /// caller; no `!` conversion. A dynamic spec (`format(x, var)`) just lowers
    /// `var` as an ordinary string-valued expr.
    pub(super) fn lower_format_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        let (value_expr, spec_expr) = match args {
            [v] => (v, None),
            [v, s] => (v, Some(s)),
            _ => return Err(parse_error("format() takes one or two arguments", span)),
        };
        let value = self.lower_expr(value_expr)?;
        let spec = match spec_expr {
            Some(s) => self.lower_expr(s)?,
            None => {
                let id = self.intern("");
                self.alloc(HirExprKind::StrLit(id), SemTy::Str, span)
            }
        };
        Ok(self.emit_format_field(
            value,
            rustpython_parser::ast::ConversionFlag::None,
            spec,
            span,
        ))
    }

    /// `getattr(obj, "name")` (§5) ≡ `obj.name` — a pure frontend desugar onto the
    /// existing [`HirExprKind::Attribute`] read (static `GetField` for a concrete
    /// receiver, gradual `GetFieldNamed` → `rt_getattr_name` for a `Dyn` one). The
    /// name must be a string literal — dynamic `getattr(o, var)` is the documented
    /// out-of-scope boundary — and the 3-arg `default` form is rejected.
    pub(super) fn lower_getattr_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if !matches!(args.len(), 2 | 3) {
            return Err(parse_error("getattr() takes two or three arguments", span));
        }
        let name = string_literal_arg(&args[1]).ok_or_else(|| {
            parse_error("dynamic getattr (non-literal name) is out of scope", span)
        })?;
        let value = self.lower_expr(&args[0])?;
        let name = self.intern(name);
        // Both forms become a `GetAttrByName` carrying the `getattr` fallback
        // semantics (§5, L1681): lowering keeps the static fast path for a
        // provably-present attr, else routes to the runtime probe (raising for
        // 2-arg, returning `default` for 3-arg).
        let default = match args.get(2) {
            Some(d) => Some(self.lower_expr(d)?),
            None => None,
        };
        Ok(self.alloc(
            HirExprKind::GetAttrByName {
                value,
                name,
                default,
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `setattr(obj, "name", value)` (§5) ≡ `obj.name = value` — a pure frontend
    /// desugar onto the existing [`HirStmt::SetAttr`] write (the `SetFieldNamed`
    /// legalize path for a gradual receiver). The name must be a string literal;
    /// the call evaluates to `None` (CPython's `setattr` return).
    pub(super) fn lower_setattr_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 3 {
            return Err(parse_error("setattr() takes three arguments", span));
        }
        let name = string_literal_arg(&args[1]).ok_or_else(|| {
            parse_error("dynamic setattr (non-literal name) is out of scope", span)
        })?;
        let base = self.lower_expr(&args[0])?;
        let value = self.lower_expr(&args[2])?;
        let name = self.intern(name);
        self.push_stmt(HirStmt::SetAttr { base, name, value });
        Ok(self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span))
    }

    /// `hasattr(obj, "name")` (§5) → `Bool`, folded statically at lowering from
    /// the receiver's `ClassInfo`. The name must be a string literal; a
    /// `Dyn` / non-class receiver is rejected in lowering (a runtime probe is out
    /// of scope), mirroring `isinstance` against a builtin type.
    pub(super) fn lower_hasattr_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error("hasattr() takes two arguments", span));
        }
        let name = string_literal_arg(&args[1]).ok_or_else(|| {
            parse_error("dynamic hasattr (non-literal name) is out of scope", span)
        })?;
        let value = self.lower_expr(&args[0])?;
        let name = self.intern(name);
        Ok(self.alloc(HirExprKind::HasAttr { value, name }, SemTy::Bool, span))
    }

    /// `issubclass(Sub, Sup)` (§5) → `Bool`, folded at lowering via the C3-MRO
    /// check. Both args must be bare names resolving to user classes (mirrors the
    /// `isinstance` builder); the builtin-type (`issubclass(bool, int)`) and tuple
    /// second-arg forms are out of scope (clean error).
    pub(super) fn lower_issubclass_builtin(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.len() != 2 {
            return Err(parse_error("issubclass() takes two arguments", span));
        }
        let resolve = |arg: &Expr| -> Option<ClassId> {
            if let Expr::Name(n) = arg {
                self.ctx.class_map.get(n.id.as_str()).map(|(cid, _)| *cid)
            } else {
                None
            }
        };
        let (Some(sub), Some(sup)) = (resolve(&args[0]), resolve(&args[1])) else {
            return Err(parse_error(
                "issubclass() requires user-class names \
                 (builtin-type / tuple forms out of scope)",
                span,
            ));
        };
        Ok(self.alloc(HirExprKind::IsSubclass { sub, sup }, SemTy::Bool, span))
    }

    /// `"literal".format(args, kwargs)` (§9) — a literal-receiver desugar onto the
    /// f-string field machinery. Each replacement field binds to a positional /
    /// keyword arg AT COMPILE TIME, so the runtime sees the same `FormatValue`
    /// concat an equivalent f-string would produce. All args are staged ONCE in
    /// written order (CPython evaluates every arg before formatting, and a field
    /// may reference the same positional twice). Scope limits (clean errors):
    /// auto↔manual numbering mix, `{0.attr}`/`{0[k]}` access, nested `{}` in a
    /// spec, a missing keyword/index.
    pub(super) fn lower_str_format(
        &mut self,
        template: &str,
        c: &ExprCall,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        // Stage positionals then keyword values, in written order. (`*`/`**`
        // spreads were already rejected by the method-call gate.)
        let mut pos: Vec<LocalId> = Vec::with_capacity(c.args.len());
        for a in &c.args {
            pos.push(self.stage_arg(a)?);
        }
        let mut kw: Vec<(InternedString, LocalId)> = Vec::with_capacity(c.keywords.len());
        for k in &c.keywords {
            let name = k.arg.as_ref().ok_or_else(|| {
                parse_error("`**kwargs` spreading is not supported for .format()", span)
            })?;
            let id = self.intern(name.as_str());
            kw.push((id, self.stage_arg(&k.value)?));
        }

        let segs = parse_format_template(template, span)?;
        let mut auto_idx = 0usize;
        let mut numbering = FmtNumbering::Unset;
        let mut parts: Vec<Idx<HirExpr>> = Vec::with_capacity(segs.len());
        for seg in segs {
            match seg {
                FmtSeg::Lit(text) => {
                    let id = self.intern(&text);
                    parts.push(self.alloc(HirExprKind::StrLit(id), SemTy::Str, span));
                }
                FmtSeg::Field { field, conv, spec } => {
                    let value_local = match field {
                        FmtFieldRef::Auto => {
                            if numbering == FmtNumbering::Manual {
                                return Err(parse_error(
                                    "cannot switch from manual field numbering to automatic field specification",
                                    span,
                                ));
                            }
                            numbering = FmtNumbering::Auto;
                            let i = auto_idx;
                            auto_idx += 1;
                            *pos.get(i).ok_or_else(|| {
                                parse_error(
                                    format!("Replacement index {i} out of range for positional args tuple"),
                                    span,
                                )
                            })?
                        }
                        FmtFieldRef::Index(i) => {
                            if numbering == FmtNumbering::Auto {
                                return Err(parse_error(
                                    "cannot switch from automatic field specification to manual field numbering",
                                    span,
                                ));
                            }
                            numbering = FmtNumbering::Manual;
                            *pos.get(i).ok_or_else(|| {
                                parse_error(
                                    format!("Replacement index {i} out of range for positional args tuple"),
                                    span,
                                )
                            })?
                        }
                        FmtFieldRef::Keyword(name) => {
                            let id = self.intern(&name);
                            kw.iter()
                                .find(|(k, _)| *k == id)
                                .map(|(_, l)| *l)
                                .ok_or_else(|| {
                                    parse_error(
                                        format!("missing keyword argument '{name}' for .format()"),
                                        span,
                                    )
                                })?
                        }
                    };
                    let value = self.local_ref(value_local, span);
                    let spec_id = self.intern(&spec);
                    let spec_expr = self.alloc(HirExprKind::StrLit(spec_id), SemTy::Str, span);
                    parts.push(self.emit_format_field(value, conv, spec_expr, span));
                }
            }
        }
        Ok(self.concat_str_parts(parts, span))
    }

    /// Stage a callable argument (reduce's `function`) with the `min`/`max`
    /// `key=` discipline: a bare unshadowed name is called by name (a builtin
    /// has no value-position thunk, and a bare-name re-read is pure); a local
    /// name / lambda / other expression is staged once and called indirectly.
    pub(super) fn stage_callable<'e>(&mut self, e: &'e Expr) -> Result<KeyMode<'e>> {
        match e {
            Expr::Name(n) => {
                let iname = self.intern(n.id.as_str());
                if self.scope.contains_key(&iname) {
                    let l = self.stage_arg(e)?;
                    Ok(KeyMode::Staged(l))
                } else {
                    Ok(KeyMode::ByName(e))
                }
            }
            other => {
                let l = self.stage_arg(other)?;
                Ok(KeyMode::Staged(l))
            }
        }
    }

    /// `func(acc, elem)` — the 2-argument reduction call through the staged
    /// callable, or a direct by-name call (builtins / top-level functions).
    pub(super) fn emit_reduce_call(
        &mut self,
        mode: &KeyMode<'_>,
        acc: LocalId,
        elem: LocalId,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let callee = match mode {
            KeyMode::Staged(l) => self.local_ref(*l, span),
            KeyMode::ByName(expr) => self.lower_callee(expr)?,
        };
        let acc_ref = self.local_ref(acc, span);
        let elem_ref = self.local_ref(elem, span);
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![acc_ref, elem_ref],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `elem = next(it)` into a fresh pin-tagged local (null on exhaustion).
    pub(super) fn emit_iter_next(&mut self, it: LocalId, span: Span) -> LocalId {
        let elem = self.fresh_local_tagged();
        let it_ref = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterNext,
                args: vec![it_ref],
            },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign {
            target: elem,
            value: next,
        });
        elem
    }

    /// `is_exhausted(it)` as a Bool condition expr.
    pub(super) fn emit_iter_exhausted(&mut self, it: LocalId, span: Span) -> Idx<HirExpr> {
        let it_ref = self.local_ref(it, span);
        self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::IterExhausted,
                args: vec![it_ref],
            },
            SemTy::Bool,
            span,
        )
    }

    /// `key(elem)` — an indirect call through the staged key callable, or a
    /// direct by-name call (builtins / top-level functions).
    pub(super) fn emit_key_call(
        &mut self,
        mode: &KeyMode<'_>,
        elem: LocalId,
        span: Span,
    ) -> Result<Idx<HirExpr>> {
        let callee = match mode {
            KeyMode::Staged(l) => self.local_ref(*l, span),
            KeyMode::ByName(expr) => self.lower_callee(expr)?,
        };
        let arg = self.local_ref(elem, span);
        Ok(self.alloc(
            HirExprKind::Call {
                callee,
                args: vec![arg],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `set()` → empty set; `set(iterable)` → fill an empty set from the iterable.
    pub(super) fn lower_set_call(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.is_empty() {
            return Ok(self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span));
        }
        if args.len() != 1 {
            return Err(parse_error("set() takes at most 1 argument", span));
        }
        let result = self.fresh_local(SemTy::set_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign {
            target: result,
            value: empty,
        });
        let iterable = self.lower_expr(&args[0])?;
        let lp = self.begin_iter_loop(iterable, span)?;
        let elem_ref = self.local_ref(lp.elem, span);
        self.push_stmt(HirStmt::ContainerPush {
            container: result,
            value: elem_ref,
        });
        self.end_iter_loop(lp);
        Ok(self.local_ref(result, span))
    }

    /// `collections.Counter(...)` construction (§10) — a pure-frontend intercept
    /// that picks the runtime symbol by arity and types the result
    /// `RuntimeObject(Counter)`:
    ///   * `Counter()`         → `rt_make_counter_empty()`.
    ///   * `Counter(iterable)` → `rt_make_counter_from_iter(iterable)`; the
    ///     runtime normalizes the iterable to an iterator internally and counts
    ///     its elements.
    ///
    /// `Counter(mapping)` and `Counter(**kwargs)` are out of scope (a mapping
    /// iterates its keys, so it would count each key once rather than honoring the
    /// mapped counts — documented limitation).
    pub(super) fn lower_counter_construct(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        reject_call_extras(c, span, "Counter()")?;
        let cty = SemTy::RuntimeObject(pyaot_core_defs::TypeTagKind::Counter);
        match c.args.len() {
            0 => Ok(self.alloc(
                HirExprKind::CallRuntime {
                    target: pyaot_hir::RuntimeCallTarget::Func(
                        &pyaot_stdlib_defs::modules::collections::COUNTER_EMPTY,
                    ),
                    args: vec![],
                    provided: 0,
                },
                cty,
                span,
            )),
            1 => {
                let iterable = self.lower_expr(&c.args[0])?;
                Ok(self.alloc(
                    HirExprKind::CallRuntime {
                        target: pyaot_hir::RuntimeCallTarget::Func(
                            &pyaot_stdlib_defs::modules::collections::COUNTER_FROM_ITER,
                        ),
                        args: vec![Some(iterable)],
                        provided: 1,
                    },
                    cty,
                    span,
                ))
            }
            _ => Err(parse_error("Counter() takes at most 1 argument", span)),
        }
    }

    /// `collections.deque(...)` construction (§10) — a pure-frontend intercept
    /// (mirroring [`Self::lower_counter_construct`]) that picks the runtime symbol
    /// by arity and types the result `RuntimeObject(Deque)`:
    ///   * `deque()`                 → `rt_make_deque_empty()`.
    ///   * `deque(maxlen=N)`         → `rt_make_deque(N)` (empty, bounded).
    ///   * `deque(iterable)`         → `rt_make_deque_from_iter(iter(iterable))`.
    ///   * `deque(iterable, N)` /
    ///     `deque(iterable, maxlen=N)`→ `rt_deque_from_iter(iter(iterable), N)`.
    ///
    /// The iterable is wrapped in `iter()` so the runtime drives a proper iterator
    /// (any iterable — list/tuple/set/dict/deque/generator — through one seam).
    /// `maxlen` may be a keyword or the second positional; `-1` is unbounded.
    pub(super) fn lower_deque_construct(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        use pyaot_stdlib_defs::modules::collections as coll;
        if has_starred_arg(c) {
            return Err(parse_error(
                "`*args` spreading is not supported for deque()",
                span,
            ));
        }
        let dty = SemTy::RuntimeObject(pyaot_core_defs::TypeTagKind::Deque);

        // `maxlen` may come as a keyword (`deque(maxlen=N)`); other keywords and
        // `**kwargs` are rejected.
        let mut maxlen_kw: Option<&Expr> = None;
        for kw in &c.keywords {
            match kw.arg.as_ref().map(|i| i.as_str()) {
                Some("maxlen") => maxlen_kw = Some(&kw.value),
                Some(other) => {
                    return Err(parse_error(
                        format!("deque() got an unexpected keyword argument '{other}'"),
                        span,
                    ))
                }
                None => return Err(parse_error("deque() does not support **kwargs", span)),
            }
        }
        if c.args.len() > 2 {
            return Err(parse_error("deque() takes at most 2 arguments", span));
        }
        let maxlen_pos = c.args.get(1);
        if maxlen_kw.is_some() && maxlen_pos.is_some() {
            return Err(parse_error(
                "deque() got multiple values for argument 'maxlen'",
                span,
            ));
        }
        let maxlen_expr = maxlen_kw.or(maxlen_pos);
        let iterable = c.args.first();

        let alloc_call = |this: &mut Self,
                          target: &'static pyaot_stdlib_defs::StdlibFunctionDef,
                          args: Vec<Option<Idx<HirExpr>>>| {
            let provided = args.len() as u32;
            this.alloc(
                HirExprKind::CallRuntime {
                    target: pyaot_hir::RuntimeCallTarget::Func(target),
                    args,
                    provided,
                },
                dty.clone(),
                span,
            )
        };

        match (iterable, maxlen_expr) {
            (None, None) => Ok(alloc_call(self, &coll::DEQUE_EMPTY, vec![])),
            (None, Some(ml)) => {
                let ml = self.lower_expr(ml)?;
                Ok(alloc_call(self, &coll::DEQUE_MAKE_MAXLEN, vec![Some(ml)]))
            }
            (Some(it_expr), None) => {
                let it = self.lower_deque_iterable(it_expr, span)?;
                Ok(alloc_call(self, &coll::DEQUE_FROM_ITER, vec![Some(it)]))
            }
            (Some(it_expr), Some(ml)) => {
                let it = self.lower_deque_iterable(it_expr, span)?;
                let ml = self.lower_expr(ml)?;
                Ok(alloc_call(
                    self,
                    &coll::DEQUE_FROM_ITER_MAXLEN,
                    vec![Some(it), Some(ml)],
                ))
            }
        }
    }

    /// Lower a deque-construction iterable and wrap it in `iter()` so
    /// `rt_(make_)deque_from_iter` (which drives `rt_iter_next`) receives a real
    /// iterator, not a raw container.
    pub(super) fn lower_deque_iterable(&mut self, iterable: &Expr, span: Span) -> Result<Idx<HirExpr>> {
        let lowered = self.lower_expr(iterable)?;
        Ok(self.alloc(
            HirExprKind::ContainerExpr {
                op: ContainerOp::Iter,
                args: vec![lowered],
            },
            SemTy::Dyn,
            span,
        ))
    }

    /// `collections.defaultdict(...)` construction (§10) — a pure-frontend
    /// intercept that maps the factory argument to a raw tag WITHOUT lowering it
    /// as a value (a bare type Name like `set` has no binding, so `lower_expr`
    /// would fail with "undefined name 'set'"). The result is typed
    /// `defaultdict_of(Dyn, V)` where `V` is the factory's value type, so a
    /// typed-`V` read (`dd_list["k"].append(...)`) dispatches the right method on
    /// the genuinely-boxed value (`Tagged → Heap(List)` is a proof-trusted no-op).
    ///   * `defaultdict()`        → factory tag −1 (a plain dict; `KeyError` on a
    ///     missing read).
    ///   * `defaultdict(int|float|str|bool|list|dict|set)` → the matching tag.
    pub(super) fn lower_defaultdict_construct(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        reject_call_extras(c, span, "defaultdict()")?;
        let (tag, value_ty) = match c.args.len() {
            0 => (-1_i64, SemTy::Dyn),
            1 => match &c.args[0] {
                Expr::Name(n) => defaultdict_factory(n.id.as_str()).ok_or_else(|| {
                    parse_error(
                        format!(
                            "defaultdict(...) factory must be one of \
                             int/float/str/bool/list/dict/set, got `{}`",
                            n.id.as_str()
                        ),
                        span,
                    )
                })?,
                _ => {
                    return Err(parse_error(
                        "defaultdict(...) factory must be a bare type name \
                         (int/float/str/bool/list/dict/set)",
                        span,
                    ))
                }
            },
            _ => return Err(parse_error("defaultdict() takes at most 1 argument", span)),
        };
        // Capacity 0 = runtime default size; the factory tag rides the second raw
        // arg (both materialize directly into raw i64 slots).
        let cap = self.alloc(HirExprKind::IntLit(0), SemTy::Int, span);
        let tag_lit = self.alloc(HirExprKind::IntLit(tag), SemTy::Int, span);
        let dty = SemTy::defaultdict_of(SemTy::Dyn, value_ty);
        Ok(self.alloc(
            HirExprKind::CallRuntime {
                target: pyaot_hir::RuntimeCallTarget::Func(
                    &pyaot_stdlib_defs::modules::collections::DEFAULTDICT_MAKE,
                ),
                args: vec![Some(cap), Some(tag_lit)],
                provided: 2,
            },
            dty,
            span,
        ))
    }

    /// Emit the innermost comprehension element action (push / insert).
    pub(super) fn emit_comp_elem(&mut self, kind: &CompKind, span: Span) -> Result<()> {
        match kind {
            CompKind::List { result, elt } | CompKind::Set { result, elt } => {
                let v = self.lower_expr(elt)?;
                self.push_stmt(HirStmt::ContainerPush {
                    container: *result,
                    value: v,
                });
            }
            CompKind::Dict { result, key, val } => {
                let k = self.lower_expr(key)?;
                let v = self.lower_expr(val)?;
                self.push_stmt(HirStmt::ContainerInsert {
                    container: *result,
                    key: k,
                    value: v,
                });
            }
        }
        let _ = span;
        Ok(())
    }

    /// Allocate a fresh synthetic local (unnamed; never referenced by a source
    /// name) for desugared result/operand slots.
    pub(super) fn fresh_local(&mut self, ty: SemTy) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: false,
            cell_shared: false,
            deletable: false,
        });
        id
    }

    /// Evaluate a call-argument expression NOW into a fresh staged local.
    /// Keyword adaptation fills parameter slots out of written order; staging
    /// pins each argument's side effects to its written position.
    pub(super) fn stage_arg(&mut self, e: &Expr) -> Result<LocalId> {
        let value = self.lower_expr(e)?;
        let l = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign { target: l, value });
        Ok(l)
    }

    /// Stage `e` unless it is a side-effect-free literal (kept as
    /// [`ArgSrc::Plain`] for slot-fill AST folds — see [`is_const_like`]).
    pub(super) fn stage_arg_src<'e>(&mut self, e: &'e Expr) -> Result<ArgSrc<'e>> {
        if is_const_like(e) {
            Ok(ArgSrc::Plain(e))
        } else {
            Ok(ArgSrc::Staged(self.stage_arg(e)?))
        }
    }

    /// Materialize an [`ArgSrc`] at slot-fill time: lower the AST expression,
    /// or reference the already-staged local.
    pub(super) fn arg_src_value(&mut self, src: ArgSrc<'_>, span: Span) -> Result<Idx<HirExpr>> {
        match src {
            ArgSrc::Plain(e) => self.lower_expr(e),
            ArgSrc::Staged(l) => Ok(self.local_ref(l, span)),
        }
    }

    /// A fresh synthetic local pinned to the `Tagged` representation — for the slot
    /// that receives an `iter_next` result (null on exhaustion, so it must never be
    /// inferred to an unboxed `Raw(F64)`/`Raw(I8)` that would deref the null).
    pub(super) fn fresh_local_tagged(&mut self) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty: SemTy::Dyn,
            raw_int_ok: false,
            pin_tagged: true,
            cell_shared: false,
            deletable: false,
        });
        id
    }

    /// A fresh synthetic local carrying an authoritative `ty` (typeck fixes it,
    /// since `ty != Dyn`) but pinned to the `Tagged` representation. Used to
    /// "launder" a gradual `Dyn` spread value into a `float`/`bool` parameter
    /// slot: typeck skips the reinterpret check on a `pin_tagged` store (so the
    /// `Dyn → float`/`bool` assignment is admitted), and lowering unboxes the
    /// Tagged value to the param's `Raw` repr at the call site.
    pub(super) fn fresh_local_pinned(&mut self, ty: SemTy) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty,
            raw_int_ok: false,
            pin_tagged: true,
            cell_shared: false,
            deletable: false,
        });
        id
    }

    // ── closures / nested functions (Phase 6A) ────────────────────────────────

    /// A unique synthetic name for a nested function: `{outer}.<locals>.{name}#k`.
    /// The `.<locals>.` infix keeps it un-typeable by user code, and the counter
    /// disambiguates same-named siblings.
    pub(super) fn synth_name(&mut self, child: &str) -> String {
        let k = self.synth_counter;
        self.synth_counter += 1;
        format!("{}.<locals>.{child}#{k}", self.base_name)
    }

    /// The subset of a child scope's free names this scope can actually supply
    /// (its own cells), each with the cell's known content type — so an
    /// annotation (e.g. a `Callable[...]` HOF parameter) survives the capture
    /// boundary. The rest resolve through `semantics` (top-level functions,
    /// classes, builtins) or 6B globals.
    pub(super) fn capture_list(&mut self, free: &[String]) -> Vec<(String, SemTy)> {
        free.iter()
            .filter_map(|n| {
                let iname = self.interner.intern(n);
                match self.scope.get(&iname).copied() {
                    Some(Binding::Cell(lid)) | Some(Binding::Direct(lid)) => {
                        Some((n.clone(), self.locals[lid.index()].ty.clone()))
                    }
                    None => None,
                }
            })
            .collect()
    }

    /// Build the `MakeClosure` value for `fid` over `captures` (each must be a
    /// `Cell` binding here — its cell *pointer* goes into the env tuple, which is
    /// what makes the capture shared and late-bound).
    pub(super) fn make_closure_expr(
        &mut self,
        fid: FuncId,
        captures: &[(String, SemTy)],
        span: Span,
        sem_ty: SemTy,
    ) -> Result<Idx<HirExpr>> {
        let mut cap_exprs = Vec::with_capacity(captures.len());
        for (cname, _) in captures {
            let iname = self.interner.intern(cname);
            let b = self.scope.get(&iname).copied();
            let Some(Binding::Cell(cell_lid)) = b else {
                return Err(parse_error(
                    format!("internal: captured variable `{cname}` has no cell binding"),
                    span,
                ));
            };
            cap_exprs.push(self.local_ref(cell_lid, span));
        }
        Ok(self.alloc(
            HirExprKind::MakeClosure {
                func: fid,
                captures: cap_exprs,
            },
            sem_ty,
            span,
        ))
    }

}
