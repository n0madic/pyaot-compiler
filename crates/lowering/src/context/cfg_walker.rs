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
//! - Per-block narrowings from JIT analysis — computed after each block's
//!   stmts have lowered (so local var types are available).
//! - Pattern predicate via `lower_match_pattern` called by the
//!   `ExprKind::MatchPattern` arm in `lower_expr`.
//! - Try-scope emission via `TryScopeCtx` pre-pass — injects
//!   `ExcPushFrame` / `TrySetjmp` at try-predecessor blocks,
//!   `ExcPopFrame` at try-body exits, handler preambles/exits, and
//!   handler-dispatch + finally infrastructure blocks post-loop.
//!   Finally scopes are lowered by routing exception/handler exits into the
//!   CFG-emitted finally blocks, then branching on a per-scope
//!   `propagating` flag at the end of the finally region.
//! - Match capturing-pattern bindings — `case_body_bindings` pre-pass
//!   emits bindings at the head of case-body blocks so `case Point(x,y)`
//!   correctly binds `x` and `y`.
//!
//! **Remaining limitations (follow-up needed for full tree deletion)**:
//!
//! - **Try/finally edge cases** — common try/except/finally shapes are
//!   handled in CFG form, but exotic cases (e.g. nested handler-body CFG
//!   blocks with their own exits, `return` from within a finally region)
//!   still rely on the HIR not constructing those shapes in the current
//!   fixtures. Follow-up: track full handler/finally regions explicitly.
//! - **Yield terminator** — generator desugaring replaces Yield with
//!   regular flow before lowering, so this never occurs at lowering
//!   time. The walker panics if it encounters one.

use indexmap::{IndexMap, IndexSet};
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, HirBlockId, LocalId, VarId};

use crate::context::Lowering;
use crate::exceptions::get_exc_type_tag_from_type;

// ---------------------------------------------------------------------------
// Pattern capture helper
// ---------------------------------------------------------------------------

/// Return true if the pattern (or any nested pattern) binds a capture name.
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

// ---------------------------------------------------------------------------
// Try-scope emission data structures
// ---------------------------------------------------------------------------

/// Collect VarIds assigned in the CFG-form try blocks of a TryScope.
/// In CFG form, stmts inside blocks are straight-line (no nested control
/// flow in the stmt list). We only need Bind targets — the filtered set
/// of variables that need cell wrapping before TrySetjmp.
fn collect_cfg_try_vars(
    scope: &hir::TryScope,
    func: &hir::Function,
    hir_module: &hir::Module,
) -> IndexSet<VarId> {
    let mut assigned = IndexSet::new();
    for &hid in &scope.try_blocks {
        if let Some(hir_block) = func.blocks.get(&hid) {
            for &stmt_id in &hir_block.stmts {
                let stmt = &hir_module.stmts[stmt_id];
                if let hir::StmtKind::Bind { target, .. } = &stmt.kind {
                    target.for_each_var(&mut |var_id| {
                        assigned.insert(var_id);
                    });
                }
            }
        }
    }
    assigned
}

/// Per-TryScope emission data — pre-allocated MIR blocks and locals.
/// All fields are Copy so the struct can be cheaply copied out of the Vec.
#[derive(Clone, Copy)]
struct TryScopeEmission {
    frame_local: LocalId,
    propagating_local: LocalId,
    /// MIR-only block: ExcCheckClass dispatch chain for this scope's handlers.
    handler_dispatch_mir_id: BlockId,
    /// MIR-only block: enters the real finally CFG (if present) or falls
    /// straight through to `finally_body_mir_id`.
    finally_dispatch_mir_id: BlockId,
    /// MIR-only block: propagating-flag check → reraise | normal_exit.
    finally_body_mir_id: BlockId,
    /// MIR-only block: `Reraise` terminator.
    reraise_mir_id: BlockId,
    /// MIR-only block: `Goto(post_bb_mir_id)` — normal continuation.
    normal_exit_mir_id: BlockId,
    /// MIR block that corresponds to the HIR post_bb (block after the try).
    post_bb_mir_id: Option<BlockId>,
    /// MIR block for the first HIR finally block, if the scope has a
    /// `finally:` clause.
    first_finally_mir_id: Option<BlockId>,
}

