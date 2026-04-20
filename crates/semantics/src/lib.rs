//! Semantic analysis: scope validation, control flow checking
//!
//! This module performs semantic checks on the HIR that cannot be done
//! during parsing, including:
//! - break/continue must be inside loops
//! - bare raise must be inside except handlers
//! - cannot instantiate abstract classes
//!
//! ## CFG-based analysis (§1.11 S1.17b-e, 2026-04-19)
//!
//! The analyzer walks `Function::blocks` (the CFG) directly. `loop_depth`
//! and `handler_depth` are read from `HirBlock.loop_depth` /
//! `HirBlock.handler_depth`, populated by `cfg_builder`. No recursive
//! tree descent remains.

#![forbid(unsafe_code)]

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{BindingTarget, CallArg, ExprId, ExprKind, Function, Module, StmtId, StmtKind};
use pyaot_utils::StringInterner;

/// Semantic analyzer for HIR
pub struct SemanticAnalyzer<'a> {
    /// String interner for resolving symbol names
    interner: &'a StringInterner,
}

impl<'a> SemanticAnalyzer<'a> {
    pub fn new(interner: &'a StringInterner) -> Self {
        Self { interner }
    }

    /// Analyze a module for semantic errors
    pub fn analyze(&mut self, module: &Module) -> Result<()> {
        // Analyze all function bodies via their CFG, including the synthetic
        // module-init function when present.
        for func in module.func_defs.values() {
            self.analyze_function_cfg(func, module)?;
        }

        Ok(())
    }

    /// Walk every block of a function's CFG, checking each statement. The
    /// bridge has already computed per-block `loop_depth` / `handler_depth`,
    /// so we look them up directly instead of tracking with counters.
    fn analyze_function_cfg(&mut self, func: &Function, module: &Module) -> Result<()> {
        for block in func.blocks.values() {
            for &stmt_id in &block.stmts {
                self.analyze_stmt_flat(stmt_id, module, block.loop_depth, block.handler_depth)?;
            }
            // Analyze the terminator expressions (cond / return value / raise
            // exc+cause / yield value). The terminator's control flow itself
            // is structural — nothing to check — but the embedded exprs need
            // the usual recursive analyzer pass.
            self.analyze_terminator(
                &block.terminator,
                module,
                block.loop_depth,
                block.handler_depth,
            )?;
        }
        Ok(())
    }

    /// Analyze a single straight-line statement inside a HIR CFG block.
    fn analyze_stmt_flat(
        &mut self,
        stmt_id: StmtId,
        module: &Module,
        loop_depth: u8,
        handler_depth: u8,
    ) -> Result<()> {
        let stmt = &module.stmts[stmt_id];
        let span = stmt.span;

        match &stmt.kind {
            StmtKind::Break => {
                if loop_depth == 0 {
                    return Err(CompilerError::semantic_error("'break' outside loop", span));
                }
            }

            StmtKind::Continue => {
                if loop_depth == 0 {
                    return Err(CompilerError::semantic_error(
                        "'continue' not properly in loop",
                        span,
                    ));
                }
            }

            StmtKind::Raise { exc, cause } => {
                // Bare raise must be inside except handler (or finally).
                if exc.is_none() && handler_depth == 0 {
                    return Err(CompilerError::semantic_error(
                        "bare 'raise' not inside an exception handler",
                        span,
                    ));
                }
                if let Some(expr_id) = exc {
                    self.analyze_expr(*expr_id, module)?;
                }
                if let Some(cause_id) = cause {
                    self.analyze_expr(*cause_id, module)?;
                }
            }

            StmtKind::Expr(expr_id) => {
                self.analyze_expr(*expr_id, module)?;
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

            StmtKind::IndexDelete { obj, index } => {
                self.analyze_expr(*obj, module)?;
                self.analyze_expr(*index, module)?;
            }

            StmtKind::Pass => {}

            StmtKind::Bind { target, value, .. } => {
                self.analyze_binding_target(target, module)?;
                self.analyze_expr(*value, module)?;
            }

            StmtKind::IterAdvance { iter, target } => {
                self.analyze_expr(*iter, module)?;
                self.analyze_binding_target(target, module)?;
            }

            StmtKind::IterSetup { iter } => {
                self.analyze_expr(*iter, module)?;
            }
        }

        Ok(())
    }

    /// Analyze the expression embedded in a `HirTerminator`, if any.
    fn analyze_terminator(
        &mut self,
        term: &pyaot_hir::HirTerminator,
        module: &Module,
        loop_depth: u8,
        handler_depth: u8,
    ) -> Result<()> {
        use pyaot_hir::HirTerminator::*;
        match term {
            Jump(_) | Unreachable | Reraise => {}
            Branch { cond, .. } => self.analyze_expr(*cond, module)?,
            Return(Some(expr_id)) => self.analyze_expr(*expr_id, module)?,
            Return(None) => {}
            Raise { exc, cause } => {
                // A Raise terminator always has `exc`; bare raise becomes
                // `Unreachable` in the bridge. No depth check needed here.
                let _ = (loop_depth, handler_depth);
                self.analyze_expr(*exc, module)?;
                if let Some(c) = cause {
                    self.analyze_expr(*c, module)?;
                }
            }
            Yield { value, .. } => self.analyze_expr(*value, module)?,
        }
        Ok(())
    }

    /// Analyze a binding target — recursively walk into expressions embedded
    /// in `Attr`/`Index` targets, and into nested `Tuple`/`Starred` patterns.
    /// `Var` and `ClassAttr` carry no nested expressions.
    fn analyze_binding_target(&mut self, target: &BindingTarget, module: &Module) -> Result<()> {
        match target {
            BindingTarget::Var(_) | BindingTarget::ClassAttr { .. } => {}
            BindingTarget::Attr { obj, .. } => {
                self.analyze_expr(*obj, module)?;
            }
            BindingTarget::Index { obj, index, .. } => {
                self.analyze_expr(*obj, module)?;
                self.analyze_expr(*index, module)?;
            }
            BindingTarget::Tuple { elts, .. } => {
                for elt in elts {
                    self.analyze_binding_target(elt, module)?;
                }
            }
            BindingTarget::Starred { inner, .. } => {
                self.analyze_binding_target(inner, module)?;
            }
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

            ExprKind::List(items) | ExprKind::Tuple(items) | ExprKind::Set(items) => {
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
                for capture in captures {
                    self.analyze_expr(*capture, module)?;
                }
            }

            ExprKind::Yield(value) => {
                if let Some(val) = value {
                    self.analyze_expr(*val, module)?;
                }
            }

            ExprKind::SuperCall { args, .. } => {
                for arg in args {
                    self.analyze_expr(*arg, module)?;
                }
            }

            ExprKind::StdlibCall { args, .. } => {
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
            | ExprKind::NotImplemented
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
            | ExprKind::ExcCurrentValue
            | ExprKind::GeneratorIntrinsic(_) => {}

            ExprKind::IterHasNext(iter) => {
                self.analyze_expr(*iter, module)?;
            }
            ExprKind::MatchPattern { subject, .. } => {
                self.analyze_expr(*subject, module)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
