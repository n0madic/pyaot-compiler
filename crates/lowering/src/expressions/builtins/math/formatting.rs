//! Number formatting lowering: bin(), hex(), oct(), fmt_int(), fmt_int_grouped(), fmt_float_grouped()

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

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_TO_BIN),
            args: vec![n_operand],
        });

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

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_TO_HEX),
            args: vec![n_operand],
        });

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

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_TO_OCT),
            args: vec![n_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower format-specific integer conversion (hex/oct/bin without prefix)
    pub(in crate::expressions::builtins) fn lower_fmt_int(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
        runtime_func: mir::RuntimeFunc,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "fmt_int", self.call_span())?;

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: runtime_func,
            args: vec![n_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower format integer with grouping separator: fmt_int_grouped(n, sep)
    pub(in crate::expressions::builtins) fn lower_fmt_int_grouped(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 2, "fmt_int_grouped", self.call_span())?;

        let n_expr = &hir_module.exprs[args[0]];
        let n_operand = self.lower_expr(n_expr, hir_module, mir_func)?;

        let sep_expr = &hir_module.exprs[args[1]];
        let sep_operand = self.lower_expr(sep_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_FMT_GROUPED),
            args: vec![n_operand, sep_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower format float with precision and grouping: fmt_float_grouped(f, precision, sep)
    pub(in crate::expressions::builtins) fn lower_fmt_float_grouped(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 3, "fmt_float_grouped", self.call_span())?;

        let f_expr = &hir_module.exprs[args[0]];
        let f_operand = self.lower_expr(f_expr, hir_module, mir_func)?;

        let prec_expr = &hir_module.exprs[args[1]];
        let prec_operand = self.lower_expr(prec_expr, hir_module, mir_func)?;

        let sep_expr = &hir_module.exprs[args[2]];
        let sep_operand = self.lower_expr(sep_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FLOAT_FMT_GROUPED),
            args: vec![f_operand, prec_operand, sep_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