/// Context built in the pre-pass; consumed by the main walk.
struct TryScopeCtx {
    scopes: Vec<TryScopeEmission>,
    /// `try_blocks[0]` of each scope → scope_idx.
    /// Consulted when emitting the *predecessor* block's terminator:
    /// `Jump(try_entry)` → `TrySetjmp(...)`.
    try_entry_map: IndexMap<HirBlockId, usize>,
    /// All HIR blocks inside any scope's `try_blocks` → Vec<scope_idx>.
    /// A block may belong to multiple scopes (nested try), so we keep all
    /// scope indices.  Used to decide whether to inject `ExcPopFrame`.
    try_body_blocks: IndexMap<HirBlockId, Vec<usize>>,
    /// Handler `entry_block` → (scope_idx, handler_idx).
    handler_entry_map: IndexMap<HirBlockId, (usize, usize)>,
    /// HIR blocks that belong to a scope's `finally:` region.
    finally_blocks_map: IndexMap<HirBlockId, usize>,
}

type ScopeCellData = IndexMap<usize, (IndexMap<VarId, LocalId>, IndexMap<VarId, LocalId>)>;

impl TryScopeCtx {
    fn new() -> Self {
        Self {
            scopes: Vec::new(),
            try_entry_map: IndexMap::new(),
            try_body_blocks: IndexMap::new(),
            handler_entry_map: IndexMap::new(),
            finally_blocks_map: IndexMap::new(),
        }
    }
}

impl<'a> Lowering<'a> {
    /// Build a map from case-body `HirBlockId` to `(subject ExprId, Pattern)`
    /// for all match cases with capturing patterns.  Used by the main loop to
    /// emit pattern-variable bindings at the head of each case-body block.
    fn build_case_body_bindings_map(
        func: &hir::Function,
        hir_module: &hir::Module,
    ) -> IndexMap<HirBlockId, (hir::ExprId, Box<hir::Pattern>)> {
        let mut map = IndexMap::new();
        for hir_block in func.blocks.values() {
            if let hir::HirTerminator::Branch { cond, then_bb, .. } = &hir_block.terminator {
                let cond_expr = &hir_module.exprs[*cond];
                if let hir::ExprKind::MatchPattern { subject, pattern } = &cond_expr.kind {
                    if pattern_has_capture(pattern) {
                        map.insert(*then_bb, (*subject, pattern.clone()));
                    }
                }
            }
        }
        map
    }

