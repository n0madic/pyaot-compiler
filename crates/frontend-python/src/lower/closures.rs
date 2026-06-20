use super::*;

impl<'a> FnLowerer<'a> {
    /// The static `Callable` signature of an already-lowered nested function /
    /// lambda (Phase 2 of the test_functions.py lift — the call bridge). The
    /// visible signature is the function's declared params *minus* the synthetic
    /// env param 0; the result type is its return annotation (`Dyn` when
    /// unannotated, as for every lambda). Typing the `MakeClosure` value
    /// `Callable(sig)` instead of `Dyn` lets a later `f()` ride the existing
    /// `CallIndirect` path. By construction `sig_repr` of this signature equals
    /// the `Repr::Closure` `lower_make_closure` derives from the MIR sig: both
    /// flow each `SemTy` through `repr_of`, and the raw-int return/param proofs
    /// are gated off for any address-taken function (a `MakeClosure` IS the
    /// address-take), so the closure ABI stays the tagged baseline (Invariant 3,
    /// PITFALLS A4 — no per-function ABI flag).
    pub(super) fn closure_sem_ty(&self, fid: FuncId) -> SemTy {
        let f = self.shared.funcs[fid.index()]
            .as_ref()
            .expect("closure function is filled before MakeClosure");
        SemTy::Callable(Box::new(Sig {
            params: f.params[1..].iter().map(|p| p.ty.clone()).collect(),
            ret: f.ret_ty.clone(),
            varargs: f.varargs,
            kwargs: f.kwargs,
        }))
    }

    /// Build the uniform thunk over an already-lowered **nested** function `fid`
    /// (its synthetic name resolves to `Symbol::Function(fid)`), so the closure's
    /// slot 0 is the arity-generic `(args, kwargs) → Value` entry that forwards
    /// the env tuple as `fid`'s leading positional. `n_fixed` / `n_kwonly` split
    /// `fid`'s parameter list (after the env param) into positional vs
    /// keyword-only for the runtime arg→param bind. Returns the thunk's `FuncId`,
    /// which becomes the `MakeClosure` target.
    pub(super) fn uniform_thunk_over_nested(
        &mut self,
        fid: FuncId,
        n_fixed: usize,
        n_kwonly: usize,
    ) -> Result<FuncId> {
        let target = {
            let f = self.shared.funcs[fid.index()]
                .as_ref()
                .expect("nested function is filled before MakeClosure");
            // params: [env, fixed.., kwonly.., *args?, **kwargs?] — skip env (base 1).
            let base = 1;
            let fixed = f.params[base..base + n_fixed]
                .iter()
                .map(ThunkParam::from_hir_param)
                .collect();
            let kwonly = f.params[base + n_fixed..base + n_fixed + n_kwonly]
                .iter()
                .map(ThunkParam::from_hir_param)
                .collect();
            UniformTarget {
                name: f.name,
                ret: f.ret_ty.clone(),
                pass_env: true,
                fixed,
                kwonly,
                varargs: f.varargs,
                kwargs: f.kwargs,
                kw_bindable: false,
            }
        };
        build_uniform_thunk(self.interner, self.ctx, self.shared, &target)
    }

    /// Lower a nested `def` (Phase 6A): a flat synthetic function with an
    /// explicit env param, then bind `MakeClosure` to the def's name. Recursion
    /// works through self-capture: the def's own name is in the enclosing celled
    /// set, so its cell exists before the closure is stored into it.
    pub(super) fn lower_nested_def(&mut self, d: &StmtFunctionDef) -> Result<()> {
        let span = to_span(d.range());
        let facts = freevars::analyze_def(d);
        let captures = self.capture_list(&facts.free);
        let synth = self.synth_name(d.name.as_str());
        let name = self.interner.intern(&synth);
        // A nested def must NOT inherit the enclosing top-level def's
        // `default_slots`: a process-global slot cannot hold a per-closure-
        // instance capture, so a non-literal default here is a clean error.
        let nested_ctx = AnnCtx {
            default_slots: None,
            ..*self.ctx
        };
        let fid = lower_callable(
            self.interner,
            &nested_ctx,
            self.shared,
            d,
            &synth,
            name,
            FirstParam::Plain,
            self.enclosing_class,
            false,
            Some((&captures, &facts)),
        )?;
        let mc_ty = self.closure_sem_ty(fid);
        // Slot 0 of the closure is the uniform thunk over `fid`, not `fid` itself
        // (the specialized native entry survives only for direct by-name calls).
        let n_fixed = d.args.posonlyargs.len() + d.args.args.len();
        let n_kwonly = d.args.kwonlyargs.len();
        let thunk = self.uniform_thunk_over_nested(fid, n_fixed, n_kwonly)?;
        let mc = self.make_closure_expr(thunk, &captures, span, mc_ty.clone())?;
        let dname = self.intern(d.name.as_str());
        self.write_named(dname, mc_ty, mc);
        Ok(())
    }

