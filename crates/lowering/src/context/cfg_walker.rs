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

/// Return true if the pattern (or any nested pattern) binds a capture name.
/// Used by `is_cfg_walker_eligible` — patterns with captures have their
/// bindings dropped by the current `lower_match_pattern`, so functions
/// containing them must fall back to the tree walker until case-body
/// binding emission is implemented.
fn pattern_has_capture(pattern: &hir::Pattern) -> bool {
    match pattern {
        hir::Pattern::MatchValue(_) | hir::Pattern::MatchSingleton(_) => false,
        hir::Pattern::MatchAs { pattern, name } => {
            if name.is_some() {
                return true;
            }
            pattern
                .as_ref()
                .map(|p| pattern_has_capture(p))
                .unwrap_or(false)
        }
        hir::Pattern::MatchSequence { patterns } => patterns.iter().any(pattern_has_capture),
        hir::Pattern::MatchStar(name) => name.is_some(),
        hir::Pattern::MatchOr(alternatives) => alternatives.iter().any(pattern_has_capture),
        hir::Pattern::MatchMapping { patterns, rest, .. } => {
            rest.is_some() || patterns.iter().any(pattern_has_capture)
        }
        hir::Pattern::MatchClass {
            patterns,
            kwd_patterns,
            ..
        } => {
            patterns.iter().any(pattern_has_capture) || kwd_patterns.iter().any(pattern_has_capture)
        }
    }
}

impl<'a> Lowering<'a> {
    /// Check whether a function is eligible for CFG-walker lowering.
    /// Returns `false` for functions that hit CFG walker limitations:
    /// - Non-empty `try_scopes` (exception-frame emission pending)
    /// - `MatchPattern` exprs with capturing patterns (binding extraction
    ///   pending — bindings would be dropped, breaking capture semantics)
    ///
    /// All limitations discovered during 2026-04-19 validation (range
    /// dispatch, primitive-list unboxing, generator semantics, JIT
    /// narrowing) have been resolved; this eligibility check now
    /// excludes only the two classes that genuinely need more work:
    /// try/except emission and capturing match patterns.
    pub(crate) fn is_cfg_walker_eligible(
        &self,
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> bool {
        if !func.try_scopes.is_empty() {
            return false;
        }
        // Scan every block's terminator for `Branch(MatchPattern(..), ..)`
        // with a capturing pattern.
        for block in func.blocks.values() {
            if let hir::HirTerminator::Branch { cond, .. } = &block.terminator {
                let cond_expr = &hir_module.exprs[*cond];
                if let hir::ExprKind::MatchPattern { pattern, .. } = &cond_expr.kind {
                    if pattern_has_capture(pattern) {
                        return false;
                    }
                }
            }
        }
        true
    }

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

        // Narrowings computed just-in-time: when a block's terminator
        // is `Branch(cond)`, we analyze `cond` after its stmts have
        // lowered and record the then/else narrowings in this map for
        // the successor blocks to pick up. Pre-computing up-front
        // doesn't work because `var_types` for LOCAL variables only
        // gets populated once their Bind stmt is lowered — so
        // `analyze_condition_for_narrowing` returns None because
        // `get_var_type` sees no entry for an as-yet-unbound local.
        // Tree walker's `lower_if` didn't hit this because it runs
        // AFTER the containing stmts have lowered.
        let mut narrowings: IndexMap<HirBlockId, Vec<crate::narrowing::TypeNarrowingInfo>> =
            IndexMap::new();

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
            // CRITICAL: narrowing must stay active while lowering the
            // TERMINATOR too — Return(expr) / Branch(cond) / Raise(exc)
            // evaluate exprs that read var types. Popping before
            // terminator emission produces "unknown attribute" errors
            // on narrowed Var accesses (e.g. `return other.x` where
            // `other` was narrowed to a class).
            let narrow = narrowings.get(hir_id).cloned();
            if let Some(ref n) = narrow {
                self.push_narrowing_frame(n);
            }

            // Lower straight-line statements.
            for &stmt_id in &hir_block.stmts {
                let stmt = &hir_module.stmts[stmt_id];
                self.lower_stmt(stmt, hir_module, mir_func)?;
            }

            // §1.17b-c — just-in-time narrowing analysis. After the
            // block's stmts have lowered (populating var_types with
            // local bindings), analyze the Branch terminator's cond
            // and record narrowings for the successor then/else
            // blocks to pick up on their visits.
            if let hir::HirTerminator::Branch {
                cond,
                then_bb,
                else_bb,
            } = &hir_block.terminator
            {
                let cond_expr = &hir_module.exprs[*cond];
                let analysis = self.analyze_condition_for_narrowing(cond_expr, hir_module);
                if !analysis.then_narrowings.is_empty() {
                    narrowings.insert(*then_bb, analysis.then_narrowings);
                }
                if !analysis.else_narrowings.is_empty() {
                    narrowings.insert(*else_bb, analysis.else_narrowings);
                }
            }

            // Emit MIR terminator WHILE narrowing is still active, so
            // embedded exprs see the narrowed types.
            if !self.current_block_has_terminator() {
                self.emit_hir_terminator(&hir_block.terminator, &hir_to_mir, hir_module, mir_func)?;
            }

            // Pop narrowing frame AFTER terminator emission.
            if narrow.is_some() {
                self.pop_narrowing_frame();
            }
        }

