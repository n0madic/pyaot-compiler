//! Tree → CFG conversion for HIR function bodies.
//!
//! Phase 1 §1.1 of `ARCHITECTURE_REFACTOR.md` requires every `hir::Function` to
//! carry an explicit control-flow graph (`blocks` + `entry_block`). This
//! module consumes the tree form (`Function.body` + nested-body `StmtKind`
//! variants) and emits a parallel CFG that consumers read after the §1.11
//! S1.17b-c/d/e migrations.
//!
//! ## Rich CFG shape (S1.17b-b enhancement, 2026-04-19)
//!
//! Post-S1.17b-a schema additions (`ExprKind::IterHasNext`,
//! `StmtKind::IterAdvance`, `ExprKind::MatchPattern`, `Function::try_scopes`,
//! `ExceptHandler::entry_block`) the bridge now allocates the new arena
//! entries directly:
//!
//! * `ForBind` — the header block terminates with
//!   `Branch(IterHasNext(iter), body_entry, else_or_exit)`. `body_entry`
//!   begins with a new `StmtKind::IterAdvance { iter, target }` to bind the
//!   next element, then runs the original body statements.
//! * `Match` — each case block is entered via
//!   `Branch(MatchPattern(subject, case.pattern), body, next_case)`. Pattern
//!   capture bindings are emitted as ordinary `StmtKind::Bind` statements at
//!   the case-body head (Stage 2 emits them via legacy lowering still;
//!   S1.17b-c moves this in-bridge).
//! * `Try` — handler `entry_block`s are populated and registered in the
//!   function's `try_scopes` side map.
//!
//! Because these additions require allocating new arena entries, the bridge
//! now takes `&mut Module` (not `&Arena<Stmt>`). The 8 call sites in the
//! frontend + generator desugaring were migrated accordingly.

use indexmap::IndexMap;
use pyaot_utils::{HirBlockId, Span};

use crate::{
    BindingTarget, ExceptHandler, Expr, ExprId, ExprKind, HirBlock, HirTerminator, Module, Stmt,
    StmtId, StmtKind, TryScope,
};

/// Reusable HIR CFG builder used by the legacy tree bridge and by direct
/// emitters that want to construct `Function::{blocks, entry_block, try_scopes}`
/// without first allocating a top-level `Vec<StmtId>`.
///
/// The API deliberately mirrors the bridge's internal structure so the
/// remaining frontend/generator tree constructors can migrate one site at a
/// time. The builder owns the block map and try-scope side table; callers are
/// responsible for creating an entry block, selecting insertion points via
/// `enter`, pushing straight-line statements into the current block, and
/// terminating open blocks explicitly.
pub struct CfgBuilder {
    blocks: IndexMap<HirBlockId, HirBlock>,
    current: HirBlockId,
    /// `true` once the current block has received a real terminator and
    /// subsequent statements in the enclosing stmt-list are dead code. Reset
    /// by `enter`.
    current_terminated: bool,
    next_id: u32,
    loop_stack: Vec<LoopCtx>,
    /// Try-scopes discovered while lowering. The caller merges these into
    /// `Function::try_scopes` after construction (§1.11 Q2).
    try_scopes: Vec<TryScope>,
    /// Current nesting depth inside for/while loop bodies. Written to each
    /// new block's `loop_depth`.
    loop_depth: u8,
    /// Current nesting depth inside exception handler / finally regions.
    /// Written to each new block's `handler_depth`.
    handler_depth: u8,
}

#[derive(Clone, Copy)]
struct LoopCtx {
    continue_bb: HirBlockId,
    break_bb: HirBlockId,
}

impl CfgBuilder {
    pub fn new() -> Self {
        Self {
            blocks: IndexMap::new(),
            current: HirBlockId::new(0), // placeholder until enter() is called
            current_terminated: false,
            next_id: 0,
            loop_stack: Vec::new(),
            try_scopes: Vec::new(),
            loop_depth: 0,
            handler_depth: 0,
        }
    }

    /// Allocate a fresh `ExprKind::IterHasNext(iter)` bool predicate.
    pub fn alloc_iter_has_next(&self, module: &mut Module, iter: ExprId, span: Span) -> ExprId {
        module.exprs.alloc(Expr {
            kind: ExprKind::IterHasNext(iter),
            ty: Some(pyaot_types::Type::Bool),
            span,
        })
    }