    /// A nested `class` statement (FIX 2). The class was already registered in
    /// `class_map` and lowered into `self.classes` by the module pre-scan, and
    /// its name resolves at every use site (construction / `isinstance` /
    /// annotation) through `ctx.class_map`, not `self.scope` — so this arm binds
    /// nothing and emits no code. It only rejects the decorations / defaults the
    /// nested class path cannot express (consistent with nested `def`s).
    pub(super) fn lower_nested_classdef(&mut self, c: &StmtClassDef) -> Result<()> {
        let span = to_span(c.range());
        // A decorated nested class is out of scope: class decorators run at
        // module-init over the class-id int (`emit_class_decorators`), which the
        // nested pre-scan path does not replay.
        if !c.decorator_list.is_empty() {
            return Err(parse_error(
                "a decorated nested class is out of scope; define it at module level",
                span,
            ));
        }
        // A method default-slot (a non-literal default) is promoted to a global
        // slot only for top-level classes (`emit_class_default_slots`); a nested
        // class method with a non-literal default would silently read an
        // uninitialized slot. A literal default folds to a `Const` and is fine.
        for stmt in &c.body {
            let Stmt::FunctionDef(m) = stmt else { continue };
            for awd in m
                .args
                .posonlyargs
                .iter()
                .chain(&m.args.args)
                .chain(&m.args.kwonlyargs)
            {
                let Some(dflt) = &awd.default else { continue };
                if try_literal_default(&mut *self.interner, dflt).is_none() {
                    return Err(parse_error(
                        "a nested class method with a non-literal default is out of \
                         scope; define the class at module level",
                        to_span(m.range()),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Lower a lambda (Phase 6A): a synthetic single-`Return` nested function.
    pub(super) fn lower_lambda(&mut self, l: &ExprLambda, span: Span) -> Result<Idx<HirExpr>> {
        let args = l.args.as_ref();
        if args.vararg.is_some() || args.kwarg.is_some() || !args.kwonlyargs.is_empty() {
            return Err(parse_error("lambda *args/**kwargs are out of scope", span));
        }
        if args
            .posonlyargs
            .iter()
            .chain(args.args.iter())
            .any(|a| a.default.is_some())
        {
            return Err(parse_error(
                "lambda default arguments are out of scope",
                span,
            ));
        }
        let facts = freevars::analyze_lambda(l);
        let captures = self.capture_list(&facts.free);
        let synth = self.synth_name("<lambda>");
        let name = self.interner.intern(&synth);

        let fid = self.shared.reserve();
        let mut fl = FnLowerer::new(
            self.interner,
            self.ctx,
            self.shared,
            name,
            &synth,
            SemTy::Dyn,
            None,
        );
        fl.set_scope_facts(&facts);
        let env_name = fl.intern("__env__");
        fl.add_param(env_name, SemTy::Dyn);
        for awd in args.posonlyargs.iter().chain(args.args.iter()) {
            let pname = fl.intern(awd.def.arg.as_str());
            fl.add_param(pname, SemTy::Dyn);
        }
        fl.install_captures(&captures, &facts, span);
        fl.init_cells();
        let body = fl.lower_expr(l.body.as_ref())?;
        fl.seal(HirTerminator::Return(Some(body)));
        let f = fl.finish(HirTerminator::Return(None));
        self.shared.fill(fid, f);

        let lam_ty = self.closure_sem_ty(fid);
        // Slot 0 is the uniform thunk over the lambda body (lambdas reject
        // keyword-only / *args / **kwargs / defaults, so all params are fixed).
        let n_fixed = args.posonlyargs.len() + args.args.len();
        let thunk = self.uniform_thunk_over_nested(fid, n_fixed, 0)?;
        self.make_closure_expr(thunk, &captures, span, lam_ty)
    }

    /// Install capture bindings: capture `i` is read out of env slot `i+1` into
    /// a fresh cell-holding local in the prologue, carrying the content type the
    /// enclosing scope knew for it.
    pub(super) fn install_captures(&mut self, captures: &[(String, SemTy)], facts: &ScopeFacts, span: Span) {
        for (i, (cname, content_ty)) in captures.iter().enumerate() {
            let iname = self.interner.intern(cname);
            let cell_lid =
                self.alloc_cell_local(iname, content_ty.clone(), facts.nonlocals.contains(cname));
            let env_ref = self.local_ref(LocalId::new(0), span);
            let idx_e = self.alloc(HirExprKind::IntLit(i as i64 + 1), SemTy::Int, span);
            let tg = self.alloc(
                HirExprKind::ContainerExpr {
                    op: ContainerOp::TupleGet,
                    args: vec![env_ref, idx_e],
                },
                SemTy::Dyn,
                span,
            );
            self.push_stmt(HirStmt::Assign {
                target: cell_lid,
                value: tg,
            });
            self.scope.insert(iname, Binding::Cell(cell_lid));
        }
    }

    /// A top-level function referenced as a VALUE (Phase 6A): the memoized
    /// **uniform thunk** over `f` wrapped in a captureless closure — `f`'s own
    /// direct-call ABI is untouched. The thunk binds the packed call args to
    /// `f`'s parameters at run time (positional, defaults, `*args`), so a function
    /// with defaults/varargs is callable as a value through the single uniform
    /// indirect ABI (no arity match required at the call site any more).
    pub(super) fn lower_top_fn_value(&mut self, fname: &str, span: Span) -> Result<Idx<HirExpr>> {
        let thunk_key = (self.shared.current_ns, fname.to_string());
        let fid = match self.shared.thunks.get(&thunk_key) {
            Some(f) => *f,
            None => {
                let info = self.ctx.top_defs[fname].clone();
                let name = self.interner.intern(fname);
                let target = UniformTarget::from_top_def(name, &info);
                let fid = build_uniform_thunk(self.interner, self.ctx, self.shared, &target)?;
                self.shared.thunks.insert(thunk_key, fid);
                fid
            }
        };
        Ok(self.alloc(
            HirExprKind::MakeClosure {
                func: fid,
                captures: vec![],
            },
            SemTy::Dyn,
            span,
        ))
    }

}
