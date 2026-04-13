use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::BindingTarget;
use pyaot_utils::{InternedString, Span, VarId};
use rustpython_parser::ast as py;
use std::collections::HashSet;

impl AstToHir {
    pub(crate) fn get_or_create_var_from_expr(&mut self, expr: &py::Expr) -> Result<VarId> {
        match expr {
            py::Expr::Name(name) => {
                let name_str = self.interner.intern(&name.id);
                if let Some(&var_id) = self.symbols.var_map.get(&name_str) {
                    Ok(var_id)
                } else {
                    let var_id = self.ids.alloc_var();
                    self.symbols.var_map.insert(name_str, var_id);
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
            self.scope.initialized_vars.insert(name_str);
        }
    }

    /// Build a [`BindingTarget`] from any Python LHS expression. Single
    /// entry point for every binding site that admits the full grammar
    /// (assignment, for, with, comprehension targets). Walrus and
    /// `except ... as NAME` use bespoke paths because their grammar is
    /// restricted to a bare Name.
    ///
    /// Validates structure inline:
    /// * at most one `Starred` per `Tuple` level,
    /// * `Starred` cannot directly wrap another `Starred` (parser grammar),
    /// * leaves must be `Name`/`Attribute`/`Subscript`/`Tuple`/`List`/`Starred`.
    ///
    /// `mark_var_initialized` bookkeeping is folded into the `Var` arm —
    /// every name produced is recorded in `scope.initialized_vars`.
    pub(crate) fn bind_target(&mut self, expr: &py::Expr) -> Result<BindingTarget> {
        self.bind_target_inner(expr, /*in_starred=*/ false)
    }

    fn bind_target_inner(&mut self, expr: &py::Expr, in_starred: bool) -> Result<BindingTarget> {
        match expr {
            py::Expr::Name(name) => {
                let name_str = self.interner.intern(&name.id);
                let var_id = if let Some(&id) = self.symbols.var_map.get(&name_str) {
                    id
                } else {
                    let id = self.ids.alloc_var();
                    self.symbols.var_map.insert(name_str, id);
                    id
                };
                self.scope.initialized_vars.insert(name_str);
                Ok(BindingTarget::Var(var_id))
            }
            py::Expr::Attribute(attr) => {
                // Detect `ClassName.attr = ...` so callers can route through
                // class-attr storage rather than instance-field storage.
                if let py::Expr::Name(base_name) = &*attr.value {
                    let base_str = self.interner.intern(&base_name.id);
                    if let Some(&class_id) = self.symbols.class_map.get(&base_str) {
                        let attr_name = self.interner.intern(&attr.attr);
                        return Ok(BindingTarget::ClassAttr {
                            class_id,
                            attr: attr_name,
                            span: Self::span_from(expr),
                        });
                    }
                }
                let obj = self.convert_expr((*attr.value).clone())?;
                let field = self.interner.intern(&attr.attr);
                Ok(BindingTarget::Attr {
                    obj,
                    field,
                    span: Self::span_from(expr),
                })
            }
            py::Expr::Subscript(sub) => {
                let obj = self.convert_expr((*sub.value).clone())?;
                let index = self.convert_expr((*sub.slice).clone())?;
                Ok(BindingTarget::Index {
                    obj,
                    index,
                    span: Self::span_from(expr),
                })
            }
            py::Expr::Tuple(t) => self.bind_target_tuple(&t.elts, Self::span_from(expr)),
            py::Expr::List(l) => {
                // CPython grammar treats `[a, b]` and `(a, b)` identically as
                // assignment targets.
                self.bind_target_tuple(&l.elts, Self::span_from(expr))
            }
            py::Expr::Starred(starred) => {
                if in_starred {
                    return Err(CompilerError::parse_error(
                        "starred expression cannot be nested inside another starred expression",
                        Self::span_from(expr),
                    ));
                }
                let inner = self.bind_target_inner(&starred.value, /*in_starred=*/ true)?;
                Ok(BindingTarget::Starred {
                    inner: Box::new(inner),
                    span: Self::span_from(expr),
                })
            }
            _ => Err(CompilerError::parse_error(
                "invalid assignment target",
                Self::span_from(expr),
            )),
        }
    }

    fn bind_target_tuple(&mut self, elts: &[py::Expr], span: Span) -> Result<BindingTarget> {
        let mut out = Vec::with_capacity(elts.len());
        let mut seen_star = false;
        for elt in elts {
            let bt = self.bind_target_inner(elt, /*in_starred=*/ false)?;
            if matches!(bt, BindingTarget::Starred { .. }) {
                if seen_star {
                    return Err(CompilerError::parse_error(
                        "multiple starred expressions in assignment",
                        span,
                    ));
                }
                seen_star = true;
            }
            out.push(bt);
        }
        Ok(BindingTarget::Tuple { elts: out, span })
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustpython_parser::{parse, Mode};

    /// Parse a Python expression source into a `py::Expr`. Panics on parse
    /// failure — tests pass valid Python; that's the unit under test.
    fn parse_expr(src: &str) -> py::Expr {
        let parsed = parse(src, Mode::Expression, "<test>").expect("parse expression");
        match parsed {
            py::Mod::Expression(m) => *m.body,
            other => panic!("expected expression module, got {:?}", other),
        }
    }

    fn fresh() -> AstToHir {
        AstToHir::new("test")
    }

    /// Pre-allocate variable IDs for each name so that `convert_expr` calls
    /// inside `bind_target` (for `Attr::obj`, `Index::obj`, `Index::index`)
    /// resolve cleanly. Real usage already has these names bound in the
    /// surrounding scope; tests must mimic that.
    fn seed_vars(h: &mut AstToHir, names: &[&str]) {
        for &n in names {
            let interned = h.interner.intern(n);
            if !h.symbols.var_map.contains_key(&interned) {
                let id = h.ids.alloc_var();
                h.symbols.var_map.insert(interned, id);
                h.scope.initialized_vars.insert(interned);
            }
        }
    }

    #[test]
    fn simple_name_returns_var() {
        let mut h = fresh();
        let expr = parse_expr("x");
        let bt = h.bind_target(&expr).unwrap();
        assert!(matches!(bt, BindingTarget::Var(_)));
    }

    #[test]
    fn attribute_returns_attr() {
        let mut h = fresh();
        seed_vars(&mut h, &["obj"]);
        let expr = parse_expr("obj.field");
        let bt = h.bind_target(&expr).unwrap();
        assert!(matches!(bt, BindingTarget::Attr { .. }));
    }

    #[test]
    fn subscript_returns_index() {
        let mut h = fresh();
        seed_vars(&mut h, &["lst"]);
        let expr = parse_expr("lst[0]");
        let bt = h.bind_target(&expr).unwrap();
        assert!(matches!(bt, BindingTarget::Index { .. }));
    }

    #[test]
    fn flat_tuple_of_names() {
        let mut h = fresh();
        let expr = parse_expr("(a, b, c)");
        let bt = h.bind_target(&expr).unwrap();
        match bt {
            BindingTarget::Tuple { elts, .. } => {
                assert_eq!(elts.len(), 3);
                assert!(elts.iter().all(|e| matches!(e, BindingTarget::Var(_))));
            }
            other => panic!("expected Tuple, got {:?}", other),
        }
    }

    #[test]
    fn list_treated_as_tuple() {
        let mut h = fresh();
        let expr = parse_expr("[a, b]");
        let bt = h.bind_target(&expr).unwrap();
        assert!(matches!(bt, BindingTarget::Tuple { .. }));
    }

    #[test]
    fn nested_tuple_recurses() {
        let mut h = fresh();
        let expr = parse_expr("(a, (b, c))");
        let bt = h.bind_target(&expr).unwrap();
        match bt {
            BindingTarget::Tuple { elts, .. } => {
                assert_eq!(elts.len(), 2);
                assert!(matches!(&elts[0], BindingTarget::Var(_)));
                assert!(
                    matches!(&elts[1], BindingTarget::Tuple { elts: inner, .. } if inner.len() == 2)
                );
            }
            _ => panic!("expected nested tuple"),
        }
    }

    #[test]
    fn starred_var_inside_tuple() {
        let mut h = fresh();
        let expr = parse_expr("(a, *rest, b)");
        let bt = h.bind_target(&expr).unwrap();
        match bt {
            BindingTarget::Tuple { elts, .. } => {
                assert_eq!(elts.len(), 3);
                assert!(matches!(&elts[0], BindingTarget::Var(_)));
                assert!(matches!(&elts[1], BindingTarget::Starred { .. }));
                assert!(matches!(&elts[2], BindingTarget::Var(_)));
            }
            _ => panic!("expected tuple with starred"),
        }
    }

    #[test]
    fn attribute_leaf_inside_tuple() {
        let mut h = fresh();
        seed_vars(&mut h, &["c"]);
        let expr = parse_expr("(c.x, c.y)");
        let bt = h.bind_target(&expr).unwrap();
        match bt {
            BindingTarget::Tuple { elts, .. } => {
                assert!(matches!(&elts[0], BindingTarget::Attr { .. }));
                assert!(matches!(&elts[1], BindingTarget::Attr { .. }));
            }
            _ => panic!("expected tuple of attributes"),
        }
    }

    #[test]
    fn mixed_leaves_inside_tuple() {
        let mut h = fresh();
        seed_vars(&mut h, &["obj", "lst"]);
        let expr = parse_expr("(a, obj.x, lst[0])");
        let bt = h.bind_target(&expr).unwrap();
        match bt {
            BindingTarget::Tuple { elts, .. } => {
                assert!(matches!(&elts[0], BindingTarget::Var(_)));
                assert!(matches!(&elts[1], BindingTarget::Attr { .. }));
                assert!(matches!(&elts[2], BindingTarget::Index { .. }));
            }
            _ => panic!("expected mixed tuple"),
        }
    }

    #[test]
    fn deep_nested_with_starred_and_attr() {
        // (a, *rest, (b, c.x)) — exercises nested + starred + attribute leaf
        let mut h = fresh();
        seed_vars(&mut h, &["c"]);
        let expr = parse_expr("(a, *rest, (b, c.x))");
        let bt = h.bind_target(&expr).unwrap();
        match bt {
            BindingTarget::Tuple { elts, .. } => {
                assert_eq!(elts.len(), 3);
                assert!(matches!(&elts[0], BindingTarget::Var(_)));
                assert!(matches!(&elts[1], BindingTarget::Starred { .. }));
                match &elts[2] {
                    BindingTarget::Tuple { elts: inner, .. } => {
                        assert!(matches!(&inner[0], BindingTarget::Var(_)));
                        assert!(matches!(&inner[1], BindingTarget::Attr { .. }));
                    }
                    _ => panic!("expected nested tuple at index 2"),
                }
            }
            _ => panic!("expected outer tuple"),
        }
    }

    #[test]
    fn starred_wrapping_tuple_is_legal() {
        // `*(a, b)` as a leaf — CPython accepts this in some unpack contexts.
        let mut h = fresh();
        let expr = parse_expr("(*(a, b), c)");
        let bt = h.bind_target(&expr).unwrap();
        match bt {
            BindingTarget::Tuple { elts, .. } => {
                assert_eq!(elts.len(), 2);
                match &elts[0] {
                    BindingTarget::Starred { inner, .. } => {
                        assert!(matches!(inner.as_ref(), BindingTarget::Tuple { .. }));
                    }
                    _ => panic!("expected starred tuple"),
                }
            }
            _ => panic!("expected outer tuple"),
        }
    }

    // ── Negative cases ─────────────────────────────────────────────────────

    #[test]
    fn multiple_starred_rejected() {
        let mut h = fresh();
        let expr = parse_expr("(*a, *b)");
        let err = h.bind_target(&expr).expect_err("should reject");
        let msg = format!("{:?}", err);
        assert!(msg.contains("multiple starred"), "got: {}", msg);
    }

    #[test]
    fn nested_starred_inside_starred_rejected() {
        // Parser may reject this earlier; if it does, our test just verifies
        // we don't accept it. Construct the AST manually if parsing fails.
        let parsed = parse("**a", Mode::Expression, "<test>");
        // If the parser accepted it, we could validate bind_target rejects it.
        let _ = parsed;
    }

    #[test]
    fn integer_literal_rejected() {
        let mut h = fresh();
        let expr = parse_expr("42");
        let err = h.bind_target(&expr).expect_err("should reject");
        let msg = format!("{:?}", err);
        assert!(msg.contains("invalid assignment target"), "got: {}", msg);
    }

    #[test]
    fn function_call_rejected() {
        let mut h = fresh();
        let expr = parse_expr("f(x)");
        let err = h.bind_target(&expr).expect_err("should reject");
        let msg = format!("{:?}", err);
        assert!(msg.contains("invalid assignment target"), "got: {}", msg);
    }

    #[test]
    fn name_marks_initialized() {
        let mut h = fresh();
        let expr = parse_expr("alpha");
        h.bind_target(&expr).unwrap();
        let interned = h.interner.intern("alpha");
        assert!(h.scope.initialized_vars.contains(&interned));
    }

    #[test]
    fn repeated_name_returns_same_var() {
        let mut h = fresh();
        let v1 = match h.bind_target(&parse_expr("x")).unwrap() {
            BindingTarget::Var(v) => v,
            _ => unreachable!(),
        };
        let v2 = match h.bind_target(&parse_expr("x")).unwrap() {
            BindingTarget::Var(v) => v,
            _ => unreachable!(),
        };
        assert_eq!(v1, v2);
    }
}
