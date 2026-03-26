//! Semantic analysis: scope validation, control flow checking
//!
//! This module performs semantic checks on the HIR that cannot be done
//! during parsing, including:
//! - break/continue must be inside loops
//! - bare raise must be inside except handlers
//! - cannot instantiate abstract classes

#![forbid(unsafe_code)]

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{CallArg, ExprId, ExprKind, Module, StmtId, StmtKind};
use pyaot_utils::StringInterner;

/// Semantic analyzer for HIR
pub struct SemanticAnalyzer<'a> {
    /// Current loop nesting depth (0 = not in a loop)
    loop_depth: usize,
    /// Current except handler nesting depth (0 = not in an except)
    except_depth: usize,
    /// String interner for resolving symbol names
    interner: &'a StringInterner,
}

impl<'a> SemanticAnalyzer<'a> {
    pub fn new(interner: &'a StringInterner) -> Self {
        Self {
            loop_depth: 0,
            except_depth: 0,
            interner,
        }
    }

    /// Analyze a module for semantic errors
    pub fn analyze(&mut self, module: &Module) -> Result<()> {
        // Analyze all function bodies
        for func in module.func_defs.values() {
            self.loop_depth = 0;
            self.except_depth = 0;
            self.analyze_stmts(&func.body, module)?;
        }

        // Analyze module-level statements
        self.loop_depth = 0;
        self.except_depth = 0;
        self.analyze_stmts(&module.module_init_stmts, module)?;

        Ok(())
    }

    /// Analyze a list of statements
    fn analyze_stmts(&mut self, stmts: &[StmtId], module: &Module) -> Result<()> {
        for &stmt_id in stmts {
            self.analyze_stmt(stmt_id, module)?;
        }
        Ok(())
    }

    /// Analyze a single statement
    fn analyze_stmt(&mut self, stmt_id: StmtId, module: &Module) -> Result<()> {
        let stmt = &module.stmts[stmt_id];
        let span = stmt.span;

        match &stmt.kind {
            StmtKind::Break => {
                if self.loop_depth == 0 {
                    return Err(CompilerError::semantic_error("'break' outside loop", span));
                }
            }

            StmtKind::Continue => {
                if self.loop_depth == 0 {
                    return Err(CompilerError::semantic_error(
                        "'continue' not properly in loop",
                        span,
                    ));
                }
            }

            StmtKind::Raise { exc, cause } => {
                // Bare raise (raise without argument) must be inside except handler
                if exc.is_none() && self.except_depth == 0 {
                    return Err(CompilerError::semantic_error(
                        "bare 'raise' not inside an exception handler",
                        span,
                    ));
                }
                // Check the expression if present
                if let Some(expr_id) = exc {
                    self.analyze_expr(*expr_id, module)?;
                }
                // Check the cause expression if present
                if let Some(cause_id) = cause {
                    self.analyze_expr(*cause_id, module)?;
                }
            }

            StmtKind::While {
                cond,
                body,
                else_block,
            } => {
                self.analyze_expr(*cond, module)?;
                self.loop_depth += 1;
                self.analyze_stmts(body, module)?;
                self.loop_depth -= 1;
                self.analyze_stmts(else_block, module)?;
            }

            StmtKind::For {
                iter,
                body,
                else_block,
                ..
            }
            | StmtKind::ForUnpack {
                iter,
                body,
                else_block,
                ..
            }
            | StmtKind::ForUnpackStarred {
                iter,
                body,
                else_block,
                ..
            } => {
                self.analyze_expr(*iter, module)?;
                self.loop_depth += 1;
                self.analyze_stmts(body, module)?;
                self.loop_depth -= 1;
                self.analyze_stmts(else_block, module)?;
            }

            StmtKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.analyze_expr(*cond, module)?;
                self.analyze_stmts(then_block, module)?;
                self.analyze_stmts(else_block, module)?;
            }

            StmtKind::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                // Analyze try body (not in except handler)
                self.analyze_stmts(body, module)?;

                // Analyze except handlers (in except context)
                for handler in handlers {
                    self.except_depth += 1;
                    self.analyze_stmts(&handler.body, module)?;
                    self.except_depth -= 1;
                }

                // Analyze else block (not in except handler)
                self.analyze_stmts(else_block, module)?;

                // Analyze finally block (in except context — CPython allows
                // bare raise in finally when an exception is active)
                self.except_depth += 1;
                self.analyze_stmts(finally_block, module)?;
                self.except_depth -= 1;
            }

