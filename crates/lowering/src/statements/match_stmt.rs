//! Match statement lowering from HIR to MIR
//!
//! Desugars match statements into if/elif chains. Each match case is converted into
//! a conditional check that tests whether the pattern matches, binds any captured
//! variables, and executes the case body if the pattern matches.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

/// Result type for pattern check: (condition_operand, bindings)
/// Bindings are (VarId, Operand, Type) tuples to be assigned
type PatternCheckResult = (mir::Operand, Vec<(VarId, mir::Operand, Type)>);

/// Context for pattern checking, grouping common parameters
struct PatternContext<'a> {
    subject: mir::Operand,
    subject_type: &'a Type,
    hir_module: &'a hir::Module,
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
        let subject_type = self.get_expr_type(subject_expr, hir_module);

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

        // Create else block (next case) — shared by both guard and no-guard paths
        let else_bb = self.new_block();
        let else_id = else_bb.id;

        if let Some(guard_expr_id) = case.guard {
            // Two-stage branch: first check pattern, then emit bindings, then check guard.
            // This ensures guard expressions can reference captured pattern variables.
            let bindings_bb = self.new_block();
            let bindings_id = bindings_bb.id;

            // Stage 1: branch on pattern match → bindings block or next case
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

            // Stage 2: branch on guard → case body or next case
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

    /// Generate code to check if a pattern matches and collect variable bindings.
    /// Returns (condition_operand, bindings) where bindings are (VarId, Operand, Type).
    fn generate_pattern_check(
        &mut self,
        pattern: &hir::Pattern,
        subject: mir::Operand,
        subject_type: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<PatternCheckResult> {
        match pattern {
            hir::Pattern::MatchValue(expr_id) => {
                // Compare subject to the value
                let value_expr = &hir_module.exprs[*expr_id];
                let value_operand = self.lower_expr(value_expr, hir_module, mir_func)?;
                let cond =
                    self.emit_equality_check(subject, value_operand, subject_type, mir_func)?;
                Ok((cond, Vec::new()))
            }

            hir::Pattern::MatchSingleton(kind) => {
                // Compare to singleton values
                let cond = match kind {
                    hir::MatchSingletonKind::True => {
                        // subject == True
                        let true_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::Const {
                            dest: true_local,
                            value: mir::Constant::Bool(true),
                        });
                        // For Union/Any subjects, use object comparison to handle boxed values
                        let cmp_type = match subject_type {
                            Type::Union(_) | Type::Any => subject_type,
                            _ => &Type::Bool,
                        };
                        self.emit_equality_check(
                            subject,
                            mir::Operand::Local(true_local),
                            cmp_type,
                            mir_func,
                        )?
                    }
                    hir::MatchSingletonKind::False => {
                        // subject == False
                        let false_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::Const {
                            dest: false_local,
                            value: mir::Constant::Bool(false),
                        });
                        let cmp_type = match subject_type {
                            Type::Union(_) | Type::Any => subject_type,
                            _ => &Type::Bool,
                        };
                        self.emit_equality_check(
                            subject,
                            mir::Operand::Local(false_local),
                            cmp_type,
                            mir_func,
                        )?
                    }
                    hir::MatchSingletonKind::None => {
                        // For None, check if subject is None
                        // Use pointer comparison with null (None is represented as 0/null)
                        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                        let null_local = self.alloc_and_add_local(Type::None, mir_func);
                        self.emit_instruction(mir::InstructionKind::Const {
                            dest: null_local,
                            value: mir::Constant::None,
                        });
                        self.emit_instruction(mir::InstructionKind::BinOp {
                            dest: result_local,
                            op: mir::BinOp::Eq,
                            left: subject,
                            right: mir::Operand::Local(null_local),
                        });
                        mir::Operand::Local(result_local)
                    }
                };
                Ok((cond, Vec::new()))
            }

            hir::Pattern::MatchAs { pattern, name } => {
                // If there's an inner pattern, check it first
                let (cond, mut bindings) = if let Some(inner) = pattern {
                    self.generate_pattern_check(
                        inner,
                        subject.clone(),
                        subject_type,
                        hir_module,
                        mir_func,
                    )?
                } else {
                    // No inner pattern - always matches
                    let true_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: true_local,
                        value: mir::Constant::Bool(true),
                    });
                    (mir::Operand::Local(true_local), Vec::new())
                };

                // Bind the name to the subject
                if let Some(var_id) = name {
                    bindings.push((*var_id, subject, subject_type.clone()));
                }

                Ok((cond, bindings))
            }

            hir::Pattern::MatchOr(patterns) => {
                // Or pattern: try each alternative in order; bindings come from
                // whichever alternative actually matched.
                if patterns.is_empty() {
                    let false_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: false_local,
                        value: mir::Constant::Bool(false),
                    });
                    return Ok((mir::Operand::Local(false_local), Vec::new()));
                }

                // Pre-allocate result local (init false)
                let or_result_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: or_result_local,
                    value: mir::Constant::Bool(false),
                });

                // Check first alternative to discover binding names/types
                let (first_cond, first_bindings) = self.generate_pattern_check(
                    &patterns[0],
                    subject.clone(),
                    subject_type,
                    hir_module,
                    mir_func,
                )?;

                // Pre-allocate shared binding locals for all alternatives
                let binding_locals: Vec<(pyaot_utils::VarId, pyaot_utils::LocalId, Type)> =
                    first_bindings
                        .iter()
                        .map(|(var_id, _, ty)| {
                            let local = self.alloc_and_add_local(ty.clone(), mir_func);
                            (*var_id, local, ty.clone())
                        })
                        .collect();

                // Create the final merge block
                let merge_bb = self.new_block();
                let merge_bb_id = merge_bb.id;

                // First alternative: if it matches, write bindings and jump to merge
                let write_bb = self.new_block();
                let next_bb = self.new_block();
                let write_bb_id = write_bb.id;
                let next_bb_id = next_bb.id;

                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: first_cond,
                    then_block: write_bb_id,
                    else_block: next_bb_id,
                };

                self.push_block(write_bb);
                for (i, (_, value, _)) in first_bindings.iter().enumerate() {
                    if i < binding_locals.len() {
                        self.emit_instruction(mir::InstructionKind::Copy {
                            dest: binding_locals[i].1,
                            src: value.clone(),
                        });
                    }
                }
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: or_result_local,
                    value: mir::Constant::Bool(true),
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb_id);

                // Remaining alternatives
                let mut prev_else_bb = next_bb;
                for pattern in &patterns[1..] {
                    self.push_block(prev_else_bb);
                    let (alt_cond, alt_bindings) = self.generate_pattern_check(
                        pattern,
                        subject.clone(),
                        subject_type,
                        hir_module,
                        mir_func,
                    )?;

                    let alt_write_bb = self.new_block();
                    let alt_next_bb = self.new_block();
                    let alt_write_bb_id = alt_write_bb.id;
                    let alt_next_bb_id = alt_next_bb.id;

                    self.current_block_mut().terminator = mir::Terminator::Branch {
                        cond: alt_cond,
                        then_block: alt_write_bb_id,
                        else_block: alt_next_bb_id,
                    };

                    self.push_block(alt_write_bb);
                    for (i, (_, value, _)) in alt_bindings.iter().enumerate() {
                        if i < binding_locals.len() {
                            self.emit_instruction(mir::InstructionKind::Copy {
                                dest: binding_locals[i].1,
                                src: value.clone(),
                            });
                        }
                    }
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: or_result_local,
                        value: mir::Constant::Bool(true),
                    });
                    self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb_id);

                    prev_else_bb = alt_next_bb;
                }

                // Last else: no alternative matched, jump to merge (or_result_local stays false)
                self.push_block(prev_else_bb);
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb_id);

                self.push_block(merge_bb);

                let final_bindings: Vec<(pyaot_utils::VarId, mir::Operand, Type)> = binding_locals
                    .iter()
                    .map(|(var_id, local, ty)| (*var_id, mir::Operand::Local(*local), ty.clone()))
                    .collect();

                Ok((mir::Operand::Local(or_result_local), final_bindings))
            }

            hir::Pattern::MatchSequence { patterns } => self.generate_sequence_pattern_check(
                patterns,
                subject,
                subject_type,
                hir_module,
                mir_func,
            ),

            hir::Pattern::MatchStar(name) => {
                // MatchStar is only valid inside a sequence pattern
                // If we get here directly, it's always true (captures remaining elements)
                let true_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: true_local,
                    value: mir::Constant::Bool(true),
                });

                let bindings = if let Some(var_id) = name {
                    // The actual binding happens in sequence pattern handling
                    vec![(*var_id, subject, subject_type.clone())]
                } else {
                    Vec::new()
                };

                Ok((mir::Operand::Local(true_local), bindings))
            }

            hir::Pattern::MatchMapping {
                keys,
                patterns,
                rest,
            } => {
                let ctx = PatternContext {
                    subject,
                    subject_type,
                    hir_module,
                };
                self.generate_mapping_pattern_check(keys, patterns, rest.as_ref(), &ctx, mir_func)
            }

            hir::Pattern::MatchClass {
                cls,
                patterns,
                kwd_attrs,
                kwd_patterns,
            } => {
                let ctx = PatternContext {
                    subject,
                    subject_type,
                    hir_module,
                };
                self.generate_class_pattern_check(
                    *cls,
                    patterns,
                    kwd_attrs,
                    kwd_patterns,
                    &ctx,
                    mir_func,
                )
            }
        }
    }

    /// Generate code to check a sequence pattern (list or tuple)
    fn generate_sequence_pattern_check(
        &mut self,
        patterns: &[hir::Pattern],
        subject: mir::Operand,
        subject_type: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<PatternCheckResult> {
        let mut bindings = Vec::new();

        // Find star pattern index if any
        let star_index = patterns
            .iter()
            .position(|p| matches!(p, hir::Pattern::MatchStar(_)));

        // Determine element type (used as a fallback for non-tuple types and for
        // tuple elements whose index exceeds the statically-known length)
        let elem_type = match subject_type {
            Type::List(elem) => (**elem).clone(),
            Type::Tuple(elems) if !elems.is_empty() => elems[0].clone(),
            _ => Type::Any,
        };

        // Get length check function
        let len_func = match subject_type {
            Type::List(_) => mir::RuntimeFunc::ListLen,
            Type::Tuple(_) => mir::RuntimeFunc::TupleLen,
            _ => mir::RuntimeFunc::ListLen,
        };

        // Get element access function
        let get_func = match subject_type {
            Type::List(_) => mir::RuntimeFunc::ListGet,
            Type::Tuple(_) => mir::RuntimeFunc::TupleGet,
            _ => mir::RuntimeFunc::ListGet,
        };

        // Check length
        let len_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: len_local,
            func: len_func,
            args: vec![subject.clone()],
        });

        // Without star: exact length match
        // With star: minimum length check
        let expected_len = if star_index.is_some() {
            patterns.len() - 1 // Exclude the star pattern itself
        } else {
            patterns.len()
        };

        let expected_len_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Const {
            dest: expected_len_local,
            value: mir::Constant::Int(expected_len as i64),
        });

        let len_check_local = self.alloc_and_add_local(Type::Bool, mir_func);
        let len_op = if star_index.is_some() {
            mir::BinOp::GtE // len >= expected (minimum)
        } else {
            mir::BinOp::Eq // len == expected (exact)
        };

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: len_check_local,
            op: len_op,
            left: mir::Operand::Local(len_local),
            right: mir::Operand::Local(expected_len_local),
        });

        // Pre-allocate result local for the overall sequence check.
        // Initialized to false; set to the combined element check result only
        // inside the elements block (where the length check passed).
        let sequence_result_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::Const {
            dest: sequence_result_local,
            value: mir::Constant::Bool(false),
        });

        // Short-circuit: only access elements if the length check passed.
        // Without this, ListGet/TupleGet at out-of-bounds indices would crash.
        let elements_bb = self.new_block();
        let skip_bb = self.new_block();
        let elements_bb_id = elements_bb.id;
        let skip_bb_id = skip_bb.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(len_check_local),
            then_block: elements_bb_id,
            else_block: skip_bb_id,
        };

        // Elements block: length is verified, safe to access elements
        self.push_block(elements_bb);

        let mut result_cond = mir::Operand::Local(len_check_local);

        // Process patterns before star
        let before_star = star_index.unwrap_or(patterns.len());
        for (i, pattern) in patterns.iter().take(before_star).enumerate() {
            // Get the correct element type for this index position.
            // For tuples we use the statically-known per-position type; for
            // all other sequence types (list, unknown) we fall back to the
            // uniform elem_type computed above.
            let idx_elem_type = match subject_type {
                Type::Tuple(elems) if !elems.is_empty() => {
                    elems.get(i).cloned().unwrap_or_else(|| elems[0].clone())
                }
                _ => elem_type.clone(),
            };

            // Get element at index i
            let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::Const {
                dest: idx_local,
                value: mir::Constant::Int(i as i64),
            });

            let elem_local = self.alloc_and_add_local(idx_elem_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: elem_local,
                func: get_func,
                args: vec![subject.clone(), mir::Operand::Local(idx_local)],
            });

            // Check pattern against element
            let (elem_cond, elem_bindings) = self.generate_pattern_check(
                pattern,
                mir::Operand::Local(elem_local),
                &idx_elem_type,
                hir_module,
                mir_func,
            )?;

            bindings.extend(elem_bindings);

            // Combine with AND
            let combined_local = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: combined_local,
                op: mir::BinOp::And,
                left: result_cond,
                right: elem_cond,
            });
            result_cond = mir::Operand::Local(combined_local);
        }

        // Handle star pattern if present
        if let Some(star_idx) = star_index {
            if let hir::Pattern::MatchStar(opt_name) = &patterns[star_idx] {
                if let Some(var_id) = opt_name {
                    // Slice from star_idx to len - after_count
                    let after_count = patterns.len() - star_idx - 1;

                    let start_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: start_local,
                        value: mir::Constant::Int(star_idx as i64),
                    });

                    let after_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: after_local,
                        value: mir::Constant::Int(after_count as i64),
                    });

                    let end_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: end_local,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(len_local),
                        right: mir::Operand::Local(after_local),
                    });

                    // Create slice for starred variable
                    let slice_func = match subject_type {
                        Type::List(_) => mir::RuntimeFunc::ListSlice,
                        Type::Tuple(_) => mir::RuntimeFunc::TupleSliceToList,
                        _ => mir::RuntimeFunc::ListSlice,
                    };

                    let star_elem_type = Type::List(Box::new(elem_type.clone()));
                    let slice_local = self.alloc_and_add_local(star_elem_type.clone(), mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: slice_local,
                        func: slice_func,
                        args: vec![
                            subject.clone(),
                            mir::Operand::Local(start_local),
                            mir::Operand::Local(end_local),
                        ],
                    });

                    bindings.push((*var_id, mir::Operand::Local(slice_local), star_elem_type));
                }

                // Process patterns after star
                let after_star_patterns = &patterns[star_idx + 1..];
                for (i, pattern) in after_star_patterns.iter().enumerate() {
                    // Index from end: len - after_count + i
                    let after_count = after_star_patterns.len();
                    let offset = after_count - 1 - i;

                    let offset_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: offset_local,
                        value: mir::Constant::Int(offset as i64),
                    });

                    let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: idx_local,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(len_local),
                        right: mir::Operand::Local(offset_local),
                    });

                    // Subtract 1 more because offset is 0-indexed from end
                    let one_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: one_local,
                        value: mir::Constant::Int(1),
                    });

                    let final_idx_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: final_idx_local,
                        op: mir::BinOp::Sub,
                        left: mir::Operand::Local(idx_local),
                        right: mir::Operand::Local(one_local),
                    });

                    let elem_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: elem_local,
                        func: get_func,
                        args: vec![subject.clone(), mir::Operand::Local(final_idx_local)],
                    });

                    // Check pattern against element
                    let (elem_cond, elem_bindings) = self.generate_pattern_check(
                        pattern,
                        mir::Operand::Local(elem_local),
                        &elem_type,
                        hir_module,
                        mir_func,
                    )?;

                    bindings.extend(elem_bindings);

                    // Combine with AND
                    let combined_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: combined_local,
                        op: mir::BinOp::And,
                        left: result_cond,
                        right: elem_cond,
                    });
                    result_cond = mir::Operand::Local(combined_local);
                }
            }
        }

        // Write combined result into sequence_result_local and jump to merge
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: sequence_result_local,
            src: result_cond,
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(skip_bb_id);

        // Skip block: merge point for both paths.
        // On false path, sequence_result_local remains false (pre-initialized).
        // On true path, it holds the combined element check result.
        self.push_block(skip_bb);

        Ok((mir::Operand::Local(sequence_result_local), bindings))
    }

    /// Generate code to check a mapping pattern (dict)
    fn generate_mapping_pattern_check(
        &mut self,
        keys: &[hir::ExprId],
        patterns: &[hir::Pattern],
        rest: Option<&VarId>,
        ctx: &PatternContext<'_>,
        mir_func: &mut mir::Function,
    ) -> Result<PatternCheckResult> {
        let mut bindings = Vec::new();

        // Determine value type from dict type
        let value_type = match ctx.subject_type {
            Type::Dict(_, v) => (**v).clone(),
            _ => Type::Any,
        };

        // Start with true condition
        let true_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::Const {
            dest: true_local,
            value: mir::Constant::Bool(true),
        });
        let mut result_cond = mir::Operand::Local(true_local);

        // Check each key-pattern pair with short-circuit branching:
        // If DictContains returns false, skip DictGet and set condition to false.
        for (key_expr_id, pattern) in keys.iter().zip(patterns.iter()) {
            let key_expr = &ctx.hir_module.exprs[*key_expr_id];
            let key_operand = self.lower_expr(key_expr, ctx.hir_module, mir_func)?;

            // Check if key exists using DictContains
            let contains_local = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: contains_local,
                func: mir::RuntimeFunc::DictContains,
                args: vec![ctx.subject.clone(), key_operand.clone()],
            });

            // Pre-allocate combined_local and initialize to current result_cond.
            // This ensures the value is valid on both true and false paths.
            let combined_local = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: combined_local,
                src: result_cond.clone(),
            });

            // Branch: if key exists, get value; otherwise skip to merge with false
            let get_bb = self.new_block();
            let merge_bb = self.new_block();
            let get_bb_id = get_bb.id;
            let merge_bb_id = merge_bb.id;

            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(contains_local),
                then_block: get_bb_id,
                else_block: merge_bb_id,
            };

            // True path: key exists, get value and check sub-pattern
            self.push_block(get_bb);

            let value_local = self.alloc_and_add_local(value_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: value_local,
                func: mir::RuntimeFunc::DictGet,
                args: vec![ctx.subject.clone(), key_operand],
            });

            let (pattern_cond, pattern_bindings) = self.generate_pattern_check(
                pattern,
                mir::Operand::Local(value_local),
                &value_type,
                ctx.hir_module,
                mir_func,
            )?;

            bindings.extend(pattern_bindings);

            // Overwrite combined_local with AND(result_cond, pattern_cond) on true path
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: combined_local,
                op: mir::BinOp::And,
                left: result_cond.clone(),
                right: pattern_cond,
            });

            // Jump to merge
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb_id);

            // Merge block: combined_local is valid on both paths
            // (pre-initialized on false path, overwritten on true path)
            self.push_block(merge_bb);

            // AND combined_local with contains_local to produce false on the false path
            let final_local = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: final_local,
                op: mir::BinOp::And,
                left: mir::Operand::Local(combined_local),
                right: mir::Operand::Local(contains_local),
            });
            result_cond = mir::Operand::Local(final_local);
        }

        // TODO: **rest should bind a new dict excluding already-matched keys.
        // Currently binds the full dict.  A correct implementation would call a
        // runtime helper that copies the subject dict and removes the matched keys,
        // leaving only the unmatched entries in the **rest binding.
        if let Some(rest_var) = rest {
            bindings.push((*rest_var, ctx.subject.clone(), ctx.subject_type.clone()));
        }

        Ok((result_cond, bindings))
    }

    /// Generate code to check a class pattern
    fn generate_class_pattern_check(
        &mut self,
        cls_expr_id: hir::ExprId,
        patterns: &[hir::Pattern],
        kwd_attrs: &[pyaot_utils::InternedString],
        kwd_patterns: &[hir::Pattern],
        ctx: &PatternContext<'_>,
        mir_func: &mut mir::Function,
    ) -> Result<PatternCheckResult> {
        let mut bindings = Vec::new();

        // Get class ID from expression
        let cls_expr = &ctx.hir_module.exprs[cls_expr_id];
        let class_id = match &cls_expr.kind {
            hir::ExprKind::ClassRef(id) => Some(*id),
            _ => None,
        };

        // isinstance check
        let isinstance_local = self.alloc_and_add_local(Type::Bool, mir_func);

        if let Some(class_id) = class_id {
            // Get class name for isinstance check
            if self.has_class(&class_id) {
                // Emit isinstance check using RuntimeFunc
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: isinstance_local,
                    func: mir::RuntimeFunc::IsinstanceClass,
                    args: vec![
                        ctx.subject.clone(),
                        mir::Operand::Constant(mir::Constant::Int(class_id.index() as i64)),
                    ],
                });
            } else {
                // Class info not found - assume false
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: isinstance_local,
                    value: mir::Constant::Bool(false),
                });
            }
        } else {
            // Not a class reference - assume false
            self.emit_instruction(mir::InstructionKind::Const {
                dest: isinstance_local,
                value: mir::Constant::Bool(false),
            });
        }

        let mut result_cond = mir::Operand::Local(isinstance_local);

        // Check keyword attribute patterns
        for (attr_name, pattern) in kwd_attrs.iter().zip(kwd_patterns.iter()) {
            // Get attribute value
            let attr_type = if let Some(class_id) = class_id {
                if let Some(class_info) = self.get_class_info(&class_id) {
                    class_info
                        .field_types
                        .get(attr_name)
                        .cloned()
                        .unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            } else {
                Type::Any
            };

            let attr_local = self.alloc_and_add_local(attr_type.clone(), mir_func);

            if let Some(class_id) = class_id {
                if let Some(class_info) = self.get_class_info(&class_id) {
                    if let Some(&offset) = class_info.field_offsets.get(attr_name) {
                        // Get field at offset
                        let offset_local = self.alloc_and_add_local(Type::Int, mir_func);
                        self.emit_instruction(mir::InstructionKind::Const {
                            dest: offset_local,
                            value: mir::Constant::Int(offset as i64),
                        });

                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: attr_local,
                            func: mir::RuntimeFunc::InstanceGetField,
                            args: vec![ctx.subject.clone(), mir::Operand::Local(offset_local)],
                        });
                    }
                }
            }

            // Check pattern against attribute
            let (attr_cond, attr_bindings) = self.generate_pattern_check(
                pattern,
                mir::Operand::Local(attr_local),
                &attr_type,
                ctx.hir_module,
                mir_func,
            )?;

            bindings.extend(attr_bindings);

            // Combine with AND
            let combined_local = self.alloc_and_add_local(Type::Bool, mir_func);
            self.emit_instruction(mir::InstructionKind::BinOp {
                dest: combined_local,
                op: mir::BinOp::And,
                left: result_cond,
                right: attr_cond,
            });
            result_cond = mir::Operand::Local(combined_local);
        }

        // Positional class patterns require __match_args__ support, which is not
        // yet implemented.  Emit an error instead of silently ignoring sub-patterns.
        if !patterns.is_empty() {
            return Err(pyaot_diagnostics::CompilerError::codegen_error(
                "positional class patterns are not yet supported (__match_args__ required)",
            ));
        }

        Ok((result_cond, bindings))
    }

    /// Emit an equality check between two operands
    fn emit_equality_check(
        &mut self,
        left: mir::Operand,
        right: mir::Operand,
        ty: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        match ty {
            Type::Str => {
                // String comparison
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Compare {
                        kind: mir::CompareKind::Str,
                        op: mir::ComparisonOp::Eq,
                    },
                    args: vec![left, right],
                });
            }
            Type::Int | Type::Bool | Type::Float => {
                // Primitive comparison
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir::BinOp::Eq,
                    left,
                    right,
                });
            }
            _ => {
                // For other types, use object equality
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Compare {
                        kind: mir::CompareKind::Obj,
                        op: mir::ComparisonOp::Eq,
                    },
                    args: vec![left, right],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Bind pattern variables to the subject (for wildcard/as patterns)
    fn bind_pattern_variables(
        &mut self,
        pattern: &hir::Pattern,
        subject: mir::Operand,
        subject_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        match pattern {
            hir::Pattern::MatchAs { pattern, name } => {
                // Recursively bind inner pattern
                if let Some(inner) = pattern {
                    self.bind_pattern_variables(inner, subject.clone(), subject_type, mir_func)?;
                }

                // Bind name to subject
                if let Some(var_id) = name {
                    let local = self.get_or_create_local_for_var(*var_id, mir_func, subject_type);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: local,
                        src: subject,
                    });
                }
            }
            _ => {
                // Other patterns don't need direct binding here
                // (handled in generate_pattern_check)
            }
        }
        Ok(())
    }

    /// Get or create a local for a variable
    fn get_or_create_local_for_var(
        &mut self,
        var_id: VarId,
        mir_func: &mut mir::Function,
        ty: &Type,
    ) -> pyaot_utils::LocalId {
        if let Some(local) = self.get_var_local(&var_id) {
            local
        } else {
            let local = self.alloc_and_add_local(ty.clone(), mir_func);
            self.insert_var_local(var_id, local);
            self.insert_var_type(var_id, ty.clone());
            local
        }
    }
}
