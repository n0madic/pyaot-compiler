//! Match statement conversion from Python AST to HIR
//!
//! Converts Python 3.10+ match statements to HIR Match nodes.
//! Pattern matching is preserved in HIR and lowered to if/elif chains in MIR.

use crate::ast_to_hir::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{cfg_builder::CfgMatchCase, cfg_builder::CfgStmt, *};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Convert a match statement to HIR
    pub(crate) fn convert_match(
        &mut self,
        match_stmt: py::StmtMatch,
        stmt_span: Span,
    ) -> Result<CfgStmt> {
        // Convert the subject expression
        let subject = self.convert_expr(*match_stmt.subject)?;

        // Convert each case
        let mut cases = Vec::with_capacity(match_stmt.cases.len());
        for case in match_stmt.cases {
            let hir_case = self.convert_match_case(case)?;
            cases.push(hir_case);
        }

        Ok(CfgStmt::Match {
            subject,
            cases,
            span: stmt_span,
        })
    }

    /// Convert a match case to HIR
    fn convert_match_case(&mut self, case: py::MatchCase) -> Result<CfgMatchCase> {
        // Collect pattern bindings first (creates variables in scope)
        self.collect_pattern_bindings(&case.pattern)?;

        // Convert the pattern
        let pattern = self.convert_pattern(case.pattern)?;

        // Convert the optional guard
        let guard = if let Some(guard_expr) = case.guard {
            Some(self.convert_expr(*guard_expr)?)
        } else {
            None
        };

        // Convert the body statements
        let mut body = Vec::with_capacity(case.body.len());
        for stmt in case.body {
            let stmt = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            body.extend(pending);
            body.push(stmt);
        }

        Ok(CfgMatchCase {
            pattern,
            guard,
            body,
        })
    }

    /// Convert a pattern from Python AST to HIR
    fn convert_pattern(&mut self, pattern: py::Pattern) -> Result<Pattern> {
        match pattern {
            py::Pattern::MatchValue(mv) => {
                let value = self.convert_expr(*mv.value)?;
                Ok(Pattern::MatchValue(value))
            }

            py::Pattern::MatchSingleton(ms) => {
                let kind = match &ms.value {
                    py::Constant::Bool(true) => MatchSingletonKind::True,
                    py::Constant::Bool(false) => MatchSingletonKind::False,
                    py::Constant::None => MatchSingletonKind::None,
                    _ => {
                        return Err(CompilerError::parse_error(
                            format!(
                                "Invalid singleton pattern: expected True, False, or None, got {:?}",
                                ms.value
                            ),
                            Self::span_from(&ms),
                        ));
                    }
                };
                Ok(Pattern::MatchSingleton(kind))
            }

            py::Pattern::MatchAs(ma) => {
                // Convert the optional inner pattern
                let inner_pattern = if let Some(pat) = ma.pattern {
                    Some(Box::new(self.convert_pattern(*pat)?))
                } else {
                    None
                };

                // Get or create variable for the name binding
                let name = if let Some(ident) = ma.name {
                    let name_str = self.interner.intern(ident.as_str());
                    let var_id = if let Some(&id) = self.symbols.var_map.get(&name_str) {
                        id
                    } else {
                        let id = self.ids.alloc_var();
                        self.symbols.var_map.insert(name_str, id);
                        id
                    };
                    Some(var_id)
                } else {
                    None
                };

                Ok(Pattern::MatchAs {
                    pattern: inner_pattern,
                    name,
                })
            }

            py::Pattern::MatchSequence(ms) => {
                let mut patterns = Vec::with_capacity(ms.patterns.len());
                for pat in ms.patterns {
                    patterns.push(self.convert_pattern(pat)?);
                }
                Ok(Pattern::MatchSequence { patterns })
            }

            py::Pattern::MatchStar(ms) => {
                let name = if let Some(ident) = ms.name {
                    let name_str = self.interner.intern(ident.as_str());
                    let var_id = if let Some(&id) = self.symbols.var_map.get(&name_str) {
                        id
                    } else {
                        let id = self.ids.alloc_var();
                        self.symbols.var_map.insert(name_str, id);
                        id
                    };
                    Some(var_id)
                } else {
                    None
                };
                Ok(Pattern::MatchStar(name))
            }

            py::Pattern::MatchOr(mo) => {
                let mut patterns = Vec::with_capacity(mo.patterns.len());
                for pat in mo.patterns {
                    patterns.push(self.convert_pattern(pat)?);
                }
                Ok(Pattern::MatchOr(patterns))
            }

            py::Pattern::MatchMapping(mm) => {
                // Convert keys
                let mut keys = Vec::with_capacity(mm.keys.len());
                for key in mm.keys {
                    keys.push(self.convert_expr(key)?);
                }

                // Convert patterns
                let mut patterns = Vec::with_capacity(mm.patterns.len());
                for pat in mm.patterns {
                    patterns.push(self.convert_pattern(pat)?);
                }

                // Handle rest binding (**rest)
                let rest = if let Some(ident) = mm.rest {
                    let name_str = self.interner.intern(ident.as_str());
                    let var_id = if let Some(&id) = self.symbols.var_map.get(&name_str) {
                        id
                    } else {
                        let id = self.ids.alloc_var();
                        self.symbols.var_map.insert(name_str, id);
                        id
                    };
                    Some(var_id)
                } else {
                    None
                };

                Ok(Pattern::MatchMapping {
                    keys,
                    patterns,
                    rest,
                })
            }

            py::Pattern::MatchClass(mc) => {
                // Convert the class reference expression
                let cls = self.convert_expr(*mc.cls)?;

                // Convert positional patterns
                let mut patterns = Vec::with_capacity(mc.patterns.len());
                for pat in mc.patterns {
                    patterns.push(self.convert_pattern(pat)?);
                }

                // Convert keyword attribute names
                let mut kwd_attrs = Vec::with_capacity(mc.kwd_attrs.len());
                for attr in mc.kwd_attrs {
                    kwd_attrs.push(self.interner.intern(attr.as_str()));
                }

                // Convert keyword patterns
                let mut kwd_patterns = Vec::with_capacity(mc.kwd_patterns.len());
                for pat in mc.kwd_patterns {
                    kwd_patterns.push(self.convert_pattern(pat)?);
                }

                Ok(Pattern::MatchClass {
                    cls,
                    patterns,
                    kwd_attrs,
                    kwd_patterns,
                })
            }
        }
    }

    /// Collect all variable bindings from a pattern (for scope tracking)
    fn collect_pattern_bindings(&mut self, pattern: &py::Pattern) -> Result<()> {
        match pattern {
            py::Pattern::MatchValue(_) | py::Pattern::MatchSingleton(_) => {
                // No bindings
            }

            py::Pattern::MatchAs(ma) => {
                // Recurse into inner pattern if present
                if let Some(ref inner) = ma.pattern {
                    self.collect_pattern_bindings(inner)?;
                }
                // Register the name binding
                if let Some(ref ident) = ma.name {
                    let name_str = self.interner.intern(ident.as_str());
                    if !self.symbols.var_map.contains_key(&name_str) {
                        let var_id = self.ids.alloc_var();
                        self.symbols.var_map.insert(name_str, var_id);
                    }
                }
            }

            py::Pattern::MatchSequence(ms) => {
                for pat in &ms.patterns {
                    self.collect_pattern_bindings(pat)?;
                }
            }

            py::Pattern::MatchStar(ms) => {
                if let Some(ref ident) = ms.name {
                    let name_str = self.interner.intern(ident.as_str());
                    if !self.symbols.var_map.contains_key(&name_str) {
                        let var_id = self.ids.alloc_var();
                        self.symbols.var_map.insert(name_str, var_id);
                    }
                }
            }

            py::Pattern::MatchOr(mo) => {
                // All branches must bind the same variables
                // We only need to collect from the first branch
                if let Some(first) = mo.patterns.first() {
                    self.collect_pattern_bindings(first)?;
                }
            }

            py::Pattern::MatchMapping(mm) => {
                for pat in &mm.patterns {
                    self.collect_pattern_bindings(pat)?;
                }
                if let Some(ref ident) = mm.rest {
                    let name_str = self.interner.intern(ident.as_str());
                    if !self.symbols.var_map.contains_key(&name_str) {
                        let var_id = self.ids.alloc_var();
                        self.symbols.var_map.insert(name_str, var_id);
                    }
                }
            }

            py::Pattern::MatchClass(mc) => {
                for pat in &mc.patterns {
                    self.collect_pattern_bindings(pat)?;
                }
                for pat in &mc.kwd_patterns {
                    self.collect_pattern_bindings(pat)?;
                }
            }
        }
        Ok(())
    }
}
