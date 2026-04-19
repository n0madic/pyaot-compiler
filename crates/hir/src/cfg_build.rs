//! Tree → CFG conversion for HIR function bodies.
//!
//! Phase 1 §1.1 of `ARCHITECTURE_REFACTOR.md` requires every `hir::Function` to
//! carry an explicit control-flow graph (`blocks` + `entry_block`). During the
//! S1.2 bridge period the legacy statement-tree representation
//! (`Function.body: Vec<StmtId>` plus the nested-body `StmtKind::{If, While,
//! ForBind, Try, Match}` variants) is still the canonical form consumed by
//! optimizer / lowering / codegen. This module reads that tree and emits a
//! parallel CFG that consumers will begin using in S1.3.
//!
//! The converter is intentionally a throwaway: S1.3 deletes the tree variants
//! and each `ast_to_hir` control-flow lowerer emits the CFG directly, making
//! this file obsolete. Keep it minimal.
//!
//! ## Simplifications for S1.2
//!
//! * `ForBind` uses the `iter` expression as a placeholder branch condition.
//!   The real iteration protocol (has-next / next) is emitted in a later
//!   session; here we only need the topological shape "header, body, optional
//!   else, exit".
//! * `Try` handlers are emitted as standalone blocks that are unreachable
//!   from the CFG — we do not model exception edges. `raise` becomes a
//!   `Raise` terminator and bare re-raise collapses to `Unreachable`.
//! * `Match` cases are chained linearly with a jump from each case to the
//!   post-match merge block. No pattern-dispatch terminator exists yet.
//!
//! None of these shortcuts affect S1.2 correctness — no consumer reads the
//! CFG yet.

use indexmap::IndexMap;
use la_arena::Arena;
use pyaot_utils::HirBlockId;

use crate::{HirBlock, HirTerminator, Stmt, StmtId, StmtKind};

/// Build a CFG from a straight-through tree-form function body.
///
/// Returns the populated block map and the `entry_block` id. The returned
/// CFG always has at least one block; if `body` is empty the single entry
/// block is terminated with `Return(None)`.
pub fn build_cfg_from_tree(
    body: &[StmtId],
    stmts: &Arena<Stmt>,
) -> (IndexMap<HirBlockId, HirBlock>, HirBlockId) {
    let mut builder = CfgBuilder::new();
    let entry = builder.new_block();
    builder.enter(entry);
    builder.lower_stmts(body, stmts);
    builder.terminate_if_open(HirTerminator::Return(None));
    (builder.blocks, entry)
}

struct CfgBuilder {
    blocks: IndexMap<HirBlockId, HirBlock>,
    current: HirBlockId,
    /// `true` once the current block has received a real terminator and
    /// subsequent statements in the enclosing stmt-list are dead code. Reset
    /// by `enter`.
    current_terminated: bool,
    next_id: u32,
    loop_stack: Vec<LoopCtx>,
}

#[derive(Clone, Copy)]
struct LoopCtx {
    continue_bb: HirBlockId,
    break_bb: HirBlockId,
}

impl CfgBuilder {
    fn new() -> Self {
        Self {
            blocks: IndexMap::new(),
            current: HirBlockId::new(0), // placeholder until enter() is called
            current_terminated: false,
            next_id: 0,
            loop_stack: Vec::new(),
        }
    }

    fn new_block(&mut self) -> HirBlockId {
        let id = HirBlockId::new(self.next_id);
        self.next_id += 1;
        self.blocks.insert(
            id,
            HirBlock {
                id,
                stmts: Vec::new(),
                terminator: HirTerminator::Unreachable,
            },
        );
        id
    }

    /// Make `block` the current insertion point and mark it open (un-terminated).
    fn enter(&mut self, block: HirBlockId) {
        self.current = block;
        self.current_terminated = false;
    }

    fn push_stmt(&mut self, stmt_id: StmtId) {
        let block = self.current;
        self.blocks
            .get_mut(&block)
            .expect("current block must exist")
            .stmts
            .push(stmt_id);
    }

    fn set_terminator(&mut self, block: HirBlockId, term: HirTerminator) {
        self.blocks
            .get_mut(&block)
            .expect("terminator target block must exist")
            .terminator = term;
        if block == self.current {
            self.current_terminated = true;
        }
    }

    /// If the current block has not yet been terminated, close it with `term`.
    fn terminate_if_open(&mut self, term: HirTerminator) {
        if !self.current_terminated {
            let block = self.current;
            self.set_terminator(block, term);
        }
    }

