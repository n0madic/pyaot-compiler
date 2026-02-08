//! Global and nonlocal variable declarations

use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_global(
        &mut self,
        global_stmt: py::StmtGlobal,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Handle `global var1, var2, ...` - marks variables as module-level
        for name in &global_stmt.names {
            let name_str = self.interner.intern(name.as_str());

            // Mark variable as global in current scope
            self.global_vars.insert(name_str);

            // Ensure variable exists in module_var_map
            if !self.module_var_map.contains_key(&name_str) {
                let var_id = self.alloc_var_id();
                self.module_var_map.insert(name_str, var_id);
            }

            // Map name in var_map to module-level VarId
            let module_var_id = *self
                .module_var_map
                .get(&name_str)
                .expect("internal error: module variable not in module_var_map");
            self.var_map.insert(name_str, module_var_id);

            // Add to module's globals set for lowering phase
            self.module.globals.insert(module_var_id);
        }

        // Return Pass statement (no runtime effect)
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Pass,
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_nonlocal(
        &mut self,
        nonlocal_stmt: py::StmtNonlocal,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Handle `nonlocal var1, var2, ...` - marks variables as from enclosing scope
        for name in &nonlocal_stmt.names {
            let name_str = self.interner.intern(name.as_str());

            // Mark variable as nonlocal in current scope
            self.nonlocal_vars.insert(name_str);

            // If the variable is already mapped (e.g., as a capture parameter in nested functions),
            // don't overwrite it - the capture parameter is the cell we should use
            if self.var_map.contains_key(&name_str) {
                continue;
            }

            // Look up variable in enclosing scopes
            let mut found = false;
            for scope in self.scope_stack.iter().rev() {
                if let Some(&var_id) = scope.get(&name_str) {
                    // Found in enclosing scope - use that VarId
                    self.var_map.insert(name_str, var_id);
                    found = true;
                    break;
                }
            }

            if !found {
                // If not found in scope stack, check module vars
                if let Some(&var_id) = self.module_var_map.get(&name_str) {
                    self.var_map.insert(name_str, var_id);
                }
                // Note: Python would raise SyntaxError if var not in enclosing scope
                // We silently ignore for simplicity - the lowering phase will handle it
            }
        }

        // Return Pass statement (no runtime effect - handled by closure capture mechanism)
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Pass,
            span: stmt_span,
        }))
    }
}
