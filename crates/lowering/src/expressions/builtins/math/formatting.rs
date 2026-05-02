//! Number formatting lowering: bin(), hex(), oct()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower bin(n) -> str (e.g., '0b1010')
    pub(in crate::expressions::builtins) fn lower_bin(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "bin", self.call_span())?;

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_TO_BIN),
            vec![n_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower hex(n) -> str (e.g., '0xff')
    pub(in crate::expressions::builtins) fn lower_hex(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "hex", self.call_span())?;

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_TO_HEX),
            vec![n_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower oct(n) -> str (e.g., '0o10')
    pub(in crate::expressions::builtins) fn lower_oct(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "oct", self.call_span())?;

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_TO_OCT),
            vec![n_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }
}