            StmtKind::Expr(expr_id) => {
                self.analyze_expr(*expr_id, module)?;
            }

            StmtKind::Assign { value, .. } => {
                self.analyze_expr(*value, module)?;
            }

            StmtKind::UnpackAssign { value, .. } => {
                // The new fields (before_star, starred, after_star) don't need
                // semantic analysis - they're just variable IDs
                self.analyze_expr(*value, module)?;
            }

            StmtKind::NestedUnpackAssign { value, .. } => {
                // Nested unpacking - targets are just variable IDs
                self.analyze_expr(*value, module)?;
            }

            StmtKind::Return(expr) => {
                if let Some(expr_id) = expr {
                    self.analyze_expr(*expr_id, module)?;
                }
            }

            StmtKind::Assert { cond, msg } => {
                self.analyze_expr(*cond, module)?;
                if let Some(msg_id) = msg {
                    self.analyze_expr(*msg_id, module)?;
                }
            }

            StmtKind::IndexAssign { obj, index, value } => {
                self.analyze_expr(*obj, module)?;
                self.analyze_expr(*index, module)?;
                self.analyze_expr(*value, module)?;
            }

            StmtKind::FieldAssign { obj, value, .. } => {
                self.analyze_expr(*obj, module)?;
                self.analyze_expr(*value, module)?;
            }

            StmtKind::ClassAttrAssign { value, .. } => {
                self.analyze_expr(*value, module)?;
            }

            StmtKind::Match { subject, cases } => {
                self.analyze_expr(*subject, module)?;
                for case in cases {
                    if let Some(guard) = case.guard {
                        self.analyze_expr(guard, module)?;
                    }
                    self.analyze_stmts(&case.body, module)?;
                }
            }

            StmtKind::IndexDelete { obj, index } => {
                self.analyze_expr(*obj, module)?;
                self.analyze_expr(*index, module)?;
            }

