//! HIR CFG construction utilities.
//!
//! `CfgBuilder` lowers a temporary nested `CfgStmt` tree into
//! `Function::{blocks, entry_block, try_scopes}`. The temporary tree never
//! enters the public HIR schema; only flat `StmtId` leaves are allocated in
//! `Module::stmts`.

use indexmap::IndexMap;
use pyaot_types::Type;
use pyaot_utils::{HirBlockId, Span, VarId};

use crate::{
    BindingTarget, ExceptHandler, Expr, ExprId, ExprKind, HirBlock, HirTerminator, Module, Stmt,
    StmtId, StmtKind, TryScope,
};

/// Temporary nested statement tree used only while constructing a CFG.
///
/// Frontend lowering and generator desugaring still need to assemble nested
/// control flow before it is materialised into `Function::{blocks, entry_block,
/// try_scopes}`. Unlike the legacy HIR tree, these nodes never enter
/// `Module::stmts`; only `Stmt(stmt_id)` leaves are real HIR statements.
#[derive(Debug, Clone)]
pub enum CfgStmt {
    Stmt(StmtId),
    If {
        cond: ExprId,
        then_body: Vec<CfgStmt>,
        else_body: Vec<CfgStmt>,
        span: Span,
    },
    While {
        cond: ExprId,
        body: Vec<CfgStmt>,
        else_body: Vec<CfgStmt>,
        span: Span,
    },
    For {
        target: BindingTarget,
        iter: ExprId,
        body: Vec<CfgStmt>,
        else_body: Vec<CfgStmt>,
        span: Span,
    },
    Try {
        body: Vec<CfgStmt>,
        handlers: Vec<CfgExceptHandler>,
        else_body: Vec<CfgStmt>,
        finally_body: Vec<CfgStmt>,
        span: Span,
    },
    Match {
        subject: ExprId,
        cases: Vec<CfgMatchCase>,
        span: Span,
    },
}

impl CfgStmt {
    pub fn stmt(stmt_id: StmtId) -> Self {
        Self::Stmt(stmt_id)
    }
}

#[derive(Debug, Clone)]
pub struct CfgMatchCase {
    pub pattern: crate::Pattern,
    pub guard: Option<ExprId>,
    pub body: Vec<CfgStmt>,
}

#[derive(Debug, Clone)]
pub struct CfgExceptHandler {
    pub ty: Option<Type>,
    pub name: Option<VarId>,
    pub body: Vec<CfgStmt>,
}

/// Reusable HIR CFG builder for emitters that want to construct
/// `Function::{blocks, entry_block, try_scopes}` from temporary nested control
/// flow without allocating tree-shaped HIR statements.
///
/// The API mirrors the structure of CFG construction used by the frontend and
/// generator desugaring. The builder owns the block map and try-scope side
/// table; callers are responsible for creating an entry block, selecting
/// insertion points via `enter`, pushing straight-line statements into the
/// current block, and terminating open blocks explicitly.
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

impl Default for CfgBuilder {
    fn default() -> Self {
        Self::new()
    }
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

    pub fn lower_cfg_stmts(&mut self, stmts_list: &[CfgStmt], module: &mut Module) {
        for stmt in stmts_list {
            if self.current_terminated {
                break;
            }
            self.lower_cfg_stmt(stmt, module);
        }
    }

    fn lower_flat_stmt(&mut self, stmt_id: StmtId, module: &mut Module) {
        let stmt_kind = module.stmts[stmt_id].kind.clone();
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
                match exc {
                    Some(exc_id) => {
                        let block = self.current;
                        self.set_terminator(block, HirTerminator::Raise { exc: exc_id, cause });
                    }
                    // Preserve invalid bare `raise` as a statement so semantic
                    // analysis can report it with the original span.
                    None if self.handler_depth == 0 => self.push_stmt(stmt_id),
                    None => {
                        let block = self.current;
                        self.set_terminator(block, HirTerminator::Reraise);
                    }
                }
            }

            StmtKind::Break => {
                if let Some(ctx) = self.loop_stack.last() {
                    let block = self.current;
                    self.set_terminator(block, HirTerminator::Jump(ctx.break_bb));
                } else {
                    self.push_stmt(stmt_id);
                }
            }