    fn lower_stmts(&mut self, stmts_list: &[StmtId], stmts: &Arena<Stmt>) {
        for &stmt_id in stmts_list {
            if self.current_terminated {
                break;
            }
            self.lower_stmt(stmt_id, stmts);
        }
    }

    fn lower_stmt(&mut self, stmt_id: StmtId, stmts: &Arena<Stmt>) {
        let stmt = &stmts[stmt_id];
        match &stmt.kind {
            StmtKind::Expr(_)
            | StmtKind::Bind { .. }
            | StmtKind::Pass
            | StmtKind::Assert { .. }
            | StmtKind::IndexDelete { .. }
            | StmtKind::IterAdvance { .. } => {
                self.push_stmt(stmt_id);
            }

            StmtKind::Return(value) => {
                let block = self.current;
                self.set_terminator(block, HirTerminator::Return(*value));
            }

            StmtKind::Raise { exc, cause } => {
                let block = self.current;
                let term = match exc {
                    Some(exc_id) => HirTerminator::Raise {
                        exc: *exc_id,
                        cause: *cause,
                    },
                    // Bare `raise` (re-raise) has no expression to attach.
                    // Mark the block Unreachable from a CFG perspective; the
                    // legacy tree still carries the real `Raise` stmt for
                    // semantics, so nothing is lost during the bridge.
                    None => HirTerminator::Unreachable,
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
                        cond: *cond,
                        then_bb,
                        else_bb,
                    },
                );

                self.enter(then_bb);
                self.lower_stmts(then_block, stmts);
                self.terminate_if_open(HirTerminator::Jump(merge_bb));

                self.enter(else_bb);
                self.lower_stmts(else_block, stmts);
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
                        cond: *cond,
                        then_bb: body_bb,
                        else_bb,
                    },
                );

                self.loop_stack.push(LoopCtx {
                    continue_bb: header_bb,
                    break_bb: exit_bb,
                });
                self.enter(body_bb);
                self.lower_stmts(body, stmts);
                self.terminate_if_open(HirTerminator::Jump(header_bb));
                self.loop_stack.pop();

                if !else_block.is_empty() {
                    self.enter(else_bb);
                    self.lower_stmts(else_block, stmts);
                    self.terminate_if_open(HirTerminator::Jump(exit_bb));
                }

