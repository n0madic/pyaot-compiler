//! Match statement lowering from HIR to MIR
//!
//! Desugars match statements into if/elif chains. Each match case is converted into
//! a conditional check that tests whether the pattern matches, binds any captured
//! variables, and executes the case body if the pattern matches.
//!
//! Split into focused submodules:
//! - `patterns`: Pattern check generation (sequence, mapping, class, or, value)
//! - `binding`: Equality checks and variable binding helpers

mod binding;
mod patterns;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

/// Result type for pattern check: (condition_operand, bindings)
/// Bindings are (VarId, Operand, Type) tuples to be assigned
pub(crate) type PatternCheckResult = (mir::Operand, Vec<(pyaot_utils::VarId, mir::Operand, Type)>);

/// Context for pattern checking, grouping common parameters
pub(super) struct PatternContext<'a> {
    pub(super) subject: mir::Operand,
    pub(super) subject_type: &'a Type,
    pub(super) hir_module: &'a hir::Module,
}

impl<'a> Lowering<'a> {
    /// Lower a match statement by desugaring to if/elif chains.
    ///
    /// The subject is evaluated once and stored in a temporary. Each case is converted
    /// into a conditional check: if the pattern matches, bind variables and execute body.
    pub(crate) fn lower_match(
        &mut self,
        subject: hir::ExprId,
        cases: &[hir::MatchCase],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        if cases.is_empty() {
            return Ok(());
        }

        // Evaluate subject once and store in a temporary local
        let subject_expr = &hir_module.exprs[subject];
        let subject_operand = self.lower_expr(subject_expr, hir_module, mir_func)?;
        let subject_type = self.get_type_of_expr_id(subject, hir_module);

        // Store subject in a local to avoid re-evaluation
        let subject_local = self.alloc_and_add_local(subject_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: subject_local,
            src: subject_operand,
        });

        // Create exit block for after all cases
        let exit_bb = self.new_block();
        let exit_id = exit_bb.id;

        // Lower each case as a chained if/else
        self.lower_match_cases(
            cases,
            mir::Operand::Local(subject_local),
            &subject_type,
            exit_id,
            hir_module,
            mir_func,
        )?;

        // Add exit block
        self.push_block(exit_bb);

        Ok(())
    }

    /// Lower a sequence of match cases as chained if/else statements
    fn lower_match_cases(
        &mut self,
        cases: &[hir::MatchCase],
        subject: mir::Operand,
        subject_type: &Type,
        exit_id: pyaot_utils::BlockId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        if cases.is_empty() {
            // No more cases - jump to exit
            self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
            return Ok(());
        }

        let case = &cases[0];
        let remaining = &cases[1..];

        // Check if this is a wildcard pattern (matches everything)
        if self.is_wildcard_pattern(&case.pattern) && case.guard.is_none() {
            // Wildcard always matches - execute body and exit
            self.bind_pattern_variables(&case.pattern, subject.clone(), subject_type, mir_func)?;

            for stmt_id in &case.body {
                let stmt = &hir_module.stmts[*stmt_id];
                self.lower_stmt(stmt, hir_module, mir_func)?;
            }

            if !self.current_block_has_terminator() {
                self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
            }
            return Ok(());
        }

        // Generate pattern check condition
        let (cond_operand, bindings) = self.generate_pattern_check(
            &case.pattern,
            subject.clone(),
            subject_type,
            hir_module,
            mir_func,
        )?;

        // Create else block (next case) --- shared by both guard and no-guard paths
        let else_bb = self.new_block();
        let else_id = else_bb.id;

        if let Some(guard_expr_id) = case.guard {
            // Two-stage branch: first check pattern, then emit bindings, then check guard.
            // This ensures guard expressions can reference captured pattern variables.
            let bindings_bb = self.new_block();
            let bindings_id = bindings_bb.id;

            // Stage 1: branch on pattern match -> bindings block or next case
            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: cond_operand,
                then_block: bindings_id,
                else_block: else_id,
            };

            // Bindings block: emit bindings, then evaluate guard
            self.push_block(bindings_bb);
            for (var_id, value, ty) in &bindings {
                let local = self.get_or_create_local_for_var(*var_id, mir_func, ty);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: local,
                    src: value.clone(),
                });
            }

            // Evaluate guard (now pattern variables are bound)
            let guard_expr = &hir_module.exprs[guard_expr_id];
            let guard_operand = self.lower_expr(guard_expr, hir_module, mir_func)?;

            // Stage 2: branch on guard -> case body or next case
            let body_bb = self.new_block();
            let body_id = body_bb.id;
            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: guard_operand,
                then_block: body_id,
                else_block: else_id,
            };

            // Case body block
            self.push_block(body_bb);
        } else {
            // No guard: single-stage branch with bindings in the then-block
            let then_bb = self.new_block();
            let then_id = then_bb.id;

            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: cond_operand,
                then_block: then_id,
                else_block: else_id,
            };

            self.push_block(then_bb);

            // Apply bindings inside the then-block (only on match success)
            for (var_id, value, ty) in &bindings {
                let local = self.get_or_create_local_for_var(*var_id, mir_func, ty);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: local,
                    src: value.clone(),
                });
            }
        }

        // Execute case body (in whichever block we ended up in)
        for stmt_id in &case.body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
        }

        // Else block: try next case
        self.push_block(else_bb);

        // Continue with remaining cases
        self.lower_match_cases(
            remaining,
            subject,
            subject_type,
            exit_id,
            hir_module,
            mir_func,
        )
    }

    /// Check if a pattern is a wildcard (matches everything)
    fn is_wildcard_pattern(&self, pattern: &hir::Pattern) -> bool {
        matches!(pattern, hir::Pattern::MatchAs { pattern: None, .. })
    }
}