            StmtKind::Pass => {}
        }

        Ok(())
    }

    /// Analyze an expression (recursively check nested expressions)
    fn analyze_expr(&mut self, expr_id: ExprId, module: &Module) -> Result<()> {
        let expr = &module.exprs[expr_id];

        match &expr.kind {
            ExprKind::BinOp { left, right, .. } => {
                self.analyze_expr(*left, module)?;
                self.analyze_expr(*right, module)?;
            }

            ExprKind::UnOp { operand, .. } => {
                self.analyze_expr(*operand, module)?;
            }

            ExprKind::Compare { left, right, .. } => {
                self.analyze_expr(*left, module)?;
                self.analyze_expr(*right, module)?;
            }

            ExprKind::LogicalOp { left, right, .. } => {
                self.analyze_expr(*left, module)?;
                self.analyze_expr(*right, module)?;
            }

            ExprKind::Call {
                func,
                args,
                kwargs,
                kwargs_unpack,
            } => {
                // Check if this is an attempt to instantiate an abstract class
                let func_expr = &module.exprs[*func];
                if let ExprKind::ClassRef(class_id) = &func_expr.kind {
                    if let Some(class_def) = module.class_defs.get(class_id) {
                        if !class_def.abstract_methods.is_empty() {
                            // Collect abstract method names for the error message
                            let method_names: Vec<_> = class_def
                                .abstract_methods
                                .iter()
                                .map(|name| self.interner.resolve(*name).to_string())
                                .collect();
                            let class_name = self.interner.resolve(class_def.name);
                            return Err(CompilerError::semantic_error(
                                format!(
                                    "Cannot instantiate abstract class '{}' with unimplemented methods: [{}]",
                                    class_name,
                                    method_names.join(", ")
                                ),
                                expr.span,
                            ));
                        }
                    }
                }

                self.analyze_expr(*func, module)?;
                for arg in args {
                    match arg {
                        CallArg::Regular(expr_id) => self.analyze_expr(*expr_id, module)?,
                        CallArg::Starred(expr_id) => self.analyze_expr(*expr_id, module)?,
                    }
                }
                for kwarg in kwargs {
                    self.analyze_expr(kwarg.value, module)?;
                }
                if let Some(kwargs_expr) = kwargs_unpack {
                    self.analyze_expr(*kwargs_expr, module)?;
                }
            }

            ExprKind::BuiltinCall { args, kwargs, .. } => {
                for arg in args {
                    self.analyze_expr(*arg, module)?;
                }
                for kwarg in kwargs {
                    self.analyze_expr(kwarg.value, module)?;
                }
            }

            ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                self.analyze_expr(*cond, module)?;
                self.analyze_expr(*then_val, module)?;
                self.analyze_expr(*else_val, module)?;
            }

            ExprKind::List(items) => {
                for item in items {
                    self.analyze_expr(*item, module)?;
                }
            }

            ExprKind::Tuple(items) => {
                for item in items {
                    self.analyze_expr(*item, module)?;
                }
            }

            ExprKind::Dict(pairs) => {
                for (key, value) in pairs {
                    self.analyze_expr(*key, module)?;
                    self.analyze_expr(*value, module)?;
                }
            }

            ExprKind::Set(items) => {
                for item in items {
                    self.analyze_expr(*item, module)?;
                }
            }

            ExprKind::Index { obj, index } => {
                self.analyze_expr(*obj, module)?;
                self.analyze_expr(*index, module)?;
            }

            ExprKind::Slice {
                obj,
                start,
                end,
                step,
            } => {
                self.analyze_expr(*obj, module)?;
                if let Some(s) = start {
                    self.analyze_expr(*s, module)?;
                }
                if let Some(e) = end {
                    self.analyze_expr(*e, module)?;
                }
                if let Some(st) = step {
                    self.analyze_expr(*st, module)?;
                }
            }

            ExprKind::MethodCall {
                obj, args, kwargs, ..
            } => {
                self.analyze_expr(*obj, module)?;
                for arg in args {
                    self.analyze_expr(*arg, module)?;
                }
                for kwarg in kwargs {
                    self.analyze_expr(kwarg.value, module)?;
                }
            }

            ExprKind::Attribute { obj, .. } => {
                self.analyze_expr(*obj, module)?;
            }

            ExprKind::Closure { captures, .. } => {
                // Analyze captured variable expressions
                for capture in captures {
                    self.analyze_expr(*capture, module)?;
                }
            }

            ExprKind::Yield(value) => {
                // Analyze yielded value if present
                if let Some(val) = value {
                    self.analyze_expr(*val, module)?;
                }
            }

            ExprKind::SuperCall { args, .. } => {
                // Analyze super() call arguments
                for arg in args {
                    self.analyze_expr(*arg, module)?;
                }
            }

            ExprKind::StdlibCall { args, .. } => {
                // Analyze stdlib call arguments
                for arg in args {
                    self.analyze_expr(*arg, module)?;
                }
            }

            // Leaf expressions - no nested expressions to analyze
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Bytes(_)
            | ExprKind::None
            | ExprKind::Var(_)
            | ExprKind::FuncRef(_)
            | ExprKind::ClassRef(_)
            | ExprKind::ClassAttrRef { .. }
            | ExprKind::TypeRef(_)
            | ExprKind::ImportedRef { .. }
            | ExprKind::ModuleAttr { .. }
            | ExprKind::StdlibAttr(_)
            | ExprKind::StdlibConst(_)
            | ExprKind::BuiltinRef(_)
            | ExprKind::ExcCurrentValue => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