        Ok(())
    }

    /// Produce a typed default operand matching `ret_ty` — for use when
    /// a `Return(None)` terminator lands in a function declared with a
    /// non-None return type (abstract methods, `pass` bodies, implicit
    /// fall-through). Mirrors the tree walker's post-processing logic
    /// in `lower_function`.
    fn default_return_operand(
        &mut self,
        ret_ty: &pyaot_types::Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match ret_ty {
            pyaot_types::Type::Int => mir::Operand::Constant(mir::Constant::Int(0)),
            pyaot_types::Type::Float => mir::Operand::Constant(mir::Constant::Float(0.0)),
            pyaot_types::Type::Bool => mir::Operand::Constant(mir::Constant::Bool(false)),
            pyaot_types::Type::Str => {
                let empty_str = self.interner.intern("");
                let str_local = self.emit_runtime_call(
                    mir::RuntimeFunc::MakeStr,
                    vec![mir::Operand::Constant(mir::Constant::Str(empty_str))],
                    pyaot_types::Type::Str,
                    mir_func,
                );
                mir::Operand::Local(str_local)
            }
            _ => mir::Operand::Constant(mir::Constant::None),
        }
    }

    /// Translate a `HirTerminator` into a `mir::Terminator` and assign it
    /// to the current MIR block. Used by `lower_function_cfg`.
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
                // Truthiness conversion: the MIR `Branch` expects an i8
                // bool operand. If the HIR cond type isn't already Bool,
                // delegate to `convert_to_bool` (Int → !=0, Float → !=0.0,
                // Str/collections → len>0, None → false, Union/Any →
                // rt_is_truthy). Mirrors what `lower_if`/`lower_while` do.
                let cond_expr = &hir_module.exprs[*cond];
                let cond_type = self.get_type_of_expr_id(*cond, hir_module);
                let cond_operand = self.lower_expr(cond_expr, hir_module, mir_func)?;
                let cond_bool =
                    self.emit_truthiness_conversion_if_needed(cond_operand, &cond_type, mir_func);
                let mir_then = hir_to_mir[then_bb];
                let mir_else = hir_to_mir[else_bb];
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: cond_bool,
                    then_block: mir_then,
                    else_block: mir_else,
                };
            }
            hir::HirTerminator::Return(opt_expr_id) => {
                // §1.17b-c — delegate to `lower_return` which applies
                // the §E.7 NotImplementedT boxing + type coercion via
                // `lower_expr_expecting`. Using `lower_return` keeps
                // emission shape identical to the tree walker's Return
                // path, avoiding signature-mismatch Cranelift errors
                // for `$resume` / Union-returning dunders.
                //
                // Special case: bridge emits `Return(None)` for
                // implicitly-terminated blocks (pass bodies, missing
                // return, abstract methods). If the function's declared
                // return type is non-None, the tree walker's
                // post-processing pass replaces with a typed default.
                // We replicate that here so the walker's emission
                // matches the MIR function signature.
                if opt_expr_id.is_none() {
                    let ret_ty = self
                        .symbols
                        .current_func_return_type
                        .clone()
                        .unwrap_or(pyaot_types::Type::None);
                    if !matches!(ret_ty, pyaot_types::Type::None) {
                        let default_operand = self.default_return_operand(&ret_ty, mir_func);
                        self.current_block_mut().terminator =
                            mir::Terminator::Return(Some(default_operand));
                        return Ok(());
                    }
                }
                self.lower_return(opt_expr_id.as_ref(), hir_module, mir_func)?;
            }
            hir::HirTerminator::Raise { exc, cause } => {
                // Delegate to `emit_raise_terminator` — the factored
                // entry point shared with the tree walker's `lower_raise`.
                // Sets the current block's MIR terminator directly
                // (Raise / RaiseInstance / RaiseCustom depending on the
                // exception kind) without pushing a dead-code block.
                let exc_opt = Some(*exc);
                self.emit_raise_terminator(&exc_opt, cause, hir_module, mir_func)?;
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
