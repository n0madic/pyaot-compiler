//! HIR construction: module + per-function CFG building.
//!
//! [`FnLowerer`] is a block builder. Statements append to the *current* block;
//! emitting a terminator seals it and switches to a successor. Block-producing
//! expressions (short-circuit `and`/`or`, ternary, chained compares) split the
//! current block and route through a single-eval result local.
//!
//! The implemented subset grows per milestone; anything outside it returns a
//! [`CompilerError::parse_error`].

use std::collections::HashMap;

use la_arena::{Arena, Idx};
use rustpython_parser::ast::{
    BoolOp as PyBoolOp, CmpOp as PyCmpOp, Comprehension, Constant, Expr, ExprBinOp, ExprBoolOp,
    ExprCall, ExprCompare, ExprDictComp, ExprIfExp, ExprListComp, ExprSetComp, ExprSubscript,
    ExprUnaryOp, Keyword, Operator as PyOperator, Ranged, Stmt, UnaryOp as PyUnaryOp,
};
use rustpython_parser::text_size::TextRange;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{
    BinOp, CmpOp, ContainerMethod, ContainerOp, HirBlock, HirExpr, HirExprKind, HirFunction,
    HirLocal, HirModule, HirParam, HirStmt, HirTerminator, SymbolRef, UnaryOp,
};
use pyaot_types::SemTy;
use pyaot_utils::{FuncId, InternedString, LocalId, Span, StringInterner};

pub(crate) struct ModuleLowerer<'a> {
    interner: &'a mut StringInterner,
}

impl<'a> ModuleLowerer<'a> {
    pub(crate) fn new(interner: &'a mut StringInterner) -> Self {
        Self { interner }
    }

    /// Lower a module body into an [`HirModule`]: `__main__` (the module body,
    /// `FuncId(0)`) followed by each top-level `def`. Cross-function references
    /// (recursion / forward calls) resolve later in `semantics`, which sees the
    /// complete function table — so no frontend pre-pass is needed.
    pub(crate) fn lower_module(self, body: Vec<Stmt>) -> Result<HirModule> {
        let interner = self.interner;

        // Partition top-level statements: `def`s vs. module-body statements.
        let mut defs: Vec<&rustpython_parser::ast::StmtFunctionDef> = Vec::new();
        let mut top: Vec<&Stmt> = Vec::new();
        for stmt in &body {
            match stmt {
                Stmt::FunctionDef(f) => defs.push(f),
                other => top.push(other),
            }
        }

        let mut functions: Vec<HirFunction> = Vec::new();

        // __main__ = FuncId(0). `__name__` is pre-bound to "__main__" so that
        // `if __name__ == "__main__":` evaluates normally.
        let main_name = interner.intern("__main__");
        let mut main = FnLowerer::new(&mut *interner, main_name, SemTy::NoneTy);
        let dunder_name = main.intern("__name__");
        let name_lid = main.declare_local(dunder_name, SemTy::Str);
        let main_str = main.intern("__main__");
        let name_val = main.alloc(HirExprKind::StrLit(main_str), SemTy::Str, Span::dummy());
        main.push_stmt(HirStmt::Assign { target: name_lid, value: name_val });
        for stmt in &top {
            if main.lower_stmt(stmt)? {
                break;
            }
        }
        functions.push(main.finish(HirTerminator::Return(None)));

        for def in &defs {
            functions.push(lower_def(&mut *interner, def)?);
        }

        Ok(HirModule { functions, main: FuncId::new(0) })
    }
}

/// A loop's jump targets, pushed while lowering its body.
struct LoopCtx {
    continue_to: Idx<HirBlock>,
    break_to: Idx<HirBlock>,
}

/// The element action of a comprehension: append to a list/set, or insert into a
/// dict. Carries the result container local plus the borrowed element expressions.
enum CompKind<'a> {
    List { result: LocalId, elt: &'a Expr },
    Set { result: LocalId, elt: &'a Expr },
    Dict { result: LocalId, key: &'a Expr, val: &'a Expr },
}

/// A simple iterator loop opened by `begin_iter_loop`: the header to jump back to,
/// the exit block to continue at, and the per-iteration element local.
struct IterLoop {
    header: Idx<HirBlock>,
    exit: Idx<HirBlock>,
    elem: LocalId,
}

pub(crate) struct FnLowerer<'a> {
    interner: &'a mut StringInterner,
    name: InternedString,
    params: Vec<HirParam>,
    ret_ty: SemTy,
    exprs: Arena<HirExpr>,
    blocks: Arena<HirBlock>,
    locals: Vec<HirLocal>,
    scope: HashMap<InternedString, LocalId>,
    entry: Idx<HirBlock>,
    cur: Idx<HirBlock>,
    loop_stack: Vec<LoopCtx>,
}

impl<'a> FnLowerer<'a> {
    pub(crate) fn new(
        interner: &'a mut StringInterner,
        name: InternedString,
        ret_ty: SemTy,
    ) -> Self {
        let mut blocks = Arena::new();
        let entry = blocks.alloc(HirBlock { stmts: Vec::new(), term: HirTerminator::Unreachable });
        Self {
            interner,
            name,
            params: Vec::new(),
            ret_ty,
            exprs: Arena::new(),
            blocks,
            locals: Vec::new(),
            scope: HashMap::new(),
            entry,
            cur: entry,
            loop_stack: Vec::new(),
        }
    }

    /// Register a parameter as the next local (params occupy locals `0..nparams`).
    fn add_param(&mut self, name: InternedString, ty: SemTy) {
        let id = LocalId::new(self.locals.len() as u32);
        self.params.push(HirParam { name, ty: ty.clone() });
        self.locals.push(HirLocal { name, ty, raw_int_ok: false, pin_tagged: false });
        self.scope.insert(name, id);
    }

    /// Seal the current block with `default_term` if it is still open, then
    /// assemble the [`HirFunction`].
    pub(crate) fn finish(mut self, default_term: HirTerminator) -> HirFunction {
        if matches!(self.blocks[self.cur].term, HirTerminator::Unreachable) {
            self.blocks[self.cur].term = default_term;
        }
        HirFunction {
            name: self.name,
            params: self.params,
            ret_ty: self.ret_ty,
            locals: self.locals,
            blocks: self.blocks,
            entry: self.entry,
            exprs: self.exprs,
        }
    }

    // ── block builder ──────────────────────────────────────────────────────

    fn new_block(&mut self) -> Idx<HirBlock> {
        self.blocks.alloc(HirBlock { stmts: Vec::new(), term: HirTerminator::Unreachable })
    }

    fn push_stmt(&mut self, stmt: HirStmt) {
        self.blocks[self.cur].stmts.push(stmt);
    }