    /// Allocate a fresh `StmtKind::IterAdvance { iter, target }` stmt.
    pub fn alloc_iter_advance(
        &self,
        module: &mut Module,
        iter: ExprId,
        target: BindingTarget,
        span: Span,
    ) -> StmtId {
        module.stmts.alloc(Stmt {
            kind: StmtKind::IterAdvance { iter, target },
            span,
        })
    }

    /// Allocate a fresh `StmtKind::IterSetup { iter }` stmt. Emitted in
    /// the for-loop's pre-block (before `Jump(header)`) so the iterator
    /// is created exactly once per loop, not per header-iteration.
    pub fn alloc_iter_setup(&self, module: &mut Module, iter: ExprId, span: Span) -> StmtId {
        module.stmts.alloc(Stmt {
            kind: StmtKind::IterSetup { iter },
            span,
        })
    }

    /// Allocate a fresh `ExprKind::MatchPattern { subject, pattern }` bool
    /// predicate.
    pub fn alloc_match_pattern(
        &self,
        module: &mut Module,
        subject: ExprId,
        pattern: crate::Pattern,
        span: Span,
    ) -> ExprId {
        module.exprs.alloc(Expr {
            kind: ExprKind::MatchPattern {
                subject,
                pattern: Box::new(pattern),
            },
            ty: Some(pyaot_types::Type::Bool),
            span,
        })
    }

    pub fn new_block(&mut self) -> HirBlockId {
        let id = HirBlockId::new(self.next_id);
        self.next_id += 1;
        self.blocks.insert(
            id,
            HirBlock {
                id,
                stmts: Vec::new(),
                terminator: HirTerminator::Unreachable,
                loop_depth: self.loop_depth,
                handler_depth: self.handler_depth,
            },
        );
        id
    }

    /// Make `block` the current insertion point and mark it open (un-terminated).
    pub fn enter(&mut self, block: HirBlockId) {
        self.current = block;
        self.current_terminated = false;
    }

    pub fn current_block(&self) -> HirBlockId {
        self.current
    }

    pub fn is_current_terminated(&self) -> bool {
        self.current_terminated
    }

    pub fn push_stmt(&mut self, stmt_id: StmtId) {
        let block = self.current;
        self.blocks
            .get_mut(&block)
            .expect("current block must exist")
            .stmts
            .push(stmt_id);
    }

    pub fn set_terminator(&mut self, block: HirBlockId, term: HirTerminator) {
        self.blocks
            .get_mut(&block)
            .expect("terminator target block must exist")
            .terminator = term;
        if block == self.current {
            self.current_terminated = true;
        }
    }

    /// If the current block has not yet been terminated, close it with `term`.
    pub fn terminate_if_open(&mut self, term: HirTerminator) {
        if !self.current_terminated {
            let block = self.current;
            self.set_terminator(block, term);
        }
    }

    pub fn block(&self, block: HirBlockId) -> Option<&HirBlock> {
        self.blocks.get(&block)
    }

    pub fn block_mut(&mut self, block: HirBlockId) -> Option<&mut HirBlock> {
        self.blocks.get_mut(&block)
    }

    pub fn push_loop(&mut self, continue_bb: HirBlockId, break_bb: HirBlockId) {
        self.loop_stack.push(LoopCtx {
            continue_bb,
            break_bb,
        });
        self.loop_depth += 1;
    }

    pub fn pop_loop(&mut self) {
        self.loop_stack.pop();
        self.loop_depth = self.loop_depth.saturating_sub(1);
    }

    pub fn continue_target(&self) -> Option<HirBlockId> {
        self.loop_stack.last().map(|ctx| ctx.continue_bb)
    }

    pub fn break_target(&self) -> Option<HirBlockId> {
        self.loop_stack.last().map(|ctx| ctx.break_bb)
    }

    pub fn loop_depth(&self) -> u8 {
        self.loop_depth
    }

    pub fn push_handler(&mut self) {
        self.handler_depth += 1;
    }

