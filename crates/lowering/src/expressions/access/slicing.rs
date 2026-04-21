//! Slice expression lowering: obj[start:end:step]

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;

use crate::context::Lowering;
use crate::type_dispatch::{select_slicing_func, select_slicing_step_func};

impl<'a> Lowering<'a> {
    /// Lower a slice expression: obj[start:end:step]
    pub(in crate::expressions) fn lower_slice(
        &mut self,
        obj: hir::ExprId,
        start: &Option<hir::ExprId>,
        end: &Option<hir::ExprId>,
        step: &Option<hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        // Use seed_expr_type for proper type inference
        let obj_type = self.seed_expr_type(obj, hir_module);

        // Default values for slice: use sentinel values for unspecified start/end
        // i64::MIN for unspecified start, i64::MAX for unspecified end
        // Runtime will apply correct defaults based on step direction
        let start_operand = if let Some(start_id) = start {
            let start_expr = &hir_module.exprs[*start_id];
            self.lower_expr(start_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::Int(i64::MIN))
        };

        // For end, we use i64::MAX to mean "unspecified"
        // The runtime will apply correct default based on step direction
        let end_operand = if let Some(end_id) = end {
            let end_expr = &hir_module.exprs[*end_id];
            self.lower_expr(end_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::Int(i64::MAX))
        };

        let result_local = if let Some(step_id) = step {
            // Slice with step: look up the step-variant function
            let Some(func_def) = select_slicing_step_func(&obj_type) else {
                return Ok(mir::Operand::Constant(mir::Constant::None));
            };
            let step_expr = &hir_module.exprs[*step_id];
            let step_operand = self.lower_expr(step_expr, hir_module, mir_func)?;
            self.emit_runtime_call(
                mir::RuntimeFunc::Call(func_def),
                vec![obj_operand, start_operand, end_operand, step_operand],
                obj_type.clone(),
                mir_func,
            )
        } else {
            // Simple slice without step: look up the plain function
            let Some(func_def) = select_slicing_func(&obj_type) else {
                return Ok(mir::Operand::Constant(mir::Constant::None));
            };
            self.emit_runtime_call(
                mir::RuntimeFunc::Call(func_def),
                vec![obj_operand, start_operand, end_operand],
                obj_type.clone(),
                mir_func,
            )
        };

        Ok(mir::Operand::Local(result_local))
    }
}