    /// Lower a function's body via CFG walking instead of tree iteration.
    ///
    /// Caller contract: the MIR entry block is already pushed onto
    /// `codegen.current_blocks` (standard `lower_function` prologue);
    /// this method allocates additional MIR blocks for each non-entry
    /// HIR block, walks them in IndexMap order, and emits terminators.
    ///
    /// Falls back (via `is_cfg_walker_eligible`) for functions with
    /// try-scopes that contain `finally_blocks` — those require additional
    /// inter-block state not yet implemented.
    pub(crate) fn lower_function_cfg(
        &mut self,
        func: &hir::Function,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Clear iter_cache — fresh for this function.
        self.codegen.iter_cache.clear();

        // ── Step 1: allocate one MIR block per HIR block ────────────────
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

        // ── Step 2: try-scope pre-pass ───────────────────────────────────
        let mut try_ctx = TryScopeCtx::new();
        // MIR-only infrastructure blocks: 5 per scope (stored in insertion order).
        // Pushed to current_blocks after the main loop.
        let mut infra_blocks: Vec<mir::BasicBlock> = Vec::new();

        // Collect variables assigned in each scope's try_blocks (in CFG form: flat
        // Bind stmts in the blocks). These will be cell-wrapped before TrySetjmp to
        // ensure their values survive longjmp — see `collect_cfg_try_vars` below.
        let scope_try_vars: Vec<IndexSet<VarId>> = func
            .try_scopes
            .iter()
            .map(|scope| collect_cfg_try_vars(scope, func, hir_module))
            .collect();

        // Scope cell data: populated during TrySetjmp emission, consumed in normal_exit.
        // Maps scope_idx → (saved_nonlocal_cells, cell_locals: VarId → cell LocalId).
        let mut scope_cell_data: ScopeCellData = IndexMap::new();

        for (scope_idx, scope) in func.try_scopes.iter().enumerate() {
            // Even if try_blocks is empty (degenerate scope), we still push emi
            // so that try_ctx.scopes[scope_idx] remains valid in the post-loop.
            // Scopes with no try_blocks have no try_entry_map entry and are
            // effectively no-ops in the main walk.
            let try_entry_opt = scope.try_blocks.first().copied();

            let frame_local = self.alloc_and_add_local(Type::Int, mir_func);
            let propagating_local = self.alloc_and_add_local(Type::Bool, mir_func);

            let handler_dispatch = self.new_block();
            let finally_dispatch = self.new_block();
            let finally_body = self.new_block();
            let reraise_block = self.new_block();
            let normal_exit = self.new_block();

            let handler_dispatch_mir_id = handler_dispatch.id;
            let finally_dispatch_mir_id = finally_dispatch.id;
            let finally_body_mir_id = finally_body.id;
            let reraise_mir_id = reraise_block.id;
            let normal_exit_mir_id = normal_exit.id;

            infra_blocks.push(handler_dispatch);
            infra_blocks.push(finally_dispatch);
            infra_blocks.push(finally_body);
            infra_blocks.push(reraise_block);
            infra_blocks.push(normal_exit);

            // Locate post_bb: the HIR block that all paths after this try
            // scope flow into.  The bridge always sets handler entry_blocks
            // to Jump(post_bb), so the first handler's terminator gives us
            // post_bb directly.  Fallback: last try_block's Jump target
            // (used when there are no handlers, which shouldn't occur for
            // finally-free scopes handled by this walker, but kept as
            // a safety net).
            let post_bb_mir_id = scope
                .finally_blocks
                .last()
                .and_then(|&last| func.blocks.get(&last))
                .and_then(|blk| match blk.terminator {
                    hir::HirTerminator::Jump(t) => hir_to_mir.get(&t).copied(),
                    _ => None,
                })
                .or_else(|| {
                    scope
                        .handlers
                        .first()
                        .and_then(|h| func.blocks.get(&h.entry_block))
                        .and_then(|blk| match blk.terminator {
                            hir::HirTerminator::Jump(t) => hir_to_mir.get(&t).copied(),
                            _ => None,
                        })
                })
                .or_else(|| {
                    scope
                        .try_blocks
                        .last()
                        .and_then(|&last| func.blocks.get(&last))
                        .and_then(|blk| match blk.terminator {
                            hir::HirTerminator::Jump(t) => hir_to_mir.get(&t).copied(),
                            _ => None,
                        })
                });

            let first_finally_mir_id = scope
                .finally_blocks
                .first()
                .and_then(|hid| hir_to_mir.get(hid).copied());

            let emi = TryScopeEmission {
                frame_local,
                propagating_local,
                handler_dispatch_mir_id,
                finally_dispatch_mir_id,
                finally_body_mir_id,
                reraise_mir_id,
                normal_exit_mir_id,
                post_bb_mir_id,
                first_finally_mir_id,
            };

            if let Some(try_entry) = try_entry_opt {
                try_ctx.try_entry_map.insert(try_entry, scope_idx);
            }
            for &hid in &scope.try_blocks {
                try_ctx
                    .try_body_blocks
                    .entry(hid)
                    .or_default()
                    .push(scope_idx);
            }
            for (h_idx, handler) in scope.handlers.iter().enumerate() {
                try_ctx
                    .handler_entry_map
                    .insert(handler.entry_block, (scope_idx, h_idx));
            }
            for &hid in &scope.finally_blocks {
                try_ctx.finally_blocks_map.insert(hid, scope_idx);
            }

            try_ctx.scopes.push(emi);
        }

        // ── Step 3: match capturing-pattern bindings pre-pass ────────────
        let case_body_bindings = Self::build_case_body_bindings_map(func, hir_module);

        // ── Step 4: JIT narrowing map ────────────────────────────────────
        let mut narrowings: IndexMap<HirBlockId, Vec<crate::narrowing::TypeNarrowingInfo>> =
            IndexMap::new();

        // ── Step 5: main block walk ──────────────────────────────────────
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
            // CRITICAL: must stay active through terminator emission.
            let narrow = narrowings.get(hir_id).cloned();
            if let Some(ref n) = narrow {
                self.push_narrowing_frame(n);
            }

            // Handler preamble — before stmts, if this is a handler entry.
            if let Some(&(scope_idx, h_idx)) = try_ctx.handler_entry_map.get(hir_id) {
                let scope = &func.try_scopes[scope_idx];
                let handler = &scope.handlers[h_idx];
                self.emit_instruction(mir::InstructionKind::ExcStartHandling);
                if let Some(var_id) = handler.name {
                    let exc_type = handler.ty.clone().unwrap_or(Type::BuiltinException(
                        pyaot_core_defs::BuiltinExceptionKind::Exception,
                    ));
                    let exc_local = self.alloc_and_add_local(exc_type.clone(), mir_func);
                    self.emit_instruction(mir::InstructionKind::ExcGetCurrent { dest: exc_local });
                    self.insert_var_local(var_id, exc_local);
                    self.insert_var_type(var_id, exc_type);
                }
                self.emit_instruction(mir::InstructionKind::ExcClear);
            }

            // Match case-body bindings — emit captured variable bindings
            // at the head of each case-body block (for capturing patterns
            // like `case Point(x, y):`).
            if let Some((subject_id, pattern)) = case_body_bindings.get(hir_id).cloned() {
                let subject_expr = &hir_module.exprs[subject_id];
                let subject_op = self.lower_expr(subject_expr, hir_module, mir_func)?;
                let subject_type = self.get_type_of_expr_id(subject_id, hir_module);
                let subject_local = self.alloc_and_add_local(subject_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: subject_local,
                    src: subject_op,
                });
                let (_, bindings) = self.generate_pattern_check(
                    &pattern,
                    mir::Operand::Local(subject_local),
                    &subject_type,
                    hir_module,
                    mir_func,
                )?;
                for (var_id, op, ty) in bindings {
                    let local = if let Some(existing) = self.get_var_local(&var_id) {
                        existing
                    } else {
                        let l = self.alloc_and_add_local(ty.clone(), mir_func);
                        self.insert_var_local(var_id, l);
                        self.insert_var_type(var_id, ty);
                        l
                    };
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: local,
                        src: op,
                    });
                }
            }

            // Lower straight-line statements.
            for &stmt_id in &hir_block.stmts {
                let stmt = &hir_module.stmts[stmt_id];
                self.lower_stmt(stmt, hir_module, mir_func)?;
            }

            // §1.17b-c — just-in-time narrowing analysis.
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

            // Emit terminator (with try-scope overrides where needed).
            if !self.current_block_has_terminator() {
                // ── Try-scope predecessor: Jump(try_entry) → TrySetjmp ──
                let mut try_setjmp_emitted = false;
                if let hir::HirTerminator::Jump(target) = hir_block.terminator {
                    if let Some(&scope_idx) = try_ctx.try_entry_map.get(&target) {
                        let emi = try_ctx.scopes[scope_idx]; // Copy

                        // Cell-wrap variables assigned in this try scope's body
                        // so their values survive longjmp. Mirrors tree walker's
                        // `lower_try` cell-wrapping logic (exceptions.rs line 523+).
                        let saved = self.clone_nonlocal_cells();
                        let mut cell_locals: IndexMap<VarId, LocalId> = IndexMap::new();
                        let vars_to_wrap = scope_try_vars[scope_idx].clone();
                        for var_id in &vars_to_wrap {
                            if let Some(existing_local) = self.get_var_local(var_id) {
                                if self.is_global(var_id)
                                    || self.is_cell_var(var_id)
                                    || self.has_nonlocal_cell(var_id)
                                    || self.has_var_func(var_id)
                                    || self.has_var_closure(var_id)
                                {
                                    continue;
                                }
                                let var_type =
                                    self.get_var_type(var_id).cloned().unwrap_or(Type::Int);
                                let make_func = self.get_make_cell_func(&var_type);
                                let cell_local = self.emit_runtime_call_gc(
                                    make_func,
                                    vec![mir::Operand::Local(existing_local)],
                                    Type::HeapAny,
                                    mir_func,
                                );
                                cell_locals.insert(*var_id, cell_local);
                                self.insert_nonlocal_cell(*var_id, cell_local);
                            }
                        }
                        scope_cell_data.insert(scope_idx, (saved, cell_locals));

                        self.emit_instruction(mir::InstructionKind::Const {
                            dest: emi.propagating_local,
                            value: mir::Constant::Bool(false),
                        });
                        self.emit_instruction(mir::InstructionKind::ExcPushFrame {
                            frame_local: emi.frame_local,
                        });
                        self.current_block_mut().terminator = mir::Terminator::TrySetjmp {
                            frame_local: emi.frame_local,
                            try_body: hir_to_mir[&target],
                            handler_entry: emi.handler_dispatch_mir_id,
                        };
                        try_setjmp_emitted = true;
                    }
                }

                if !try_setjmp_emitted {
                    // ── ExcPopFrame injection for try-body exits ──────────
                    // For nested try scopes, a single block may belong to
                    // multiple scopes. Emit ExcPopFrame for each scope where
                    // the block's terminator exits that scope's try_blocks.
                    // Inner scopes are registered first (lower scope_idx) so
                    // we iterate in scope order — inner pops happen before
                    // outer pops (LIFO, matching the setjmp stack order).
                    // Raise does NOT get ExcPopFrame — the runtime unwinds.
                    if let Some(scope_indices) = try_ctx.try_body_blocks.get(hir_id).cloned() {
                        for scope_idx in scope_indices {
                            let scope_try_blocks = &func.try_scopes[scope_idx].try_blocks;
                            let emit_pop = match &hir_block.terminator {
                                hir::HirTerminator::Return(_) => true,
                                hir::HirTerminator::Jump(target) => {
                                    !scope_try_blocks.contains(target)
                                }
                                _ => false,
                            };
                            if emit_pop {
                                self.emit_instruction(mir::InstructionKind::ExcPopFrame);
                            }
                        }
                    }

                    // ── Handler exit: ExcEndHandling + conditional redirect ──
                    // ExcEndHandling clears `handling_exception` (the exception
                    // saved by ExcStartHandling for __context__ chaining and
                    // message recovery). It must only be emitted when the handler
                    // exits NORMALLY (Jump → finally_dispatch, Return from fn).
                    //
                    // For exception-propagating terminators (Raise, Reraise),
                    // DO NOT emit ExcEndHandling:
                    // - Reraise: rt_exc_reraise restores handling_exception →
                    //   clearing it first would leave nothing to re-raise.
                    // - Raise: rt_exc_raise* consumes handling_exception for
                    //   __context__ chaining and message recovery in
                    //   rt_exc_raise_instance; clearing it first loses the message.
                    if let Some(&(scope_idx, _)) = try_ctx.handler_entry_map.get(hir_id) {
                        let finally_dispatch_id = try_ctx.scopes[scope_idx].finally_dispatch_mir_id;
                        match &hir_block.terminator {
                            hir::HirTerminator::Jump(_) => {
                                self.emit_instruction(mir::InstructionKind::ExcEndHandling);
                                self.current_block_mut().terminator =
                                    mir::Terminator::Goto(finally_dispatch_id);
                            }
                            hir::HirTerminator::Reraise | hir::HirTerminator::Raise { .. } => {
                                // Exception propagates — let the raise* runtime consume
                                // handling_exception naturally (for context/message).
                                self.emit_hir_terminator(
                                    &hir_block.terminator,
                                    &hir_to_mir,
                                    hir_module,
                                    mir_func,
                                )?;
                            }
                            _ => {
                                self.emit_instruction(mir::InstructionKind::ExcEndHandling);
                                self.emit_hir_terminator(
                                    &hir_block.terminator,
                                    &hir_to_mir,
                                    hir_module,
                                    mir_func,
                                )?;
                            }
                        }
                    } else if let Some(&scope_idx) = try_ctx.finally_blocks_map.get(hir_id) {
                        let scope = &func.try_scopes[scope_idx];
                        let exits_finally = match &hir_block.terminator {
                            hir::HirTerminator::Jump(target) => {
                                !scope.finally_blocks.contains(target)
                            }
                            _ => false,
                        };
                        if exits_finally {
                            self.current_block_mut().terminator = mir::Terminator::Goto(
                                try_ctx.scopes[scope_idx].finally_body_mir_id,
                            );
                        } else {
                            self.emit_hir_terminator(
                                &hir_block.terminator,
                                &hir_to_mir,
                                hir_module,
                                mir_func,
                            )?;
                        }
                    } else {
                        // Normal terminator emission (with narrowing still active).
                        self.emit_hir_terminator(
                            &hir_block.terminator,
                            &hir_to_mir,
                            hir_module,
                            mir_func,
                        )?;
                    }
                }
            }

            // Pop narrowing frame AFTER terminator emission.
            if narrow.is_some() {
                self.pop_narrowing_frame();
            }
        }

        // ── Step 6: emit try-scope infrastructure blocks ─────────────────
        // Each scope emitted 5 infra blocks in this order:
        //   [handler_dispatch, finally_dispatch, finally_body, reraise, normal_exit]
        let mut infra_iter = infra_blocks.into_iter();
        for (scope_idx, scope) in func.try_scopes.iter().enumerate() {
            let emi = try_ctx.scopes[scope_idx]; // Copy

            let handler_dispatch_blk = infra_iter.next().unwrap();
            let finally_dispatch_blk = infra_iter.next().unwrap();
            let finally_body_blk = infra_iter.next().unwrap();
            let reraise_blk = infra_iter.next().unwrap();
            let normal_exit_blk = infra_iter.next().unwrap();

            // — Handler dispatch block —
            self.push_block(handler_dispatch_blk);
            self.emit_cfg_handler_dispatch(scope, emi, &hir_to_mir, mir_func);

            // — Finally dispatch: enter real finally CFG if present, else
            //   fall through to the synthetic finally decision block. —
            self.push_block(finally_dispatch_blk);
            self.current_block_mut().terminator =
                mir::Terminator::Goto(emi.first_finally_mir_id.unwrap_or(emi.finally_body_mir_id));

            // — Finally body: propagating check after the real finally CFG
            //   has completed (or immediately, for finally-free scopes). —
            self.push_block(finally_body_blk);
            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(emi.propagating_local),
                then_block: emi.reraise_mir_id,
                else_block: emi.normal_exit_mir_id,
            };

            // — Reraise —
            self.push_block(reraise_blk);
            self.current_block_mut().terminator = mir::Terminator::Reraise;

            // — Normal exit: extract cell values, restore nonlocal_cells, Goto(post_bb) —
            self.push_block(normal_exit_blk);
            // Extract cell-wrapped variable values back to regular locals, then
            // restore nonlocal_cells so post-try code accesses vars directly.
            if let Some((saved_cells, cell_locals)) = scope_cell_data.shift_remove(&scope_idx) {
                for (var_id, cell_local) in &cell_locals {
                    let var_type = self.get_var_type(var_id).cloned().unwrap_or(Type::Int);
                    let get_func = self.get_cell_get_func(&var_type);
                    let normal_local =
                        self.get_or_create_local(*var_id, var_type.clone(), mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: normal_local,
                        func: get_func,
                        args: vec![mir::Operand::Local(*cell_local)],
                    });
                }
                self.restore_nonlocal_cells(saved_cells);
            }
            if let Some(post_mir_id) = emi.post_bb_mir_id {
                self.current_block_mut().terminator = mir::Terminator::Goto(post_mir_id);
            } else {
                // Should not happen for well-formed functions; use Return(None).
                self.current_block_mut().terminator =
                    mir::Terminator::Return(Some(mir::Operand::Constant(mir::Constant::None)));
            }

            let _ = scope; // consumed above via emi
        }

        Ok(())
    }

    /// Emit the handler dispatch chain into the current block (which should
    /// already be pushed).  Replicates `build_exception_dispatch` but uses
    /// the TryScope metadata directly instead of a pre-built handler_info Vec.
    fn emit_cfg_handler_dispatch(
        &mut self,
        scope: &hir::TryScope,
        emi: TryScopeEmission,
        hir_to_mir: &IndexMap<HirBlockId, BlockId>,
        mir_func: &mut mir::Function,
    ) {
        if scope.handlers.is_empty() {
            // No handlers: set propagating = true, goto finally.
            self.emit_instruction(mir::InstructionKind::Const {
                dest: emi.propagating_local,
                value: mir::Constant::Bool(true),
            });
            self.current_block_mut().terminator =
                mir::Terminator::Goto(emi.finally_dispatch_mir_id);
            return;
        }

        for (h_idx, handler) in scope.handlers.iter().enumerate() {
            let handler_mir_id = hir_to_mir[&handler.entry_block];
            let (type_tag, _is_custom) = if let Some(ty) = handler.ty.as_ref() {
                get_exc_type_tag_from_type(ty).unwrap_or((0, false))
            } else {
                // Bare except: catch all
                (0xff, false) // sentinel: see below
            };

            let is_bare = handler.ty.is_none()
                || matches!(type_tag, t if t == pyaot_core_defs::BuiltinExceptionKind::BaseException.tag());
            let is_last = h_idx + 1 == scope.handlers.len();

            if is_bare {
                // Bare except (or BaseException): catch all, go to handler directly.
                self.current_block_mut().terminator = mir::Terminator::Goto(handler_mir_id);
                return;
            }

            // Typed handler: emit ExcCheckClass + Branch.
            let check_local = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::ExcCheckClass {
                dest: check_local,
                class_id: type_tag,
            });

            if is_last {
                // Last typed handler: if no match → propagating = true + finally.
                let no_match_bb = self.new_block();
                let no_match_id = no_match_bb.id;
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(check_local),
                    then_block: handler_mir_id,
                    else_block: no_match_id,
                };
                self.push_block(no_match_bb);
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: emi.propagating_local,
                    value: mir::Constant::Bool(true),
                });
                self.current_block_mut().terminator =
                    mir::Terminator::Goto(emi.finally_dispatch_mir_id);
            } else {
                // More handlers follow: create a next-check block.
                let next_check = self.new_block();
                let next_id = next_check.id;
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(check_local),
                    then_block: handler_mir_id,
                    else_block: next_id,
                };
                self.push_block(next_check);
                // Loop continues with the next handler emitting into next_check.
            }
        }
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
                let exc_opt = Some(*exc);
                self.emit_raise_terminator(&exc_opt, cause, hir_module, mir_func)?;
            }
            hir::HirTerminator::Reraise => {
                self.current_block_mut().terminator = mir::Terminator::Reraise;
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
