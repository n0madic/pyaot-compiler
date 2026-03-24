//! Unified type planning system
//!
//! Single module for all type inference in lowering:
//! - `infer`: bottom-up type synthesis (`compute_expr_type`)
//! - `pre_scan`: closure/lambda/decorator discovery before codegen
//! - `check`: top-down type validation + error reporting

mod check;
mod infer;
mod pre_scan;

use pyaot_hir as hir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Run type planning: pre-scan for closures, decorators, lambda hints.
    pub(crate) fn run_type_planning(&mut self, hir_module: &hir::Module) {
        self.precompute_closure_capture_types(hir_module);
        self.process_module_decorated_functions(hir_module);
    }

    /// Get the type of an expression by its ID (memoized).
    pub(crate) fn get_type_of_expr_id(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
    ) -> Type {
        if let Some(cached) = self.expr_types.get(&expr_id).cloned() {
            return cached;
        }
        let expr = &hir_module.exprs[expr_id];
        let result = self.compute_expr_type(expr, hir_module);
        self.expr_types.insert(expr_id, result.clone());
        result
    }

    /// Get the effective type of an expression.
    pub(crate) fn get_expr_type(&mut self, expr: &hir::Expr, hir_module: &hir::Module) -> Type {
        self.compute_expr_type(expr, hir_module)
    }
}