    pub fn pop_handler(&mut self) {
        self.handler_depth = self.handler_depth.saturating_sub(1);
    }

    pub fn handler_depth(&self) -> u8 {
        self.handler_depth
    }

    pub fn register_try_scope(&mut self, scope: TryScope) {
        self.try_scopes.push(scope);
    }

    pub fn lower_stmts(&mut self, stmts_list: &[StmtId], module: &mut Module) {
        for &stmt_id in stmts_list {
            if self.current_terminated {
                break;
            }
            self.lower_stmt(stmt_id, module);
        }
    }

    pub fn lower_stmt(&mut self, stmt_id: StmtId, module: &mut Module) {
        // Clone the kind so we can release the shared borrow on `module.stmts`
        // before recursing into `lower_stmts` with a mutable borrow.
        let stmt_kind = module.stmts[stmt_id].kind.clone();
        let stmt_span = module.stmts[stmt_id].span;
        match stmt_kind {
            StmtKind::Expr(_)
            | StmtKind::Bind { .. }
            | StmtKind::Pass
            | StmtKind::Assert { .. }
            | StmtKind::IndexDelete { .. }
            | StmtKind::IterAdvance { .. }
            | StmtKind::IterSetup { .. } => {
                self.push_stmt(stmt_id);
            }

            StmtKind::Return(value) => {
                let block = self.current;
                self.set_terminator(block, HirTerminator::Return(value));
            }

            StmtKind::Raise { exc, cause } => {
                let block = self.current;
                let term = match exc {
                    Some(exc_id) => HirTerminator::Raise { exc: exc_id, cause },
                    // Bare `raise` — re-raise the active exception.
                    None => HirTerminator::Reraise,
                };
                self.set_terminator(block, term);
            }

            StmtKind::Break => {
                let block = self.current;
                let term = match self.loop_stack.last() {
                    Some(ctx) => HirTerminator::Jump(ctx.break_bb),
                    None => HirTerminator::Unreachable,
                };
                self.set_terminator(block, term);
            }

            StmtKind::Continue => {
                let block = self.current;
                let term = match self.loop_stack.last() {
                    Some(ctx) => HirTerminator::Jump(ctx.continue_bb),
                    None => HirTerminator::Unreachable,
                };
                self.set_terminator(block, term);
            }

            StmtKind::If {
                cond,
                then_block,
                else_block,
            } => {
                let then_bb = self.new_block();
                let else_bb = self.new_block();
                let merge_bb = self.new_block();

                let branch_block = self.current;
                self.set_terminator(
                    branch_block,
                    HirTerminator::Branch {
                        cond,
                        then_bb,
                        else_bb,
                    },
                );

                self.enter(then_bb);
                self.lower_stmts(&then_block, module);
                self.terminate_if_open(HirTerminator::Jump(merge_bb));

                self.enter(else_bb);
                self.lower_stmts(&else_block, module);
                self.terminate_if_open(HirTerminator::Jump(merge_bb));

                self.enter(merge_bb);
            }

            StmtKind::While {
                cond,
                body,
                else_block,
            } => {
                let header_bb = self.new_block();
                let body_bb = self.new_block();
                let exit_bb = self.new_block();
                let else_bb = if else_block.is_empty() {
                    exit_bb
                } else {
                    self.new_block()
                };

                let pre_block = self.current;
                self.set_terminator(pre_block, HirTerminator::Jump(header_bb));

                self.enter(header_bb);
                self.set_terminator(
                    header_bb,
                    HirTerminator::Branch {
                        cond,
                        then_bb: body_bb,
                        else_bb,
                    },
                );

                self.push_loop(header_bb, exit_bb);
                self.enter(body_bb);
                // Rewrite the block's loop_depth — it was allocated at the
                // outer depth, but its stmts run inside the loop.
                self.blocks.get_mut(&body_bb).unwrap().loop_depth = self.loop_depth;
                self.lower_stmts(&body, module);
                self.terminate_if_open(HirTerminator::Jump(header_bb));
                self.pop_loop();

                if !else_block.is_empty() {
                    self.enter(else_bb);
                    self.lower_stmts(&else_block, module);
                    self.terminate_if_open(HirTerminator::Jump(exit_bb));
                }

                self.enter(exit_bb);
            }

            // §1.11 Q1 Scheme A — emit IterHasNext + IterAdvance.
            StmtKind::ForBind {
                target,
                iter,
                body,
                else_block,
            } => {
                let header_bb = self.new_block();
                let body_bb = self.new_block();
                let exit_bb = self.new_block();
                let else_bb = if else_block.is_empty() {
                    exit_bb
                } else {
                    self.new_block()
                };

                // Pre-block: push IterSetup stmt BEFORE Jump(header). This
                // evaluates iter_expr exactly once and caches the resulting
                // iterator local in `CodeGenState::iter_cache` during
                // lowering — so subsequent IterHasNext (in header) and
                // IterAdvance (in body) read from the cache without
                // re-creating the iterator each iteration.
                let iter_setup_stmt = self.alloc_iter_setup(module, iter, stmt_span);
                self.push_stmt(iter_setup_stmt);

                let pre_block = self.current;
                self.set_terminator(pre_block, HirTerminator::Jump(header_bb));

                // Header: `Branch(IterHasNext(iter), body, else_or_exit)`.
                let has_next = self.alloc_iter_has_next(module, iter, stmt_span);
                self.enter(header_bb);
                self.set_terminator(
                    header_bb,
                    HirTerminator::Branch {
                        cond: has_next,
                        then_bb: body_bb,
                        else_bb,
                    },
                );

                // Body: prefix with `IterAdvance { iter, target }` then emit
                // the original body statements. Lowering will recognise the
                // IterAdvance and emit the runtime iterator-next protocol.
                self.push_loop(header_bb, exit_bb);
                self.enter(body_bb);
                self.blocks.get_mut(&body_bb).unwrap().loop_depth = self.loop_depth;
                let advance_stmt = self.alloc_iter_advance(module, iter, target.clone(), stmt_span);
                self.push_stmt(advance_stmt);
                self.lower_stmts(&body, module);
                self.terminate_if_open(HirTerminator::Jump(header_bb));
                self.pop_loop();

                if !else_block.is_empty() {
                    self.enter(else_bb);
                    self.lower_stmts(&else_block, module);
                    self.terminate_if_open(HirTerminator::Jump(exit_bb));
                }

                self.enter(exit_bb);
            }

            // §1.11 Q2 — handlers are registered as a `TryScope`. Handler
            // entry blocks have no CFG predecessors; runtime unwinding
            // dispatches into them on a matching raise.
            StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                let body_bb = self.new_block();
                let post_bb = self.new_block();

                let pre_block = self.current;
                self.set_terminator(pre_block, HirTerminator::Jump(body_bb));

                // Track blocks emitted inside body / else / finally so the
                // TryScope can cite them as "guarded by this handler chain".
                let mut try_blocks_set: Vec<HirBlockId> = Vec::new();
                let mut else_blocks_set: Vec<HirBlockId> = Vec::new();
                let mut finally_blocks_set: Vec<HirBlockId> = Vec::new();

                // body_bb was allocated before the snapshot so we include it
                // explicitly — the snapshot only captures blocks created during
                // lower_stmts (nested if/while/for inside the try body).
                let blocks_before_body = self.next_id;
                self.enter(body_bb);
                self.lower_stmts(&body, module);
                try_blocks_set.push(body_bb);
                for id in blocks_before_body..self.next_id {
                    try_blocks_set.push(HirBlockId::new(id));
                }

                let blocks_before_else = self.next_id;
                let after_body = if else_block.is_empty() {
                    post_bb
                } else {
                    let else_bb = self.new_block();
                    self.terminate_if_open(HirTerminator::Jump(else_bb));
                    self.enter(else_bb);
                    self.lower_stmts(&else_block, module);
                    post_bb
                };
                for id in blocks_before_else..self.next_id {
                    else_blocks_set.push(HirBlockId::new(id));
                }

                let blocks_before_finally = self.next_id;
                if !finally_block.is_empty() {
                    let finally_bb = self.new_block();
                    self.terminate_if_open(HirTerminator::Jump(finally_bb));
                    // §1.11 Q2 — `finally` runs in an exception-handler
                    // context (bare raise is allowed). Bump handler_depth.
                    self.push_handler();
                    self.enter(finally_bb);
                    self.blocks.get_mut(&finally_bb).unwrap().handler_depth = self.handler_depth;
                    self.lower_stmts(&finally_block, module);
                    self.terminate_if_open(HirTerminator::Jump(post_bb));
                    self.pop_handler();
                } else {
                    self.terminate_if_open(HirTerminator::Jump(after_body));
                }
                for id in blocks_before_finally..self.next_id {
                    finally_blocks_set.push(HirBlockId::new(id));
                }

                // Emit a CFG block for each handler, populating its
                // `entry_block`. Each handler's body lives inside the block
                // (bindings + user-written code); terminator is Jump(post_bb).
                // §1.11 Q2 — handler bodies run in an exception-handler
                // context so bare `raise` is allowed. Bump handler_depth.
                let mut handlers_out: Vec<ExceptHandler> = Vec::with_capacity(handlers.len());
                for handler in handlers {
                    let handler_bb = self.new_block();
                    self.push_handler();
                    self.enter(handler_bb);
                    self.blocks.get_mut(&handler_bb).unwrap().handler_depth = self.handler_depth;
                    self.lower_stmts(&handler.body, module);
                    self.terminate_if_open(HirTerminator::Jump(post_bb));
                    self.pop_handler();
                    handlers_out.push(ExceptHandler {
                        entry_block: handler_bb,
                        ..handler
                    });
                }

                // Register the scope. Consumer migration (S1.17b-c/d) reads
                // `Function::try_scopes` to find handler chains for each
                // guarded body block.
                self.register_try_scope(TryScope {
                    try_blocks: try_blocks_set,
                    else_blocks: else_blocks_set,
                    handlers: handlers_out,
                    finally_blocks: finally_blocks_set,
                    span: stmt_span,
                });

                self.enter(post_bb);
            }

            // §1.11 Q3 — match desugars to an if/else ladder of
            // `Branch(MatchPattern(subject, pattern), body, next_case)`.
            StmtKind::Match { subject, cases } => {
                let post_bb = self.new_block();

                if cases.is_empty() {
                    let pre_block = self.current;
                    self.set_terminator(pre_block, HirTerminator::Jump(post_bb));
                } else {
                    // Allocate one block per case-body plus the "no match"
                    // fallthrough block used as the else target of the last
                    // case's predicate branch.
                    let case_bbs: Vec<HirBlockId> =
                        cases.iter().map(|_| self.new_block()).collect();
                    let fallthrough_bb = self.new_block();

                    // Allocate predicate ExprIds upfront so the loop below
                    // can freely call `lower_stmts(.. module)`.
                    let predicates: Vec<ExprId> = cases
                        .iter()
                        .map(|case| {
                            self.alloc_match_pattern(
                                module,
                                subject,
                                case.pattern.clone(),
                                stmt_span,
                            )
                        })
                        .collect();

                    // Test block N: if case has a guard, split into
                    //   test_bb → Branch(pattern, guard_bb, next_test)
                    //   guard_bb → Branch(guard_expr, case_body, next_test)
                    // Otherwise:
                    //   test_bb → Branch(pattern, case_body, next_test)
                    // Guard block allocated only when case has a guard.
                    let test_bbs: Vec<HirBlockId> =
                        cases.iter().map(|_| self.new_block()).collect();
                    let guard_bbs: Vec<Option<HirBlockId>> = cases
                        .iter()
                        .map(|case| {
                            if case.guard.is_some() {
                                Some(self.new_block())
                            } else {
                                None
                            }
                        })
                        .collect();
                    let pre_block = self.current;
                    self.set_terminator(pre_block, HirTerminator::Jump(test_bbs[0]));
                    for (i, ((&test_bb, &case_bb), predicate)) in test_bbs
                        .iter()
                        .zip(case_bbs.iter())
                        .zip(predicates.iter())
                        .enumerate()
                    {
                        let next_test = if i + 1 < test_bbs.len() {
                            test_bbs[i + 1]
                        } else {
                            fallthrough_bb
                        };
                        // If this case has a guard, branch to guard_bb
                        // on pattern match; otherwise straight to case
                        // body.
                        let then_target = guard_bbs[i].unwrap_or(case_bb);
                        self.enter(test_bb);
                        self.set_terminator(
                            test_bb,
                            HirTerminator::Branch {
                                cond: *predicate,
                                then_bb: then_target,
                                else_bb: next_test,
                            },
                        );
                        // If there's a guard, emit the guard check
                        // block: Branch(guard_expr, case_body, next_test).
                        if let (Some(guard_bb), Some(guard_expr_id)) =
                            (guard_bbs[i], cases[i].guard)
                        {
                            self.enter(guard_bb);
                            self.set_terminator(
                                guard_bb,
                                HirTerminator::Branch {
                                    cond: guard_expr_id,
                                    then_bb: case_bb,
                                    else_bb: next_test,
                                },
                            );
                        }
                    }

                    for (case, &bb) in cases.iter().zip(&case_bbs) {
                        self.enter(bb);
                        self.lower_stmts(&case.body, module);
                        self.terminate_if_open(HirTerminator::Jump(post_bb));
                    }

                    self.enter(fallthrough_bb);
                    self.terminate_if_open(HirTerminator::Jump(post_bb));
                }

                self.enter(post_bb);
            }
        }
    }

    pub fn finish(
        self,
        entry: HirBlockId,
    ) -> (IndexMap<HirBlockId, HirBlock>, HirBlockId, Vec<TryScope>) {
        (self.blocks, entry, self.try_scopes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BindingTarget, Expr, ExprKind, Module, Stmt, StmtKind};
    use la_arena::Arena;
    use pyaot_utils::{Span, StringInterner, VarId};

    fn dummy_span() -> Span {
        Span::dummy()
    }

    fn make_module() -> Module {
        let mut interner = StringInterner::new();
        Module::new(interner.intern("test"))
    }

    fn alloc_expr(exprs: &mut Arena<Expr>) -> la_arena::Idx<Expr> {
        exprs.alloc(Expr {
            kind: ExprKind::None,
            ty: None,
            span: dummy_span(),
        })
    }

    fn alloc_stmt(stmts: &mut Arena<Stmt>, kind: StmtKind) -> StmtId {
        stmts.alloc(Stmt {
            kind,
            span: dummy_span(),
        })
    }

    fn bind_stmt(stmts: &mut Arena<Stmt>, exprs: &mut Arena<Expr>) -> StmtId {
        let value = alloc_expr(exprs);
        alloc_stmt(
            stmts,
            StmtKind::Bind {
                target: BindingTarget::Var(VarId::new(0)),
                value,
                type_hint: None,
            },
        )
    }

    fn build_cfg(
        body: &[StmtId],
        module: &mut Module,
    ) -> (IndexMap<HirBlockId, HirBlock>, HirBlockId, Vec<TryScope>) {
        let mut builder = CfgBuilder::new();
        let entry = builder.new_block();
        builder.enter(entry);
        builder.lower_stmts(body, module);
        builder.terminate_if_open(HirTerminator::Return(None));
        builder.finish(entry)
    }

    #[test]
    fn empty_body_returns_single_block_with_return_none() {
        let mut module = make_module();
        let (blocks, entry, _try_scopes) = build_cfg(&[], &mut module);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[&entry];
        assert!(block.stmts.is_empty());
        assert!(matches!(block.terminator, HirTerminator::Return(None)));
    }

    #[test]
    fn straight_line_body_collapses_to_one_block() {
        let mut module = make_module();
        let s1 = bind_stmt(&mut module.stmts, &mut module.exprs);
        let s2 = bind_stmt(&mut module.stmts, &mut module.exprs);
        let s3 = bind_stmt(&mut module.stmts, &mut module.exprs);

        let (blocks, entry, _try_scopes) = build_cfg(&[s1, s2, s3], &mut module);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[&entry];
        assert_eq!(block.stmts, vec![s1, s2, s3]);
        assert!(matches!(block.terminator, HirTerminator::Return(None)));
    }

    #[test]
    fn if_emits_then_else_merge() {
        let mut module = make_module();
        let cond = alloc_expr(&mut module.exprs);
        let then_stmt = bind_stmt(&mut module.stmts, &mut module.exprs);
        let else_stmt = bind_stmt(&mut module.stmts, &mut module.exprs);
        let if_stmt = alloc_stmt(
            &mut module.stmts,
            StmtKind::If {
                cond,
                then_block: vec![then_stmt],
                else_block: vec![else_stmt],
            },
        );

        let (blocks, entry, _try_scopes) = build_cfg(&[if_stmt], &mut module);
        // entry(branch) + then + else + merge = 4 blocks.
        assert_eq!(blocks.len(), 4);
        let branch = &blocks[&entry];
        let (then_bb, else_bb) = match branch.terminator {
            HirTerminator::Branch {
                then_bb, else_bb, ..
            } => (then_bb, else_bb),
            _ => panic!("entry block must end in Branch"),
        };
        // Both branches jump to the same merge block.
        let then_term = &blocks[&then_bb].terminator;
        let else_term = &blocks[&else_bb].terminator;
        let merge_from_then = match then_term {
            HirTerminator::Jump(m) => *m,
            _ => panic!("then block must jump to merge"),
        };
        let merge_from_else = match else_term {
            HirTerminator::Jump(m) => *m,
            _ => panic!("else block must jump to merge"),
        };
        assert_eq!(merge_from_then, merge_from_else);
        assert!(matches!(
            blocks[&merge_from_then].terminator,
            HirTerminator::Return(None)
        ));
    }

    #[test]
    fn while_with_break_continue() {
        let mut module = make_module();
        let cond = alloc_expr(&mut module.exprs);
        let break_stmt = alloc_stmt(&mut module.stmts, StmtKind::Break);
        let continue_stmt = alloc_stmt(&mut module.stmts, StmtKind::Continue);
        let inner_cond = alloc_expr(&mut module.exprs);
        let inner_if = alloc_stmt(
            &mut module.stmts,
            StmtKind::If {
                cond: inner_cond,
                then_block: vec![break_stmt],
                else_block: vec![continue_stmt],
            },
        );
        let while_stmt = alloc_stmt(
            &mut module.stmts,
            StmtKind::While {
                cond,
                body: vec![inner_if],
                else_block: vec![],
            },
        );

        let (blocks, _entry, _try_scopes) = build_cfg(&[while_stmt], &mut module);
        // Verify at least one Jump to a header (continue) and at least one
        // Jump to an exit (break) exist — precise block ids are internal.
        let jumps: Vec<HirBlockId> = blocks
            .values()
            .filter_map(|b| match b.terminator {
                HirTerminator::Jump(id) => Some(id),
                _ => None,
            })
            .collect();
        assert!(!jumps.is_empty(), "while body should produce jumps");
        // Every branch terminator must target a block that exists.
        for block in blocks.values() {
            match block.terminator {
                HirTerminator::Jump(id) => assert!(blocks.contains_key(&id)),
                HirTerminator::Branch {
                    then_bb, else_bb, ..
                } => {
                    assert!(blocks.contains_key(&then_bb));
                    assert!(blocks.contains_key(&else_bb));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn return_shortcircuits_remaining_stmts() {
        let mut module = make_module();
        let pre = bind_stmt(&mut module.stmts, &mut module.exprs);
        let ret = alloc_stmt(&mut module.stmts, StmtKind::Return(None));
        let after = bind_stmt(&mut module.stmts, &mut module.exprs);

        let (blocks, entry, _try_scopes) = build_cfg(&[pre, ret, after], &mut module);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[&entry];
        // `after` must not be emitted — it lives past the Return.
        assert_eq!(block.stmts, vec![pre]);
        assert!(matches!(block.terminator, HirTerminator::Return(None)));
    }

    #[test]
    fn raise_with_expr_becomes_raise_terminator() {
        let mut module = make_module();
        let exc = alloc_expr(&mut module.exprs);
        let raise_stmt = alloc_stmt(
            &mut module.stmts,
            StmtKind::Raise {
                exc: Some(exc),
                cause: None,
            },
        );

        let (blocks, entry, _try_scopes) = build_cfg(&[raise_stmt], &mut module);
        assert!(matches!(
            blocks[&entry].terminator,
            HirTerminator::Raise { cause: None, .. }
        ));
    }
}
