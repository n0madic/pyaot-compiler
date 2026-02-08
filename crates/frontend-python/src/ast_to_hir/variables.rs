use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::UnpackTarget;
use pyaot_utils::{FuncId, InternedString, Span, VarId};
use rustpython_parser::ast as py;
use std::collections::HashSet;

impl AstToHir {
    pub(crate) fn get_or_create_var_from_expr(&mut self, expr: &py::Expr) -> Result<VarId> {
        match expr {
            py::Expr::Name(name) => {
                let name_str = self.interner.intern(&name.id);
                if let Some(&var_id) = self.var_map.get(&name_str) {
                    Ok(var_id)
                } else {
                    let var_id = self.alloc_var_id();
                    self.var_map.insert(name_str, var_id);
                    Ok(var_id)
                }
            }
            _ => Err(CompilerError::parse_error(
                "Assignment target must be a simple name",
                Self::span_from(expr),
            )),
        }
    }

    /// Mark a variable as initialized in the current scope.
    /// Used for compile-time detection of unbound nonlocal variables.
    pub(crate) fn mark_var_initialized(&mut self, expr: &py::Expr) {
        if let py::Expr::Name(name) = expr {
            let name_str = self.interner.intern(&name.id);
            self.initialized_vars.insert(name_str);
        }
    }

    /// Mark multiple variables as initialized (for tuple unpacking).
    /// Also handles starred expressions like `*rest`.
    pub(crate) fn mark_vars_initialized(&mut self, elts: &[py::Expr]) {
        for elt in elts {
            match elt {
                py::Expr::Starred(starred) => {
                    // Mark the inner variable of the starred expression
                    self.mark_var_initialized(&starred.value);
                }
                _ => {
                    self.mark_var_initialized(elt);
                }
            }
        }
    }

    /// Parse an unpacking pattern with optional starred expression.
    /// Returns (before_star, starred, after_star) where:
    /// - `before_star`: variables before the starred expression
    /// - `starred`: the starred variable (if present)
    /// - `after_star`: variables after the starred expression
    ///
    /// Examples:
    /// - `a, b, c` → `([a, b, c], None, [])`
    /// - `a, *rest` → `([a], Some(rest), [])`
    /// - `*rest, last` → `([], Some(rest), [last])`
    /// - `first, *middle, last` → `([first], Some(middle), [last])`
    pub(crate) fn parse_unpack_pattern(
        &mut self,
        elts: &[py::Expr],
        pattern_span: Span,
    ) -> Result<(Vec<VarId>, Option<VarId>, Vec<VarId>)> {
        let mut before_star = Vec::new();
        let mut starred: Option<VarId> = None;
        let mut after_star = Vec::new();
        let mut found_star = false;

        for elt in elts {
            match elt {
                py::Expr::Starred(starred_expr) => {
                    if found_star {
                        return Err(CompilerError::parse_error(
                            "multiple starred expressions in assignment",
                            Self::span_from(elt),
                        ));
                    }
                    found_star = true;

                    // Extract the inner variable from *rest
                    match &*starred_expr.value {
                        py::Expr::Name(name) => {
                            let name_str = self.interner.intern(&name.id);
                            let var_id = if let Some(&id) = self.var_map.get(&name_str) {
                                id
                            } else {
                                let id = self.alloc_var_id();
                                self.var_map.insert(name_str, id);
                                id
                            };
                            starred = Some(var_id);
                        }
                        _ => {
                            return Err(CompilerError::parse_error(
                                "starred expression must be a simple name",
                                Self::span_from(&*starred_expr.value),
                            ));
                        }
                    }
                }
                py::Expr::Name(name) => {
                    let name_str = self.interner.intern(&name.id);
                    let var_id = if let Some(&id) = self.var_map.get(&name_str) {
                        id
                    } else {
                        let id = self.alloc_var_id();
                        self.var_map.insert(name_str, id);
                        id
                    };
                    if found_star {
                        after_star.push(var_id);
                    } else {
                        before_star.push(var_id);
                    }
                }
                _ => {
                    return Err(CompilerError::parse_error(
                        "nested unpacking patterns not yet supported",
                        pattern_span,
                    ));
                }
            }
        }

        Ok((before_star, starred, after_star))
    }