    /// Seal the current block with `term` (only if still open) and leave `cur`
    /// pointing at it; the caller must `switch` to a fresh block next.
    fn seal(&mut self, term: HirTerminator) {
        if matches!(self.blocks[self.cur].term, HirTerminator::Unreachable) {
            self.blocks[self.cur].term = term;
        }
    }

    fn switch(&mut self, block: Idx<HirBlock>) {
        self.cur = block;
    }

    fn alloc(&mut self, kind: HirExprKind, ty: SemTy, span: Span) -> Idx<HirExpr> {
        self.exprs.alloc(HirExpr { kind, ty, span })
    }

    fn intern(&mut self, s: &str) -> InternedString {
        self.interner.intern(s)
    }

    // ── statements ──────────────────────────────────────────────────────────

    /// Lower a statement list, stopping after a statement that terminates the
    /// current block (so trailing dead code is not emitted into a sealed block).
    fn lower_body(&mut self, stmts: &[Stmt]) -> Result<()> {
        for stmt in stmts {
            if self.lower_stmt(stmt)? {
                break;
            }
        }
        Ok(())
    }

    /// Lower one statement. Returns `true` if it terminated the current block
    /// (`break` / `continue` / `return`).
    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<bool> {
        match stmt {
            Stmt::Expr(s) => {
                // `print(...)` is the one special statement (it carries sep/end).
                if let Some(call) = as_print_call(s.value.as_ref()) {
                    self.lower_print(call)?;
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
                let cond = self.lower_expr(s.test.as_ref())?;
                self.push_stmt(HirStmt::Assert { cond });
                Ok(false)
            }
            Stmt::Pass(_) => Ok(false),
            Stmt::Break(b) => {
                let target = self
                    .loop_stack
                    .last()
                    .map(|c| c.break_to)
                    .ok_or_else(|| parse_error("'break' outside loop", to_span(b.range())))?;
                self.seal(HirTerminator::Jump(target));
                Ok(true)
            }
            Stmt::Continue(c) => {
                let target = self
                    .loop_stack
                    .last()
                    .map(|c| c.continue_to)
                    .ok_or_else(|| parse_error("'continue' outside loop", to_span(c.range())))?;
                self.seal(HirTerminator::Jump(target));
                Ok(true)
            }
            Stmt::Return(r) => {
                let val = match &r.value {
                    Some(e) => Some(self.lower_expr(e.as_ref())?),
                    None => None,
                };
                self.seal(HirTerminator::Return(val));
                Ok(true)
            }
            other => Err(parse_error(
                "unsupported statement for this milestone",
                to_span(other.range()),
            )),
        }
    }

    /// `a = b = value` — evaluate `value` once, assign to each target (a `Name` or
    /// a subscript `base[index]`).
    fn lower_assign(&mut self, a: &rustpython_parser::ast::StmtAssign) -> Result<()> {
        // Tuple/list unpacking target: `a, b = …` / `a, b = c, d`.
        if a.targets.len() == 1 {
            if let Some(targets) = seq_target_elts(&a.targets[0]) {
                let span = to_span(a.range());
                // A literal sequence RHS unpacks element-wise with a static arity
                // check and no intermediate tuple; anything else stages a tuple and
                // reads it back positionally.
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
    fn assign_to_target(&mut self, target: &Expr, value: Idx<HirExpr>) -> Result<()> {
        match target {
            Expr::Name(_) => {
                let lid = self.assign_target(target)?;
                self.push_stmt(HirStmt::Assign { target: lid, value });
                Ok(())
            }
            Expr::Subscript(s) => {
                let span = to_span(s.range());
                if matches!(s.slice.as_ref(), Expr::Slice(_)) {
                    return Err(parse_error("slice assignment is not yet supported", span));
                }
                let base = self.lower_expr(s.value.as_ref())?;
                let index = self.lower_expr(s.slice.as_ref())?;
                self.push_stmt(HirStmt::SetItem { base, index, value });
                Ok(())
            }
            Expr::Tuple(_) | Expr::List(_) => Err(parse_error(
                "tuple/list unpacking assignment is not yet supported",
                to_span(target.range()),
            )),
            other => Err(parse_error(
                "unsupported assignment target",
                to_span(other.range()),
            )),
        }
    }

    /// Unpack a literal-sequence RHS (`a, b = e0, e1`): stage every RHS value
    /// first (so `a, b = b, a` swaps correctly), then bind each target — no
    /// intermediate tuple allocation.
    fn lower_unpack_literal(
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
            self.push_stmt(HirStmt::Assign { target: tmp, value: vv });
            staged.push(tmp);
        }
        for (target, tmp) in targets.iter().zip(staged) {
            let v = self.local_ref(tmp, span);
            self.assign_to_target(target, v)?;
        }
        Ok(())
    }

    /// Unpack an arbitrary iterable RHS (`a, b = expr`, `for k, v in pairs`): stage
    /// the value once, then bind `target_i = tmp[i]` via positional subscripts.
    /// Arity beyond the pattern is a runtime `IndexError` (no static shape here).
    fn lower_unpack_subscript(
        &mut self,
        targets: &[Expr],
        value: Idx<HirExpr>,
        span: Span,
    ) -> Result<()> {
        reject_starred(targets, span)?;
        let tmp = self.fresh_local(SemTy::Dyn);
        self.push_stmt(HirStmt::Assign { target: tmp, value });
        for (i, target) in targets.iter().enumerate() {
            let tmp_ref = self.local_ref(tmp, span);
            let idx = self.alloc(HirExprKind::IntLit(i as i64), SemTy::Int, span);
            let sub = self.alloc(
                HirExprKind::Subscript { base: tmp_ref, index: idx },
                SemTy::Dyn,
                span,
            );
            self.assign_to_target(target, sub)?;
        }
        Ok(())
    }

    fn lower_augassign(&mut self, a: &rustpython_parser::ast::StmtAugAssign) -> Result<()> {
        let span = to_span(a.range());
        let lid = self.assign_target(a.target.as_ref())?;
        let op = binop_from_ast(&a.op, span)?;
        let l = self.local_ref(lid, span);
        let r = self.lower_expr(a.value.as_ref())?;
        let combined = self.alloc(HirExprKind::BinOp { op, l, r }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign { target: lid, value: combined });
        Ok(())
    }

    fn lower_annassign(&mut self, a: &rustpython_parser::ast::StmtAnnAssign) -> Result<()> {
        let span = to_span(a.range());
        let ty = annotation_to_semty(a.annotation.as_ref());
        let Expr::Name(n) = a.target.as_ref() else {
            return Err(parse_error("annotated assignment target must be a name", span));
        };
        let name = self.intern(n.id.as_str());
        let lid = self.declare_local(name, ty);
        if let Some(value) = &a.value {
            let v = self.lower_expr(value.as_ref())?;
            self.push_stmt(HirStmt::Assign { target: lid, value: v });
        }
        Ok(())
    }

    /// Resolve an assignment target name to its local (allocating on first use).
    fn assign_target(&mut self, target: &Expr) -> Result<LocalId> {
        let Expr::Name(n) = target else {
            return Err(parse_error(
                "only simple name assignment targets are supported",
                to_span(target.range()),
            ));
        };
        let name = self.intern(n.id.as_str());
        Ok(self.declare_local(name, SemTy::Dyn))
    }

    /// Look up or allocate a named local. A new local takes `ty`; an existing one
    /// keeps its slot (flat per-function scope).
    fn declare_local(&mut self, name: InternedString, ty: SemTy) -> LocalId {
        if let Some(lid) = self.scope.get(&name).copied() {
            return lid;
        }
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal { name, ty, raw_int_ok: false, pin_tagged: false });
        self.scope.insert(name, id);
        id
    }

    fn lower_if(&mut self, s: &rustpython_parser::ast::StmtIf) -> Result<()> {
        let cond = self.lower_expr(s.test.as_ref())?;
        let then_b = self.new_block();
        let join = self.new_block();
        let else_b = if s.orelse.is_empty() { join } else { self.new_block() };
        self.seal(HirTerminator::Branch { cond, then: then_b, else_: else_b });

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

    fn lower_while(&mut self, s: &rustpython_parser::ast::StmtWhile) -> Result<bool> {
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let cond = self.lower_expr(s.test.as_ref())?;
        let body_b = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() { exit } else { self.new_block() };
        self.seal(HirTerminator::Branch { cond, then: body_b, else_: else_b });

        self.switch(body_b);
        self.loop_stack.push(LoopCtx { continue_to: header, break_to: exit });
        self.lower_body(&s.body)?;
        self.loop_stack.pop();
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    fn lower_for(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        // The `range(...)` fast path (Phase 3c raw-i64 cursors) is preserved
        // verbatim; any other iterable takes the general iterator-protocol path.
        if is_range_call(s.iter.as_ref()) {
            self.lower_for_range(s)
        } else {
            self.lower_for_iter(s)
        }
    }

    /// General `for target in <iterable>`: drive the runtime iterator protocol
    /// (`iter` → `next` → `is_exhausted`), binding the target (a name or a tuple
    /// pattern) each iteration. `for`-else / `break` / `continue` reuse the loop
    /// stack exactly as the `while`/range paths do.
    fn lower_for_iter(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        let span = to_span(s.range());

        // it = iter(iterable)  — a Heap(Iterator) local, live across the loop.
        let iterable = self.lower_expr(s.iter.as_ref())?;
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::Iter, args: vec![iterable] },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign { target: it, value: iter_expr });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);

        // elem = next(it)   then   done = is_exhausted(it)  (this call order is the
        // runtime contract: `next` advances and sets the exhausted flag).
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next_expr = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::IterNext, args: vec![it_ref1] },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign { target: elem, value: next_expr });
        let it_ref2 = self.local_ref(it, span);
        let done = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::IterExhausted, args: vec![it_ref2] },
            SemTy::Bool,
            span,
        );

        let body_b = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() { exit } else { self.new_block() };
        // done == true → exit (or the for-else); else run the body.
        self.seal(HirTerminator::Branch { cond: done, then: else_b, else_: body_b });

        self.switch(body_b);
        let elem_ref = self.local_ref(elem, span);
        self.bind_for_target(s.target.as_ref(), elem_ref, span)?;
        self.loop_stack.push(LoopCtx { continue_to: header, break_to: exit });
        self.lower_body(&s.body)?;
        self.loop_stack.pop();
        self.seal(HirTerminator::Jump(header));

        if !s.orelse.is_empty() {
            self.switch(else_b);
            self.lower_body(&s.orelse)?;
            self.seal(HirTerminator::Jump(exit));
        }

        self.switch(exit);
        Ok(false)
    }

    /// Bind a `for`-loop target: a simple name, or a tuple/list pattern unpacked
    /// element-wise (`for k, v in pairs`).
    fn bind_for_target(&mut self, target: &Expr, value: Idx<HirExpr>, span: Span) -> Result<()> {
        match target {
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                let lid = self.declare_local(name, SemTy::Dyn);
                self.push_stmt(HirStmt::Assign { target: lid, value });
                Ok(())
            }
            _ => match seq_target_elts(target) {
                Some(elts) => self.lower_unpack_subscript(elts, value, span),
                None => Err(parse_error("unsupported for-loop target", span)),
            },
        }
    }

    /// The preserved Phase-3 `range(...)` loop with proof-gated raw-i64 cursors.
    fn lower_for_range(&mut self, s: &rustpython_parser::ast::StmtFor) -> Result<bool> {
        let span = to_span(s.range());
        let (start, stop, step) = parse_range(s.iter.as_ref(), span)?;
        if step == 0 {
            return Err(parse_error("range() step argument must not be zero", span));
        }
        let Expr::Name(n) = s.target.as_ref() else {
            return Err(parse_error("for-loop target must be a simple name", span));
        };
        let i_name = self.intern(n.id.as_str());
        let i_lid = self.declare_local(i_name, SemTy::Dyn);
        let cursor = self.fresh_local(SemTy::Dyn);
        let stop_l = self.fresh_local(SemTy::Dyn);

        // Phase 3c: a literal-bounded `range()` cursor provably stays in a small
        // i64 sub-range, so the loop compare and increment can run on raw machine
        // i64 (no tagging, no `rt_obj_*` call). Flag the cursor + stop slot; the
        // loop variable `i` stays tagged (it is read in the body, where derived
        // expressions like `i * i` could leave the proven range — PITFALLS A6).
        // Lowering narrows the flagged slots to `Raw(I64)` only after typeck
        // confirms they are `int`. Non-literal or out-of-bounds ranges stay
        // tagged (the always-correct baseline).
        if range_is_raw_int_eligible(&start, &stop, step) {
            self.locals[cursor.index()].raw_int_ok = true;
            self.locals[stop_l.index()].raw_int_ok = true;
        }

        // cursor = start; stop_l = stop  (range args evaluated once).
        let s_idx = self.lower_range_arg(&start, span)?;
        self.push_stmt(HirStmt::Assign { target: cursor, value: s_idx });
        let stop_idx = self.lower_range_arg(&stop, span)?;
        self.push_stmt(HirStmt::Assign { target: stop_l, value: stop_idx });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let cursor_ref = self.local_ref(cursor, span);
        let stop_ref = self.local_ref(stop_l, span);
        let cmp_op = if step > 0 { CmpOp::Lt } else { CmpOp::Gt };
        let cond = self.alloc(
            HirExprKind::Compare { op: cmp_op, l: cursor_ref, r: stop_ref },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let incr = self.new_block();
        let exit = self.new_block();
        let else_b = if s.orelse.is_empty() { exit } else { self.new_block() };
        self.seal(HirTerminator::Branch { cond, then: body_b, else_: else_b });

        self.switch(body_b);
        // i = cursor
        let cref = self.local_ref(cursor, span);
        self.push_stmt(HirStmt::Assign { target: i_lid, value: cref });
        self.loop_stack.push(LoopCtx { continue_to: incr, break_to: exit });
        self.lower_body(&s.body)?;
        self.loop_stack.pop();
        self.seal(HirTerminator::Jump(incr));

        // incr: cursor = cursor + step
        self.switch(incr);
        let cref2 = self.local_ref(cursor, span);
        let step_kind = self.int_literal_const(step);
        let step_lit = self.alloc(step_kind, SemTy::Int, span);
        let inc = self.alloc(HirExprKind::BinOp { op: BinOp::Add, l: cref2, r: step_lit }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign { target: cursor, value: inc });
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
    fn lower_range_arg(&mut self, arg: &RangeArg, span: Span) -> Result<Idx<HirExpr>> {
        match arg {
            RangeArg::Zero => Ok(self.alloc(HirExprKind::IntLit(0), SemTy::Int, span)),
            RangeArg::Expr(e) => self.lower_expr(e),
        }
    }

    /// A fixnum/bignum int-literal expr kind (used for the loop step).
    fn int_literal_const(&mut self, v: i64) -> HirExprKind {
        if pyaot_core_defs::int_fits(v) {
            HirExprKind::IntLit(v)
        } else {
            HirExprKind::BigIntLit(self.intern(&v.to_string()))
        }
    }

    /// `print(args, sep=…, end=…)` → [`HirStmt::Print`].
    fn lower_print(&mut self, call: &rustpython_parser::ast::ExprCall) -> Result<()> {
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
    fn kw_str_literal(&mut self, kw: &Keyword, name: &str) -> Result<InternedString> {
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

    // ── expressions ──────────────────────────────────────────────────────────

    fn lower_expr(&mut self, expr: &Expr) -> Result<Idx<HirExpr>> {
        let span = to_span(expr.range());
        match expr {
            Expr::Constant(c) => self.lower_constant(&c.value, span),
            Expr::Name(n) => {
                let name = self.intern(n.id.as_str());
                // A name the frontend already has in scope resolves directly to
                // its local; everything else defers to `semantics`.
                if let Some(lid) = self.scope.get(&name).copied() {
                    let ty = self.locals[lid.index()].ty.clone();
                    Ok(self.alloc(HirExprKind::Local(lid), ty, span))
                } else {
                    Ok(self.alloc(HirExprKind::Name(SymbolRef::Unresolved(name)), SemTy::Dyn, span))
                }
            }
            Expr::UnaryOp(u) => self.lower_unary(u, span),
            Expr::BinOp(b) => self.lower_binop(b, span),
            Expr::Compare(c) => self.lower_compare(c, span),
            Expr::BoolOp(b) => self.lower_boolop(b),
            Expr::IfExp(e) => self.lower_ifexp(e),
            Expr::Call(c) => self.lower_call_expr(c, span),
            // ── containers (Phase 4) ──
            Expr::List(l) => {
                let elems = self.lower_expr_list(&l.elts)?;
                Ok(self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span))
            }
            Expr::Tuple(t) => {
                let elems = self.lower_expr_list(&t.elts)?;
                Ok(self.alloc(HirExprKind::TupleLit { elems }, SemTy::Dyn, span))
            }
            Expr::Set(s) => {
                let elems = self.lower_expr_list(&s.elts)?;
                Ok(self.alloc(HirExprKind::SetLit { elems }, SemTy::Dyn, span))
            }
            Expr::Dict(d) => {
                let mut pairs = Vec::with_capacity(d.values.len());
                for (k, v) in d.keys.iter().zip(d.values.iter()) {
                    let Some(k) = k else {
                        return Err(parse_error("dict unpacking (`**`) is out of scope", span));
                    };
                    let kk = self.lower_expr(k)?;
                    let vv = self.lower_expr(v)?;
                    pairs.push((kk, vv));
                }
                Ok(self.alloc(HirExprKind::DictLit { pairs }, SemTy::Dyn, span))
            }
            Expr::Subscript(s) => self.lower_subscript_expr(s, span),
            Expr::ListComp(c) => self.lower_listcomp(c, span),
            Expr::SetComp(c) => self.lower_setcomp(c, span),
            Expr::DictComp(c) => self.lower_dictcomp(c, span),
            other => Err(parse_error(
                "unsupported expression for this milestone",
                to_span(other.range()),
            )),
        }
    }

    /// Lower a list of expressions (literal elements).
    fn lower_expr_list(&mut self, exprs: &[Expr]) -> Result<Vec<Idx<HirExpr>>> {
        exprs.iter().map(|e| self.lower_expr(e)).collect()
    }

    /// Lower a subscript read `value[index]`. Slicing is deferred.
    fn lower_subscript_expr(&mut self, s: &ExprSubscript, span: Span) -> Result<Idx<HirExpr>> {
        if matches!(s.slice.as_ref(), Expr::Slice(_)) {
            return Err(parse_error("slicing is not yet supported", span));
        }
        let base = self.lower_expr(s.value.as_ref())?;
        let index = self.lower_expr(s.slice.as_ref())?;
        Ok(self.alloc(HirExprKind::Subscript { base, index }, SemTy::Dyn, span))
    }

    // ── comprehensions (Phase 4C) ──────────────────────────────────────────────

    /// `[elt for … if …]` → an empty list built by nesting an iterator loop per
    /// `for` clause; each `if` clause branches past the element push.
    fn lower_listcomp(&mut self, c: &ExprListComp, span: Span) -> Result<Idx<HirExpr>> {
        let result = self.fresh_local(SemTy::list_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::ListLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign { target: result, value: empty });
        let kind = CompKind::List { result, elt: c.elt.as_ref() };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// `{elt for … if …}` → an empty set filled the same way.
    fn lower_setcomp(&mut self, c: &ExprSetComp, span: Span) -> Result<Idx<HirExpr>> {
        let result = self.fresh_local(SemTy::set_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign { target: result, value: empty });
        let kind = CompKind::Set { result, elt: c.elt.as_ref() };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// `{k: v for … if …}` → an empty dict filled key/value-wise.
    fn lower_dictcomp(&mut self, c: &ExprDictComp, span: Span) -> Result<Idx<HirExpr>> {
        let result = self.fresh_local(SemTy::dict_of(SemTy::Dyn, SemTy::Dyn));
        let empty = self.alloc(HirExprKind::DictLit { pairs: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign { target: result, value: empty });
        let kind = CompKind::Dict { result, key: c.key.as_ref(), val: c.value.as_ref() };
        self.lower_comp_clauses(&c.generators, 0, &kind, span)?;
        Ok(self.local_ref(result, span))
    }

    /// Nest the comprehension's `for`/`if` clauses (one iterator loop per `for`),
    /// emitting the element action at the innermost point.
    fn lower_comp_clauses(
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

        // it = iter(gen.iter)
        let iterable = self.lower_expr(&gen.iter)?;
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::Iter, args: vec![iterable] },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign { target: it, value: iter_expr });

        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::IterNext, args: vec![it_ref1] },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign { target: elem, value: next });
        let it_ref2 = self.local_ref(it, span);
        let done = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::IterExhausted, args: vec![it_ref2] },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch { cond: done, then: exit, else_: body_b });

        self.switch(body_b);
        let elem_ref = self.local_ref(elem, span);
        self.bind_for_target(&gen.target, elem_ref, span)?;
        // Filters: a false `if` skips to the next element (jump back to header).
        for cond_expr in &gen.ifs {
            let cond = self.lower_expr(cond_expr)?;
            let cont = self.new_block();
            self.seal(HirTerminator::Branch { cond, then: cont, else_: header });
            self.switch(cont);
        }
        // Recurse into the next clause (or emit the element at the innermost).
        self.lower_comp_clauses(generators, idx + 1, kind, span)?;
        self.seal(HirTerminator::Jump(header));
        self.switch(exit);
        Ok(())
    }

    // ── reduce/loop builtins: sum / min / max / set (Phase 4C) ─────────────────

    /// Emit the iterator-protocol prologue for a simple loop over an
    /// already-lowered iterable, switching to the loop body and returning the
    /// per-iteration element local plus the header/exit blocks. Pair with
    /// [`Self::end_iter_loop`]. (Used by `sum`/`min`/`max`/`set` — no target
    /// binding, filters, or `break`/`continue`, unlike the `for`/comprehension
    /// paths.)
    fn begin_iter_loop(&mut self, iterable: Idx<HirExpr>, span: Span) -> Result<IterLoop> {
        let it = self.fresh_local(SemTy::Dyn);
        let iter_expr = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::Iter, args: vec![iterable] },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign { target: it, value: iter_expr });
        let header = self.new_block();
        self.seal(HirTerminator::Jump(header));
        self.switch(header);
        let elem = self.fresh_local_tagged();
        let it_ref1 = self.local_ref(it, span);
        let next = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::IterNext, args: vec![it_ref1] },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign { target: elem, value: next });
        let it_ref2 = self.local_ref(it, span);
        let done = self.alloc(
            HirExprKind::ContainerExpr { op: ContainerOp::IterExhausted, args: vec![it_ref2] },
            SemTy::Bool,
            span,
        );
        let body_b = self.new_block();
        let exit = self.new_block();
        self.seal(HirTerminator::Branch { cond: done, then: exit, else_: body_b });
        self.switch(body_b);
        Ok(IterLoop { header, exit, elem })
    }

    /// Close a [`Self::begin_iter_loop`] loop: jump back to the header and switch
    /// to the exit block.
    fn end_iter_loop(&mut self, lp: IterLoop) {
        self.seal(HirTerminator::Jump(lp.header));
        self.switch(lp.exit);
    }

    /// `sum(iterable[, start])` → `acc = start; for x in iterable: acc = acc + x`.
    fn lower_sum(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.is_empty() || args.len() > 2 {
            return Err(parse_error("sum() takes 1 or 2 arguments", span));
        }
        let acc = self.fresh_local(SemTy::Dyn);
        let start = match args.get(1) {
            Some(s) => self.lower_expr(s)?,
            None => self.alloc(HirExprKind::IntLit(0), SemTy::Int, span),
        };
        self.push_stmt(HirStmt::Assign { target: acc, value: start });
        let iterable = self.lower_expr(&args[0])?;
        let lp = self.begin_iter_loop(iterable, span)?;
        let acc_ref = self.local_ref(acc, span);
        let elem_ref = self.local_ref(lp.elem, span);
        let add = self.alloc(
            HirExprKind::BinOp { op: BinOp::Add, l: acc_ref, r: elem_ref },
            SemTy::Dyn,
            span,
        );
        self.push_stmt(HirStmt::Assign { target: acc, value: add });
        self.end_iter_loop(lp);
        Ok(self.local_ref(acc, span))
    }

    /// `min`/`max` over a single iterable, or over 2+ positional args (wrapped in a
    /// synthetic list). Compares with the tagged baseline (`rt_obj_cmp`), so heap
    /// elements order by value, not pointer (PITFALLS B13). Empty input yields
    /// `None` (CPython raises `ValueError`; that path is out of scope).
    fn lower_minmax(&mut self, args: &[Expr], span: Span, is_min: bool) -> Result<Idx<HirExpr>> {
        if args.is_empty() {
            return Err(parse_error("min()/max() require at least one argument", span));
        }
        // 1 arg → iterate it; 2+ args → iterate a synthetic list of the args.
        let iterable = if args.len() == 1 {
            self.lower_expr(&args[0])?
        } else {
            let elems = self.lower_expr_list(args)?;
            self.alloc(HirExprKind::ListLit { elems }, SemTy::Dyn, span)
        };

        let acc = self.fresh_local(SemTy::Dyn);
        let have = self.fresh_local(SemTy::Bool);
        let init_have = self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign { target: have, value: init_have });
        let init_acc = self.alloc(HirExprKind::NoneLit, SemTy::NoneTy, span);
        self.push_stmt(HirStmt::Assign { target: acc, value: init_acc });

        let lp = self.begin_iter_loop(iterable, span)?;
        // if not have: first element; else compare and maybe replace.
        let have_ref = self.local_ref(have, span);
        let first_b = self.new_block();
        let cmp_b = self.new_block();
        let cont = self.new_block();
        self.seal(HirTerminator::Branch { cond: have_ref, then: cmp_b, else_: first_b });

        // first_b: acc = elem; have = True
        self.switch(first_b);
        let e1 = self.local_ref(lp.elem, span);
        self.push_stmt(HirStmt::Assign { target: acc, value: e1 });
        let t = self.alloc(HirExprKind::BoolLit(true), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign { target: have, value: t });
        self.seal(HirTerminator::Jump(cont));

        // cmp_b: if elem </> acc: acc = elem
        self.switch(cmp_b);
        let e2 = self.local_ref(lp.elem, span);
        let acc_ref = self.local_ref(acc, span);
        let op = if is_min { CmpOp::Lt } else { CmpOp::Gt };
        let cmp = self.alloc(HirExprKind::Compare { op, l: e2, r: acc_ref }, SemTy::Bool, span);
        let upd = self.new_block();
        self.seal(HirTerminator::Branch { cond: cmp, then: upd, else_: cont });
        self.switch(upd);
        let e3 = self.local_ref(lp.elem, span);
        self.push_stmt(HirStmt::Assign { target: acc, value: e3 });
        self.seal(HirTerminator::Jump(cont));

        self.switch(cont);
        self.end_iter_loop(lp);
        Ok(self.local_ref(acc, span))
    }

    /// `set()` → empty set; `set(iterable)` → fill an empty set from the iterable.
    fn lower_set_call(&mut self, args: &[Expr], span: Span) -> Result<Idx<HirExpr>> {
        if args.is_empty() {
            return Ok(self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span));
        }
        if args.len() != 1 {
            return Err(parse_error("set() takes at most 1 argument", span));
        }
        let result = self.fresh_local(SemTy::set_of(SemTy::Dyn));
        let empty = self.alloc(HirExprKind::SetLit { elems: vec![] }, SemTy::Dyn, span);
        self.push_stmt(HirStmt::Assign { target: result, value: empty });
        let iterable = self.lower_expr(&args[0])?;
        let lp = self.begin_iter_loop(iterable, span)?;
        let elem_ref = self.local_ref(lp.elem, span);
        self.push_stmt(HirStmt::ContainerPush { container: result, value: elem_ref });
        self.end_iter_loop(lp);
        Ok(self.local_ref(result, span))
    }

    /// Emit the innermost comprehension element action (push / insert).
    fn emit_comp_elem(&mut self, kind: &CompKind, span: Span) -> Result<()> {
        match kind {
            CompKind::List { result, elt } | CompKind::Set { result, elt } => {
                let v = self.lower_expr(elt)?;
                self.push_stmt(HirStmt::ContainerPush { container: *result, value: v });
            }
            CompKind::Dict { result, key, val } => {
                let k = self.lower_expr(key)?;
                let v = self.lower_expr(val)?;
                self.push_stmt(HirStmt::ContainerInsert { container: *result, key: k, value: v });
            }
        }
        let _ = span;
        Ok(())
    }

    /// Allocate a fresh synthetic local (unnamed; never referenced by a source
    /// name) for desugared result/operand slots.
    fn fresh_local(&mut self, ty: SemTy) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal { name, ty, raw_int_ok: false, pin_tagged: false });
        id
    }

    /// A fresh synthetic local pinned to the `Tagged` representation — for the slot
    /// that receives an `iter_next` result (null on exhaustion, so it must never be
    /// inferred to an unboxed `Raw(F64)`/`Raw(I8)` that would deref the null).
    fn fresh_local_tagged(&mut self) -> LocalId {
        let name = self.interner.intern("");
        let id = LocalId::new(self.locals.len() as u32);
        self.locals.push(HirLocal {
            name,
            ty: SemTy::Dyn,
            raw_int_ok: false,
            pin_tagged: true,
        });
        id
    }

    fn local_ref(&mut self, lid: LocalId, span: Span) -> Idx<HirExpr> {
        let ty = self.locals[lid.index()].ty.clone();
        self.alloc(HirExprKind::Local(lid), ty, span)
    }

    fn lower_unary(&mut self, u: &ExprUnaryOp, span: Span) -> Result<Idx<HirExpr>> {
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
        let ty = if op == UnaryOp::Not { SemTy::Bool } else { SemTy::Dyn };
        Ok(self.alloc(HirExprKind::Unary { op, operand }, ty, span))
    }

    /// Try to fold a unary `+`/`-` applied to a numeric constant.
    fn try_fold_numeric(
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

    fn lower_binop(&mut self, b: &ExprBinOp, span: Span) -> Result<Idx<HirExpr>> {
        let op = binop_from_ast(&b.op, span)?;
        let l = self.lower_expr(b.left.as_ref())?;
        let r = self.lower_expr(b.right.as_ref())?;
        Ok(self.alloc(HirExprKind::BinOp { op, l, r }, SemTy::Dyn, span))
    }

    fn map_cmp(&self, op: &PyCmpOp, span: Span) -> Result<CmpOp> {
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

    fn lower_compare(&mut self, c: &ExprCompare, span: Span) -> Result<Idx<HirExpr>> {
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
                        HirExprKind::Unary { op: UnaryOp::Not, operand: contains },
                        SemTy::Bool,
                        span,
                    ));
                }
                return Ok(contains);
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
        self.push_stmt(HirStmt::Assign { target: prev, value: lv });

        for (i, comp) in c.comparators.iter().enumerate() {
            let op = self.map_cmp(&c.ops[i], span)?;
            let cv = self.lower_expr(comp)?;
            let cur = self.fresh_local(SemTy::Dyn);
            self.push_stmt(HirStmt::Assign { target: cur, value: cv });
            let lref = self.local_ref(prev, span);
            let rref = self.local_ref(cur, span);
            let cmp = self.alloc(HirExprKind::Compare { op, l: lref, r: rref }, SemTy::Bool, span);
            let next = self.new_block();
            self.seal(HirTerminator::Branch { cond: cmp, then: next, else_: false_b });
            self.switch(next);
            prev = cur;
        }
        self.seal(HirTerminator::Jump(true_b));

        self.switch(true_b);
        let t = self.alloc(HirExprKind::BoolLit(true), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign { target: res, value: t });
        self.seal(HirTerminator::Jump(join));

        self.switch(false_b);
        let fb = self.alloc(HirExprKind::BoolLit(false), SemTy::Bool, span);
        self.push_stmt(HirStmt::Assign { target: res, value: fb });
        self.seal(HirTerminator::Jump(join));

        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    /// Short-circuit `and`/`or` over `values` (≥2), into branch CFG + result local.
    fn lower_boolop(&mut self, b: &ExprBoolOp) -> Result<Idx<HirExpr>> {
        let span = to_span(b.range());
        let res = self.fresh_local(SemTy::Dyn);
        let join = self.new_block();
        let n = b.values.len();
        for (i, val) in b.values.iter().enumerate() {
            let v = self.lower_expr(val)?;
            self.push_stmt(HirStmt::Assign { target: res, value: v });
            if i + 1 < n {
                let next = self.new_block();
                let cond = self.local_ref(res, span);
                match b.op {
                    // `and`: keep going while truthy; short-circuit (res = falsy) to join.
                    PyBoolOp::And => {
                        self.seal(HirTerminator::Branch { cond, then: next, else_: join })
                    }
                    // `or`: short-circuit (res = truthy) to join; else keep going.
                    PyBoolOp::Or => {
                        self.seal(HirTerminator::Branch { cond, then: join, else_: next })
                    }
                }
                self.switch(next);
            } else {
                self.seal(HirTerminator::Jump(join));
            }
        }
        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    fn lower_ifexp(&mut self, e: &ExprIfExp) -> Result<Idx<HirExpr>> {
        let span = to_span(e.range());
        let res = self.fresh_local(SemTy::Dyn);
        let cond = self.lower_expr(e.test.as_ref())?;
        let then_b = self.new_block();
        let else_b = self.new_block();
        let join = self.new_block();
        self.seal(HirTerminator::Branch { cond, then: then_b, else_: else_b });

        self.switch(then_b);
        let bv = self.lower_expr(e.body.as_ref())?;
        self.push_stmt(HirStmt::Assign { target: res, value: bv });
        self.seal(HirTerminator::Jump(join));

        self.switch(else_b);
        let ev = self.lower_expr(e.orelse.as_ref())?;
        self.push_stmt(HirStmt::Assign { target: res, value: ev });
        self.seal(HirTerminator::Jump(join));

        self.switch(join);
        Ok(self.local_ref(res, span))
    }

    /// A call used as a value (builtins now; user functions in 2d). `print` is a
    /// statement, not a value-call, so reject it here.
    fn lower_call_expr(&mut self, c: &ExprCall, span: Span) -> Result<Idx<HirExpr>> {
        if let Expr::Name(n) = c.func.as_ref() {
            if n.id.as_str() == "print" {
                return Err(parse_error("print() is only supported as a statement", span));
            }
        }
        if !c.keywords.is_empty() {
            return Err(parse_error("keyword arguments are not supported in calls", span));
        }
        // `recv.method(args)` → a bounded container method call (Phase 4D). A bare
        // attribute outside call position, or a non-container method name, stays
        // rejected (Phase 5).
        if let Expr::Attribute(attr) = c.func.as_ref() {
            let method = ContainerMethod::from_name(attr.attr.as_str()).ok_or_else(|| {
                parse_error(format!("unsupported method `.{}()`", attr.attr.as_str()), span)
            })?;
            let recv = self.lower_expr(attr.value.as_ref())?;
            let args = self.lower_expr_list(&c.args)?;
            return Ok(self.alloc(HirExprKind::MethodCall { recv, method, args }, SemTy::Dyn, span));
        }
        // Builtins that desugar to reduce / iterator loops are recognized by name
        // (like `print`/`range`; shadowing these names is not supported).
        if let Expr::Name(n) = c.func.as_ref() {
            match n.id.as_str() {
                "sum" => return self.lower_sum(&c.args, span),
                "min" => return self.lower_minmax(&c.args, span, true),
                "max" => return self.lower_minmax(&c.args, span, false),
                "set" => return self.lower_set_call(&c.args, span),
                _ => {}
            }
        }
        let callee = self.lower_expr(c.func.as_ref())?;
        let mut args = Vec::with_capacity(c.args.len());
        for a in &c.args {
            args.push(self.lower_expr(a)?);
        }
        Ok(self.alloc(HirExprKind::Call { callee, args }, SemTy::Dyn, span))
    }

    fn lower_constant(&mut self, c: &Constant, span: Span) -> Result<Idx<HirExpr>> {
        let (kind, ty) = match c {
            Constant::Str(s) => (HirExprKind::StrLit(self.intern(s)), SemTy::Str),
            Constant::Int(big) => (self.int_literal(&big.to_string(), false), SemTy::Int),
            Constant::Float(f) => (HirExprKind::FloatLit(*f), SemTy::Float),
            Constant::Bool(b) => (HirExprKind::BoolLit(*b), SemTy::Bool),
            Constant::None => (HirExprKind::NoneLit, SemTy::NoneTy),
            Constant::Bytes(b) => {
                // The bytes are interned through the string table (codegen reads
                // them back as raw bytes). Non-UTF-8 byte literals are out of scope.
                let s = std::str::from_utf8(b).map_err(|_| {
                    parse_error("non-UTF-8 bytes literals are out of scope", span)
                })?;
                (HirExprKind::BytesLit(self.intern(s)), SemTy::Bytes)
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
    fn int_literal(&mut self, decimal: &str, negative: bool) -> HirExprKind {
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

/// A `range()` bound argument: the literal `0` start of `range(stop)`, or an
/// arbitrary expression.
enum RangeArg<'a> {
    Zero,
    Expr(&'a Expr),
}

/// True iff `iter` is a direct `range(...)` call — selects the Phase-3 fast path.
fn is_range_call(iter: &Expr) -> bool {
    matches!(iter, Expr::Call(c)
        if matches!(c.func.as_ref(), Expr::Name(n) if n.id.as_str() == "range"))
}

/// The element expressions of a tuple/list target or literal-sequence value, used
/// for unpacking (`a, b = …`). `None` for any other expression.
fn seq_target_elts(e: &Expr) -> Option<&[Expr]> {
    match e {
        Expr::Tuple(t) => Some(&t.elts),
        Expr::List(l) => Some(&l.elts),
        _ => None,
    }
}

/// Reject starred unpacking targets (`a, *rest = …`) — deferred to Phase 6.
fn reject_starred(targets: &[Expr], span: Span) -> Result<()> {
    if targets.iter().any(|t| matches!(t, Expr::Starred(_))) {
        return Err(parse_error("starred unpacking targets are out of scope", span));
    }
    Ok(())
}

/// Parse `range(...)` from a `for` iterable into `(start, stop, step)`. `step`
/// must be an integer literal (the loop direction is decided at compile time).
fn parse_range(iter: &Expr, span: Span) -> Result<(RangeArg<'_>, RangeArg<'_>, i64)> {
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
        2 => Ok((RangeArg::Expr(&call.args[0]), RangeArg::Expr(&call.args[1]), 1)),
        3 => {
            let step = literal_int(&call.args[2])
                .ok_or_else(|| parse_error("range() step must be an integer literal", span))?;
            Ok((RangeArg::Expr(&call.args[0]), RangeArg::Expr(&call.args[1]), step))
        }
        _ => Err(parse_error("range() takes 1 to 3 arguments", span)),
    }
}

/// True iff `range(start, stop, step)` is a proof-gated `Raw(I64)`-eligible loop
/// (Phase 3c): every bound is an integer literal whose magnitude is well within
/// the conservative narrowing bound, so the cursor cannot overflow i64 or
/// promote to a heap `BigInt`. Conservative and sound — any non-literal bound
/// (or one out of range) makes the whole loop ineligible (stays tagged).
fn range_is_raw_int_eligible(start: &RangeArg, stop: &RangeArg, step: i64) -> bool {
    let bound = pyaot_types::RAW_I64_NARROW_BOUND;
    let in_bound = |v: i64| v >= -bound && v <= bound;
    let lit = |a: &RangeArg| match a {
        RangeArg::Zero => Some(0i64),
        RangeArg::Expr(e) => literal_int(e),
    };
    match (lit(start), lit(stop)) {
        (Some(lo), Some(hi)) => in_bound(lo) && in_bound(hi) && in_bound(step),
        _ => false,
    }
}

/// Extract an `i64` from an integer-literal expression (possibly unary-signed).
fn literal_int(e: &Expr) -> Option<i64> {
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

fn binop_from_ast(op: &PyOperator, span: Span) -> Result<BinOp> {
    Ok(match op {
        PyOperator::Add => BinOp::Add,
        PyOperator::Sub => BinOp::Sub,
        PyOperator::Mult => BinOp::Mul,
        PyOperator::Div => BinOp::Div,
        PyOperator::FloorDiv => BinOp::FloorDiv,
        PyOperator::Mod => BinOp::Mod,
        PyOperator::Pow => BinOp::Pow,
        PyOperator::LShift => BinOp::Shl,
        PyOperator::RShift => BinOp::Shr,
        PyOperator::BitOr => BinOp::BitOr,
        PyOperator::BitXor => BinOp::BitXor,
        PyOperator::BitAnd => BinOp::BitAnd,
        PyOperator::MatMult => {
            return Err(parse_error("matrix multiply (@) is out of scope", span))
        }
    })
}

/// Map a type annotation to a `SemTy` (primitives and built-in containers drive
/// `Repr`; everything else is `Dyn`). A bare container name (`list`) defaults its
/// element types to `Dyn`; a subscripted one (`list[int]`, `dict[str, int]`,
/// `tuple[int, ...]`) carries them — this is what lets the empty-literal bootstrap
/// seed `x: list[int] = []` (PITFALLS B4).
fn annotation_to_semty(ann: &Expr) -> SemTy {
    match ann {
        Expr::Name(n) => match n.id.as_str() {
            "int" => SemTy::Int,
            "float" => SemTy::Float,
            "bool" => SemTy::Bool,
            "str" => SemTy::Str,
            "bytes" => SemTy::Bytes,
            "None" | "NoneType" => SemTy::NoneTy,
            "list" | "List" => SemTy::list_of(SemTy::Dyn),
            "dict" | "Dict" => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
            "set" | "Set" | "frozenset" => SemTy::set_of(SemTy::Dyn),
            "tuple" | "Tuple" => SemTy::tuple_var_of(SemTy::Dyn),
            _ => SemTy::Dyn,
        },
        Expr::Subscript(s) => annotation_subscript(s.value.as_ref(), s.slice.as_ref()),
        Expr::Constant(c) if matches!(c.value, Constant::None) => SemTy::NoneTy,
        _ => SemTy::Dyn,
    }
}

/// Map a subscripted generic annotation (`list[int]`, `dict[K, V]`, …) to a
/// `SemTy`. Unknown bases fall back to `Dyn`.
fn annotation_subscript(base: &Expr, slice: &Expr) -> SemTy {
    let Expr::Name(n) = base else { return SemTy::Dyn };
    match n.id.as_str() {
        "list" | "List" => SemTy::list_of(annotation_to_semty(slice)),
        "set" | "Set" | "frozenset" => SemTy::set_of(annotation_to_semty(slice)),
        "dict" | "Dict" => match slice {
            Expr::Tuple(t) if t.elts.len() == 2 => {
                SemTy::dict_of(annotation_to_semty(&t.elts[0]), annotation_to_semty(&t.elts[1]))
            }
            _ => SemTy::dict_of(SemTy::Dyn, SemTy::Dyn),
        },
        "tuple" | "Tuple" => match slice {
            // `tuple[T, ...]` is the homogeneous variable-length tuple.
            Expr::Tuple(t) if t.elts.len() == 2 && is_ellipsis(&t.elts[1]) => {
                SemTy::tuple_var_of(annotation_to_semty(&t.elts[0]))
            }
            Expr::Tuple(t) => SemTy::tuple_of(t.elts.iter().map(annotation_to_semty).collect()),
            single => SemTy::tuple_of(vec![annotation_to_semty(single)]),
        },
        "Optional" => SemTy::optional(annotation_to_semty(slice)),
        _ => SemTy::Dyn,
    }
}

/// True iff `e` is the `...` (Ellipsis) literal — the `tuple[T, ...]` marker.
fn is_ellipsis(e: &Expr) -> bool {
    matches!(e, Expr::Constant(c) if matches!(c.value, Constant::Ellipsis))
}

/// Lower a top-level `def` into an [`HirFunction`]. Parameters and return type
/// take their annotations (driving their `Repr`); unannotated → `Dyn`.
fn lower_def(
    interner: &mut StringInterner,
    def: &rustpython_parser::ast::StmtFunctionDef,
) -> Result<HirFunction> {
    let span = to_span(def.range());
    if !def.decorator_list.is_empty() {
        return Err(parse_error("decorators are out of scope for Phase 2", span));
    }
    let args = def.args.as_ref();
    if args.vararg.is_some() || args.kwarg.is_some() || !args.kwonlyargs.is_empty() {
        return Err(parse_error(
            "*args / **kwargs / keyword-only parameters are out of scope",
            span,
        ));
    }
    let ret_ty = match &def.returns {
        Some(e) => annotation_to_semty(e.as_ref()),
        None => SemTy::Dyn,
    };
    let name = interner.intern(def.name.as_str());
    let mut fl = FnLowerer::new(interner, name, ret_ty);
    for awd in args.posonlyargs.iter().chain(args.args.iter()) {
        if awd.default.is_some() {
            return Err(parse_error("default arguments are out of scope", span));
        }
        let pty = match &awd.def.annotation {
            Some(a) => annotation_to_semty(a.as_ref()),
            None => SemTy::Dyn,
        };
        let pname = fl.intern(awd.def.arg.as_str());
        fl.add_param(pname, pty);
    }
    fl.lower_body(&def.body)?;
    Ok(fl.finish(HirTerminator::Return(None)))
}

/// If `expr` is a direct `print(...)` call, return it.
fn as_print_call(expr: &Expr) -> Option<&rustpython_parser::ast::ExprCall> {
    if let Expr::Call(call) = expr {
        if let Expr::Name(n) = call.func.as_ref() {
            if n.id.as_str() == "print" {
                return Some(call);
            }
        }
    }
    None
}

fn to_span(range: TextRange) -> Span {
    Span::new(range.start().to_u32(), range.end().to_u32())
}

fn parse_error(msg: impl Into<String>, span: Span) -> CompilerError {
    CompilerError::parse_error(msg.into(), span)
}