            StmtKind::Continue => {
                if let Some(ctx) = self.loop_stack.last() {
                    let block = self.current;
                    self.set_terminator(block, HirTerminator::Jump(ctx.continue_bb));
                } else {
                    self.push_stmt(stmt_id);
                }
            }
        }
    }

    pub fn lower_cfg_stmt(&mut self, stmt: &CfgStmt, module: &mut Module) {
        match stmt {
            CfgStmt::Stmt(stmt_id) => self.lower_flat_stmt(*stmt_id, module),
            CfgStmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                let then_bb = self.new_block();
                let else_bb = self.new_block();
                let merge_bb = self.new_block();

                let branch_block = self.current;
                self.set_terminator(
                    branch_block,
                    HirTerminator::Branch {
                        cond: *cond,
                        then_bb,
                        else_bb,
                    },
                );

                self.enter(then_bb);
                self.lower_cfg_stmts(then_body, module);
                self.terminate_if_open(HirTerminator::Jump(merge_bb));

                self.enter(else_bb);
                self.lower_cfg_stmts(else_body, module);
                self.terminate_if_open(HirTerminator::Jump(merge_bb));

                self.enter(merge_bb);
            }
            CfgStmt::While {
                cond,
                body,
                else_body,
                ..
            } => {
                let header_bb = self.new_block();
                let body_bb = self.new_block();
                let exit_bb = self.new_block();
                let else_bb = if else_body.is_empty() {
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
                        cond: *cond,
                        then_bb: body_bb,
                        else_bb,
                    },
                );

                self.push_loop(header_bb, exit_bb);
                self.enter(body_bb);
                self.blocks.get_mut(&body_bb).unwrap().loop_depth = self.loop_depth;
                self.lower_cfg_stmts(body, module);
                self.terminate_if_open(HirTerminator::Jump(header_bb));
                self.pop_loop();

                if !else_body.is_empty() {
                    self.enter(else_bb);
                    self.lower_cfg_stmts(else_body, module);
                    self.terminate_if_open(HirTerminator::Jump(exit_bb));
                }

                self.enter(exit_bb);
            }
            CfgStmt::For {
                target,
                iter,
                body,
                else_body,
                span,
            } => {
                let header_bb = self.new_block();
                let body_bb = self.new_block();
                let exit_bb = self.new_block();
                let else_bb = if else_body.is_empty() {
                    exit_bb
                } else {
                    self.new_block()
                };

                let iter_setup_stmt = self.alloc_iter_setup(module, *iter, *span);
                self.push_stmt(iter_setup_stmt);

                let pre_block = self.current;
                self.set_terminator(pre_block, HirTerminator::Jump(header_bb));

                let has_next = self.alloc_iter_has_next(module, *iter, *span);
                self.enter(header_bb);
                self.set_terminator(
                    header_bb,
                    HirTerminator::Branch {
                        cond: has_next,
                        then_bb: body_bb,
                        else_bb,
                    },
                );

                self.push_loop(header_bb, exit_bb);
                self.enter(body_bb);
                self.blocks.get_mut(&body_bb).unwrap().loop_depth = self.loop_depth;
                let advance_stmt = self.alloc_iter_advance(module, *iter, target.clone(), *span);
                self.push_stmt(advance_stmt);
                self.lower_cfg_stmts(body, module);
                self.terminate_if_open(HirTerminator::Jump(header_bb));
                self.pop_loop();

                if !else_body.is_empty() {
                    self.enter(else_bb);
                    self.lower_cfg_stmts(else_body, module);
                    self.terminate_if_open(HirTerminator::Jump(exit_bb));
                }

                self.enter(exit_bb);
            }
            CfgStmt::Try {
                body,
                handlers,
                else_body,
                finally_body,
                span,
            } => {
                let body_bb = self.new_block();
                let post_bb = self.new_block();

                let pre_block = self.current;
                self.set_terminator(pre_block, HirTerminator::Jump(body_bb));

                let mut try_blocks_set: Vec<HirBlockId> = Vec::new();
                let mut else_blocks_set: Vec<HirBlockId> = Vec::new();
                let mut finally_blocks_set: Vec<HirBlockId> = Vec::new();

                let blocks_before_body = self.next_id;
                self.enter(body_bb);
                self.lower_cfg_stmts(body, module);
                try_blocks_set.push(body_bb);
                for id in blocks_before_body..self.next_id {
                    try_blocks_set.push(HirBlockId::new(id));
                }

                let blocks_before_else = self.next_id;
                let after_body = if else_body.is_empty() {
                    post_bb
                } else {
                    let else_bb = self.new_block();
                    self.terminate_if_open(HirTerminator::Jump(else_bb));
                    self.enter(else_bb);
                    self.lower_cfg_stmts(else_body, module);
                    post_bb
                };
                for id in blocks_before_else..self.next_id {
                    else_blocks_set.push(HirBlockId::new(id));
                }

                let blocks_before_finally = self.next_id;
                if !finally_body.is_empty() {
                    let finally_bb = self.new_block();
                    self.terminate_if_open(HirTerminator::Jump(finally_bb));
                    self.push_handler();
                    self.enter(finally_bb);
                    self.blocks.get_mut(&finally_bb).unwrap().handler_depth = self.handler_depth;
                    self.lower_cfg_stmts(finally_body, module);
                    self.terminate_if_open(HirTerminator::Jump(post_bb));
                    self.pop_handler();
                } else {
                    self.terminate_if_open(HirTerminator::Jump(after_body));
                }
                for id in blocks_before_finally..self.next_id {
                    finally_blocks_set.push(HirBlockId::new(id));
                }

                let mut handlers_out: Vec<ExceptHandler> = Vec::with_capacity(handlers.len());
                for handler in handlers {
                    let handler_bb = self.new_block();
                    self.push_handler();
                    self.enter(handler_bb);
                    self.blocks.get_mut(&handler_bb).unwrap().handler_depth = self.handler_depth;
                    self.lower_cfg_stmts(&handler.body, module);
                    self.terminate_if_open(HirTerminator::Jump(post_bb));
                    self.pop_handler();
                    handlers_out.push(ExceptHandler {
                        ty: handler.ty.clone(),
                        name: handler.name,
                        entry_block: handler_bb,
                    });
                }

                self.register_try_scope(TryScope {
                    try_blocks: try_blocks_set,
                    else_blocks: else_blocks_set,
                    handlers: handlers_out,
                    finally_blocks: finally_blocks_set,
                    span: *span,
                });

                self.enter(post_bb);
            }
            CfgStmt::Match {
                subject,
                cases,
                span,
            } => {
                let post_bb = self.new_block();

                if cases.is_empty() {
                    let pre_block = self.current;
                    self.set_terminator(pre_block, HirTerminator::Jump(post_bb));
                } else {
                    let case_bbs: Vec<HirBlockId> =
                        cases.iter().map(|_| self.new_block()).collect();
                    let fallthrough_bb = self.new_block();

                    let predicates: Vec<ExprId> = cases
                        .iter()
                        .map(|case| {
                            self.alloc_match_pattern(module, *subject, case.pattern.clone(), *span)
                        })
                        .collect();

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
                        self.lower_cfg_stmts(&case.body, module);
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
        body: &[CfgStmt],
        module: &mut Module,
    ) -> (IndexMap<HirBlockId, HirBlock>, HirBlockId, Vec<TryScope>) {
        let mut builder = CfgBuilder::new();
        let entry = builder.new_block();
        builder.enter(entry);
        builder.lower_cfg_stmts(body, module);
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

        let (blocks, entry, _try_scopes) = build_cfg(
            &[CfgStmt::stmt(s1), CfgStmt::stmt(s2), CfgStmt::stmt(s3)],
            &mut module,
        );
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
        let (blocks, entry, _try_scopes) = build_cfg(
            &[CfgStmt::If {
                cond,
                then_body: vec![CfgStmt::stmt(then_stmt)],
                else_body: vec![CfgStmt::stmt(else_stmt)],
                span: dummy_span(),
            }],
            &mut module,
        );
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
        let (blocks, _entry, _try_scopes) = build_cfg(
            &[CfgStmt::While {
                cond,
                body: vec![CfgStmt::If {
                    cond: inner_cond,
                    then_body: vec![CfgStmt::stmt(break_stmt)],
                    else_body: vec![CfgStmt::stmt(continue_stmt)],
                    span: dummy_span(),
                }],
                else_body: vec![],
                span: dummy_span(),
            }],
            &mut module,
        );
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

        let (blocks, entry, _try_scopes) = build_cfg(
            &[CfgStmt::stmt(pre), CfgStmt::stmt(ret), CfgStmt::stmt(after)],
            &mut module,
        );
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

        let (blocks, entry, _try_scopes) = build_cfg(&[CfgStmt::stmt(raise_stmt)], &mut module);
        assert!(matches!(
            blocks[&entry].terminator,
            HirTerminator::Raise { cause: None, .. }
        ));
    }
}