    /// Parse a nested unpacking pattern like (a, (b, c))
    /// Returns a vector of UnpackTarget which can be Var or Nested
    /// Does NOT support starred expressions in nested contexts
    pub(crate) fn parse_nested_unpack_pattern(
        &mut self,
        elts: &[py::Expr],
    ) -> Result<Vec<UnpackTarget>> {
        let mut targets = Vec::new();

        for elt in elts {
            match elt {
                // Starred expressions not allowed in nested patterns
                py::Expr::Starred(_) => {
                    return Err(CompilerError::parse_error(
                        "starred expression not allowed in nested unpacking pattern",
                        Self::span_from(elt),
                    ));
                }
                // Simple name - create a Var target
                py::Expr::Name(name) => {
                    let name_str = self.interner.intern(&name.id);
                    let var_id = if let Some(&id) = self.var_map.get(&name_str) {
                        id
                    } else {
                        let id = self.alloc_var_id();
                        self.var_map.insert(name_str, id);
                        id
                    };
                    targets.push(UnpackTarget::Var(var_id));
                }
                // Tuple or List - recursively parse as nested pattern
                py::Expr::Tuple(tuple) => {
                    let nested_targets = self.parse_nested_unpack_pattern(&tuple.elts)?;
                    targets.push(UnpackTarget::Nested(nested_targets));
                }
                py::Expr::List(list) => {
                    let nested_targets = self.parse_nested_unpack_pattern(&list.elts)?;
                    targets.push(UnpackTarget::Nested(nested_targets));
                }
                _ => {
                    return Err(CompilerError::parse_error(
                        "invalid target in nested unpacking pattern",
                        Self::span_from(elt),
                    ));
                }
            }
        }

        Ok(targets)
    }

    pub(crate) fn alloc_var_id(&mut self) -> VarId {
        let id = VarId::new(self.next_var_id);
        self.next_var_id += 1;
        id
    }

    pub(crate) fn alloc_func_id(&mut self) -> FuncId {
        let id = FuncId::new(self.next_func_id);
        self.next_func_id += 1;
        id
    }

    /// Scan a function body for `nonlocal` declarations and return the set of variable names.
    /// This is used to identify which outer variables need to be wrapped in cells.
    pub(crate) fn scan_for_nonlocal_declarations(
        &mut self,
        body: &[py::Stmt],
    ) -> HashSet<InternedString> {
        let mut nonlocal_names = HashSet::new();
        for stmt in body {
            self.collect_nonlocal_names_from_stmt(stmt, &mut nonlocal_names);
        }
        nonlocal_names
    }

    /// Recursively collect nonlocal variable names from a statement
    fn collect_nonlocal_names_from_stmt(
        &mut self,
        stmt: &py::Stmt,
        nonlocal_names: &mut HashSet<InternedString>,
    ) {
        match stmt {
            py::Stmt::Nonlocal(nonlocal_stmt) => {
                for name in &nonlocal_stmt.names {
                    let name_str = self.interner.intern(name.as_str());
                    nonlocal_names.insert(name_str);
                }
            }
            py::Stmt::If(if_stmt) => {
                for stmt in &if_stmt.body {
                    self.collect_nonlocal_names_from_stmt(stmt, nonlocal_names);
                }
                for stmt in &if_stmt.orelse {
                    self.collect_nonlocal_names_from_stmt(stmt, nonlocal_names);
                }
            }
            py::Stmt::While(while_stmt) => {
                for stmt in &while_stmt.body {
                    self.collect_nonlocal_names_from_stmt(stmt, nonlocal_names);
                }
            }
            py::Stmt::For(for_stmt) => {
                for stmt in &for_stmt.body {
                    self.collect_nonlocal_names_from_stmt(stmt, nonlocal_names);
                }
            }
            py::Stmt::Try(try_stmt) => {
                for stmt in &try_stmt.body {
                    self.collect_nonlocal_names_from_stmt(stmt, nonlocal_names);
                }
                for handler in &try_stmt.handlers {
                    let py::ExceptHandler::ExceptHandler(h) = handler;
                    for stmt in &h.body {
                        self.collect_nonlocal_names_from_stmt(stmt, nonlocal_names);
                    }
                }
                for stmt in &try_stmt.finalbody {
                    self.collect_nonlocal_names_from_stmt(stmt, nonlocal_names);
                }
            }
            // Note: We don't recurse into FunctionDef because nonlocal in a nested-nested
            // function refers to that nested function's enclosing scope, not our scope
            _ => {}
        }
    }
}
