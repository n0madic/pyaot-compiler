//! Control flow statement lowering
//!
//! Handles: Return, If, While, Break, Continue

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a return statement
    pub(crate) fn lower_return(
        &mut self,
        value_expr: Option<&hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let return_operand = if let Some(expr_id) = value_expr {
            let expr = &hir_module.exprs[*expr_id];
            // Type check: validate return value against function return type
            if let Some(ref ret_ty) = self.symbols.current_func_return_type.clone() {
                self.check_expr_type(*expr_id, ret_ty, hir_module);
            }
            // Bidirectional: propagate function return type into expression
            let expected = self.symbols.current_func_return_type.clone();
            let operand = self.lower_expr_expecting(expr, expected, hir_module, mir_func)?;
            Some(operand)
        } else {
            None
        };
        self.current_block_mut().terminator = mir::Terminator::Return(return_operand);
        Ok(())
    }

    /// Lower an if statement with type narrowing support.
    ///
    /// When the condition contains isinstance() checks on Union-typed variables,
    /// the compiler narrows the type in the appropriate branch:
    /// - In the then-branch: variable is narrowed to the checked type
    /// - In the else-branch: variable is narrowed to exclude the checked type
    ///
    /// Example:
    /// ```python
    /// x: int | str = get_value()
    /// if isinstance(x, int):
    ///     print(x + 1)      # x is narrowed to int
    /// else:
    ///     print(x.upper())  # x is narrowed to str
    /// ```
    pub(crate) fn lower_if(
        &mut self,
        cond: hir::ExprId,
        then_block: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let cond_expr = &hir_module.exprs[cond];

        // Check for dead code in isinstance checks and emit warnings
        if let Some(isinstance_info) = self.extract_isinstance_info_from_expr(cond_expr, hir_module)
        {
            if let Some(dead_branch) = isinstance_info.dead_branch {
                // Get variable name by searching:
                // 1. module_var_map (for module-level variables)
                // 2. Function parameters (for local function parameters)
                // 3. Fall back to a placeholder if not found
                let var_name = isinstance_info.var_name.clone().unwrap_or_else(|| {
                    // Try module_var_map first
                    if let Some((name, _)) = hir_module
                        .module_var_map
                        .iter()
                        .find(|(_, &vid)| vid == isinstance_info.var_id)
                    {
                        return self.resolve(*name).to_string();
                    }

                    // Try function parameters (search all functions)
                    for func in hir_module.func_defs.values() {
                        for param in &func.params {
                            if param.var == isinstance_info.var_id {
                                return self.resolve(param.name).to_string();
                            }
                        }
                    }

                    // Fall back to placeholder
                    format!("var_{}", isinstance_info.var_id.0)
                });

                self.emit_dead_code_warning(
                    isinstance_info.span,
                    &var_name,
                    &isinstance_info.checked_type,
                    dead_branch,
                );
            }
        }

        // Analyze condition for type narrowing BEFORE lowering
        let narrowing = self.analyze_condition_for_narrowing(cond_expr, hir_module);

        // Get the condition expression type to check if we need truthiness conversion
        let cond_type = self.get_type_of_expr_id(cond, hir_module);

        // Lower the condition expression
        let cond_operand = self.lower_expr(cond_expr, hir_module, mir_func)?;

        // If the condition is a Union type (or other heap type), we need to emit
        // a call to rt_is_truthy to convert the pointer to a boolean
        let final_cond_operand =
            self.emit_truthiness_conversion_if_needed(cond_operand, &cond_type, mir_func);

        let then_bb = self.new_block();
        let else_bb = self.new_block();
        let merge_bb = self.new_block();

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: final_cond_operand,
            then_block: then_bb.id,
            else_block: else_bb.id,
        };

        // Then block - apply then-narrowings
        self.push_block(then_bb);
        let then_saved = self.apply_narrowings(&narrowing.then_narrowings);
        for stmt_id in then_block {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }
        self.restore_types(then_saved);
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);
        }

        // Else block - apply else-narrowings
        self.push_block(else_bb);
        let else_saved = self.apply_narrowings(&narrowing.else_narrowings);
        for stmt_id in else_block {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }
        self.restore_types(else_saved);
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);
        }

        // Merge block - types are already restored
        self.push_block(merge_bb);

        Ok(())
    }

    /// Lower a while loop with type narrowing support.
    ///
    /// When the condition is a Union-typed variable (e.g., `while x:` where `x: int | None`),
    /// the body narrows `x` to exclude falsy types like None.
    pub(crate) fn lower_while(
        &mut self,
        cond: hir::ExprId,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let has_else = !else_block.is_empty();

        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let else_bb = if has_else {
            Some(self.new_block())
        } else {
            None
        };
        let exit_bb = self.new_block();

        // Save block IDs before moving
        let header_id = header_bb.id;
        let body_id = body_bb.id;
        let else_id = else_bb.as_ref().map(|b| b.id);
        let exit_id = exit_bb.id;

        // The else block runs when the loop condition becomes false (no break)
        // break jumps directly to exit_bb (skipping else)
        let normal_exit_id = else_id.unwrap_or(exit_id);

        // Analyze condition for type narrowing BEFORE lowering
        let cond_expr = &hir_module.exprs[cond];
        let narrowing = self.analyze_condition_for_narrowing(cond_expr, hir_module);

        // Jump to header to evaluate condition
        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Header block: evaluate condition and branch
        self.push_block(header_bb);
        let cond_type = self.get_expr_type(cond_expr, hir_module);
        let cond_operand = self.lower_expr(cond_expr, hir_module, mir_func)?;

        // If the condition is a Union type (or other heap type), we need to emit
        // a call to rt_is_truthy to convert the pointer to a boolean
        let final_cond_operand =
            self.emit_truthiness_conversion_if_needed(cond_operand, &cond_type, mir_func);

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: final_cond_operand,
            then_block: body_id,
            else_block: normal_exit_id,
        };

        // Body block: execute statements and loop back
        self.push_block(body_bb);

        // Apply then-narrowings for the loop body (condition is truthy)
        let body_saved = self.apply_narrowings(&narrowing.then_narrowings);

        // Push loop context: break jumps to exit (skipping else), continue goes to header
        self.push_loop(header_id, exit_id);

        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }

        // Pop loop context
        self.pop_loop();

        // Restore types before going back to header
        self.restore_types(body_saved);

        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(header_id);
        }

        // Else block: execute if loop completed without break
        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            for stmt_id in else_block {
                let stmt = &hir_module.stmts[*stmt_id];
                self.lower_stmt(stmt, hir_module, mir_func)?;
            }
            if !self.current_block_has_terminator() {
                self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
            }
        }

        // Exit block: continue after loop
        self.push_block(exit_bb);

        Ok(())
    }

    /// Lower a break statement
    pub(crate) fn lower_break(&mut self) {
        // Jump to the exit block of the innermost loop
        if let Some((_continue_target, break_target)) = self.current_loop() {
            self.current_block_mut().terminator = mir::Terminator::Goto(break_target);
        } else {
            panic!("internal error: break outside loop should be caught by semantic analysis");
        }
    }

    /// Lower a continue statement
    pub(crate) fn lower_continue(&mut self) {
        // Jump to the header block of the innermost loop
        if let Some((continue_target, _break_target)) = self.current_loop() {
            self.current_block_mut().terminator = mir::Terminator::Goto(continue_target);
        } else {
            panic!("internal error: continue outside loop should be caught by semantic analysis");
        }
    }

    /// Convert a condition to boolean for use in if/while/assert branch conditions.
    ///
    /// Delegates to `convert_to_bool()` which handles all types correctly:
    /// Bool → as-is, Int → !=0, Float → !=0.0, Str/List/Dict/Tuple/Set → len>0,
    /// None → false, Union/Any → rt_is_truthy().
    pub(crate) fn emit_truthiness_conversion_if_needed(
        &mut self,
        cond_operand: mir::Operand,
        cond_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        if matches!(cond_type, Type::Bool) {
            cond_operand
        } else {
            self.convert_to_bool(cond_operand, cond_type, mir_func)
        }
    }
}