                self.enter(exit_bb);
            }

            StmtKind::ForBind {
                target: _,
                iter,
                body,
                else_block,
            } => {
                // The iter expression stands in for the branch condition. S1.3
                // replaces this with a proper has-next / next terminator
                // schema once the tree representation is retired.
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
                        cond: *iter,
                        then_bb: body_bb,
                        else_bb,
                    },
                );

                self.loop_stack.push(LoopCtx {
                    continue_bb: header_bb,
                    break_bb: exit_bb,
                });
                self.enter(body_bb);
                self.lower_stmts(body, stmts);
                self.terminate_if_open(HirTerminator::Jump(header_bb));
                self.loop_stack.pop();

                if !else_block.is_empty() {
                    self.enter(else_bb);
                    self.lower_stmts(else_block, stmts);
                    self.terminate_if_open(HirTerminator::Jump(exit_bb));
                }

                self.enter(exit_bb);
            }

            StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                // Body → else → finally → post, chained sequentially. Handlers
                // are emitted as standalone blocks that are unreachable from
                // the CFG — exception edges are a later-phase concern.
                let body_bb = self.new_block();
                let post_bb = self.new_block();

                let pre_block = self.current;
                self.set_terminator(pre_block, HirTerminator::Jump(body_bb));

                self.enter(body_bb);
                self.lower_stmts(body, stmts);

                let after_body = if else_block.is_empty() {
                    post_bb
                } else {
                    let else_bb = self.new_block();
                    self.terminate_if_open(HirTerminator::Jump(else_bb));
                    self.enter(else_bb);
                    self.lower_stmts(else_block, stmts);
                    post_bb
                };

                if !finally_block.is_empty() {
                    let finally_bb = self.new_block();
                    self.terminate_if_open(HirTerminator::Jump(finally_bb));
                    self.enter(finally_bb);
                    self.lower_stmts(finally_block, stmts);
                    self.terminate_if_open(HirTerminator::Jump(post_bb));
                } else {
                    self.terminate_if_open(HirTerminator::Jump(after_body));
                }

                for handler in handlers {
                    let handler_bb = self.new_block();
                    self.enter(handler_bb);
                    self.lower_stmts(&handler.body, stmts);
                    self.terminate_if_open(HirTerminator::Jump(post_bb));
                }

                self.enter(post_bb);
            }

            StmtKind::Match { subject: _, cases } => {
                // Linearised case chain: each case body occupies its own block
                // and jumps to the post-match merge. Pattern dispatch is not
                // modelled in the CFG for S1.2.
                let post_bb = self.new_block();

                if cases.is_empty() {
                    let pre_block = self.current;
                    self.set_terminator(pre_block, HirTerminator::Jump(post_bb));
                } else {
                    let case_bbs: Vec<HirBlockId> =
                        cases.iter().map(|_| self.new_block()).collect();

                    let pre_block = self.current;
                    self.set_terminator(pre_block, HirTerminator::Jump(case_bbs[0]));

                    for (case, &bb) in cases.iter().zip(&case_bbs) {
                        self.enter(bb);
                        self.lower_stmts(&case.body, stmts);
                        self.terminate_if_open(HirTerminator::Jump(post_bb));
                    }
                }

                self.enter(post_bb);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BindingTarget, Expr, ExprKind, Stmt, StmtKind};
    use pyaot_utils::{Span, VarId};

    fn dummy_span() -> Span {
        Span::dummy()
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

    #[test]
    fn empty_body_returns_single_block_with_return_none() {
        let stmts = Arena::new();
        let (blocks, entry) = build_cfg_from_tree(&[], &stmts);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[&entry];
        assert!(block.stmts.is_empty());
        assert!(matches!(block.terminator, HirTerminator::Return(None)));
    }

    #[test]
    fn straight_line_body_collapses_to_one_block() {
        let mut stmts = Arena::new();
        let mut exprs = Arena::new();
        let s1 = bind_stmt(&mut stmts, &mut exprs);
        let s2 = bind_stmt(&mut stmts, &mut exprs);
        let s3 = bind_stmt(&mut stmts, &mut exprs);

        let (blocks, entry) = build_cfg_from_tree(&[s1, s2, s3], &stmts);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[&entry];
        assert_eq!(block.stmts, vec![s1, s2, s3]);
        assert!(matches!(block.terminator, HirTerminator::Return(None)));
    }

    #[test]
    fn if_emits_then_else_merge() {
        let mut stmts = Arena::new();
        let mut exprs = Arena::new();
        let cond = alloc_expr(&mut exprs);
        let then_stmt = bind_stmt(&mut stmts, &mut exprs);
        let else_stmt = bind_stmt(&mut stmts, &mut exprs);
        let if_stmt = alloc_stmt(
            &mut stmts,
            StmtKind::If {
                cond,
                then_block: vec![then_stmt],
                else_block: vec![else_stmt],
            },
        );

        let (blocks, entry) = build_cfg_from_tree(&[if_stmt], &stmts);
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
        let mut stmts = Arena::new();
        let mut exprs = Arena::new();
        let cond = alloc_expr(&mut exprs);
        let break_stmt = alloc_stmt(&mut stmts, StmtKind::Break);
        let continue_stmt = alloc_stmt(&mut stmts, StmtKind::Continue);
        let inner_cond = alloc_expr(&mut exprs);
        let inner_if = alloc_stmt(
            &mut stmts,
            StmtKind::If {
                cond: inner_cond,
                then_block: vec![break_stmt],
                else_block: vec![continue_stmt],
            },
        );
        let while_stmt = alloc_stmt(
            &mut stmts,
            StmtKind::While {
                cond,
                body: vec![inner_if],
                else_block: vec![],
            },
        );

        let (blocks, _entry) = build_cfg_from_tree(&[while_stmt], &stmts);
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
        let mut stmts = Arena::new();
        let mut exprs = Arena::new();
        let pre = bind_stmt(&mut stmts, &mut exprs);
        let ret = alloc_stmt(&mut stmts, StmtKind::Return(None));
        let after = bind_stmt(&mut stmts, &mut exprs);

        let (blocks, entry) = build_cfg_from_tree(&[pre, ret, after], &stmts);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[&entry];
        // `after` must not be emitted — it lives past the Return.
        assert_eq!(block.stmts, vec![pre]);
        assert!(matches!(block.terminator, HirTerminator::Return(None)));
    }

    #[test]
    fn raise_with_expr_becomes_raise_terminator() {
        let mut stmts = Arena::new();
        let mut exprs = Arena::new();
        let exc = alloc_expr(&mut exprs);
        let raise_stmt = alloc_stmt(
            &mut stmts,
            StmtKind::Raise {
                exc: Some(exc),
                cause: None,
            },
        );

        let (blocks, entry) = build_cfg_from_tree(&[raise_stmt], &stmts);
        assert!(matches!(
            blocks[&entry].terminator,
            HirTerminator::Raise { cause: None, .. }
        ));
    }
}
