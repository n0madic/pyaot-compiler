//! §1.17b-c — CFG walker for function lowering.
//!
//! Alternative to the tree-walking `lower_function` core loop: iterates
//! `func.blocks` in IndexMap order (bridge's pre-order DFS of the source
//! tree), emits one MIR block per HIR block, translates HIR terminators
//! to MIR terminators. Consumes the S1.17b-c infrastructure pieces:
//!
//! - Per-function iterator cache (`CodeGenState::iter_cache`) — populated
//!   by `StmtKind::IterSetup` lowering, consumed by `IterHasNext` /
//!   `IterAdvance` lowering.
//! - Per-block narrowings from `precompute_block_narrowings(func)` —
//!   pushed/popped around each block's stmt lowering.
//! - Pattern predicate via `lower_match_pattern` called by the
//!   `ExprKind::MatchPattern` arm in `lower_expr`.
//!
//! **Limitations (by design, follow-up needed for tree deletion)**:
//!
//! - **Try-scope emission** not yet implemented. Functions with
//!   non-empty `func.try_scopes` must still go through the legacy tree
//!   walker. The CFG walker returns an error in that case so callers
//!   can fall back.
//! - **MatchPattern bindings** — captures from patterns like
//!   `case Point(x, y)` are dropped by the current `lower_match_pattern`.
//!   Patterns without captures (MatchValue, MatchSingleton, wildcard
//!   MatchAs) work correctly; capturing patterns lose their bindings.
//!   Full correctness requires the bridge to emit binding-extraction
//!   HIR stmts in each case-body block head (follow-up work).
//! - **Yield terminator** — generator desugaring replaces Yield with
//!   regular flow before lowering, so this never occurs at lowering
//!   time. The walker panics if it encounters one.
//!
//! The walker is **not** yet the default lowering path. It's exposed as
//! a standalone method that the next session can switch to once the
//! limitations above are resolved. `lower_function` continues to use
//! the tree walker.

use indexmap::IndexMap;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_utils::{BlockId, HirBlockId};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a function's body via CFG walking instead of tree iteration.
    ///
    /// Caller contract: the MIR entry block is already pushed onto
    /// `codegen.current_blocks` (standard `lower_function` prologue);
    /// this method allocates additional MIR blocks for each non-entry
    /// HIR block, walks them in IndexMap order, and emits terminators.
    ///
    /// Errors if `func.try_scopes` is non-empty — exception-frame
    /// emission is not yet implemented. Callers should fall back to the
    /// tree walker for those functions.
    #[allow(dead_code)] // not yet wired — S1.17b-c final piece (try-scope emission pending)
    pub(crate) fn lower_function_cfg(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        if !func.try_scopes.is_empty() {
            return Err(CompilerError::type_error(
                "lower_function_cfg: try-scope emission not yet implemented".to_string(),
                func.span,
            ));
        }

        // Clear iter_cache — fresh for this function.
        self.codegen.iter_cache.clear();

        // Pre-allocate an IndexMap<HirBlockId, BlockId> by calling
        // `new_block` for each non-entry HIR block. The entry HIR block
        // maps to the MIR block already pushed by `lower_function`'s
        // prologue. Allocated-but-not-pushed MIR blocks are stashed in
        // `pending_blocks` and pushed when their HIR block is visited.
        let entry_mir_id = self.codegen.current_blocks[self.codegen.current_block_idx].id;
        let mut hir_to_mir: IndexMap<HirBlockId, BlockId> = IndexMap::new();
        hir_to_mir.insert(func.entry_block, entry_mir_id);

        let mut pending_blocks: IndexMap<BlockId, mir::BasicBlock> = IndexMap::new();
        for hir_id in func.blocks.keys() {
            if *hir_id == func.entry_block {
                continue;
            }
            let block = self.new_block();
            hir_to_mir.insert(*hir_id, block.id);
            pending_blocks.insert(block.id, block);
        }

        // Precompute narrowings for each block.
        let narrowings = self.precompute_block_narrowings(func, hir_module);

        // Walk HIR blocks in IndexMap order, pushing MIR blocks as we go.
        for (hir_id, hir_block) in &func.blocks {
            // Position at the MIR block for this HIR block.
            if *hir_id != func.entry_block {
                let mir_id = hir_to_mir[hir_id];
                let block = pending_blocks
                    .shift_remove(&mir_id)
                    .expect("mir block allocated but missing from pending_blocks");
                self.push_block(block);
            }

            // Apply narrowing frame for this block's entry type-info.
            let narrow = narrowings.get(hir_id).cloned();
            if let Some(ref n) = narrow {
                self.push_narrowing_frame(n);
            }

            // Lower straight-line statements.
            for &stmt_id in &hir_block.stmts {
                let stmt = &hir_module.stmts[stmt_id];
                self.lower_stmt(stmt, hir_module, mir_func)?;
            }

            // Pop narrowing frame.
            if narrow.is_some() {
                self.pop_narrowing_frame();
            }

            // Emit MIR terminator if the block isn't already terminated
            // (e.g., by a Return stmt inside the block).
            if !self.current_block_has_terminator() {
                self.emit_hir_terminator(&hir_block.terminator, &hir_to_mir, hir_module, mir_func)?;
            }
        }

        Ok(())
    }

    /// Translate a `HirTerminator` into a `mir::Terminator` and assign it
    /// to the current MIR block. Used by `lower_function_cfg`.
    #[allow(dead_code)]
    fn emit_hir_terminator(
        &mut self,
        term: &hir::HirTerminator,
        hir_to_mir: &IndexMap<HirBlockId, BlockId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        match term {
            hir::HirTerminator::Jump(target) => {
                let mir_target = hir_to_mir[target];
                self.current_block_mut().terminator = mir::Terminator::Goto(mir_target);
            }
            hir::HirTerminator::Branch {
                cond,
                then_bb,
                else_bb,
            } => {
                let cond_expr = &hir_module.exprs[*cond];
                let cond_operand = self.lower_expr(cond_expr, hir_module, mir_func)?;
                let mir_then = hir_to_mir[then_bb];
                let mir_else = hir_to_mir[else_bb];
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: cond_operand,
                    then_block: mir_then,
                    else_block: mir_else,
                };
            }
            hir::HirTerminator::Return(opt_expr_id) => {
                let operand = match opt_expr_id {
                    Some(expr_id) => {
                        let expr = &hir_module.exprs[*expr_id];
                        Some(self.lower_expr(expr, hir_module, mir_func)?)
                    }
                    None => None,
                };
                self.current_block_mut().terminator = mir::Terminator::Return(operand);
            }
            hir::HirTerminator::Raise { .. } => {
                // Raise terminator requires full `rt_exc_raise` dispatch
                // with exception-type tag + message + cause handling —
                // same complexity as `lower_raise`. Leave as a TODO
                // guarded by the try-scope skip above (functions without
                // try/except can still raise, but those typically go
                // through straight-line `StmtKind::Raise` which becomes
                // a Raise terminator in the bridge). Follow-up work.
                return Err(CompilerError::type_error(
                    "lower_function_cfg: Raise terminator emission not yet implemented".to_string(),
                    pyaot_utils::Span::dummy(),
                ));
            }
            hir::HirTerminator::Unreachable => {
                // MIR block's default terminator is already Unreachable.
            }
            hir::HirTerminator::Yield { .. } => {
                unreachable!(
                    "Yield terminator reached lowering — generator desugaring should \
                     have replaced it pre-lowering"
                );
            }
        }
        Ok(())
    }
}
