//! Literal expression lowering: Int, Float, Bool, Str, None, Var

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{InternedString, VarId};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a string literal to a heap-allocated string object.
    pub(super) fn lower_str_literal(
        &mut self,
        s: InternedString,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // For string literals, we need to allocate them on the heap
        // so they can be used with string operations
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::MakeStr,
            vec![mir::Operand::Constant(mir::Constant::Str(s))],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a bytes literal to a heap-allocated bytes object.
    pub(super) fn lower_bytes_literal(
        &mut self,
        data: &[u8],
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // For bytes literals, we need to allocate them on the heap
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::MakeBytes,
            vec![mir::Operand::Constant(mir::Constant::Bytes(data.to_vec()))],
            Type::Bytes,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a variable reference.
    pub(super) fn lower_var(
        &mut self,
        var_id: VarId,
        expr: &hir::Expr,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if let Some(local_id) = self.get_block_narrowed_local(&var_id) {
            return Ok(mir::Operand::Local(local_id));
        }

        // Check if this is a global variable
        if self.is_global(&var_id) {
            // Global variable: emit runtime call to get the value
            let var_type = self
                .get_var_type(&var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Int); // Globals default to Int for backward compatibility

            // Determine the type-specific runtime function for global get
            let runtime_func = self.get_global_get_func(&var_type);

            // Emit type-specific GlobalGet runtime call with offset-adjusted VarId
            let effective_var_id = self.get_effective_var_id(var_id);
            let result_local = self.emit_runtime_call(
                runtime_func,
                vec![mir::Operand::Constant(mir::Constant::Int(effective_var_id))],
                var_type,
                mir_func,
            );

            Ok(mir::Operand::Local(result_local))
        } else if let Some(cell_local) = self.get_nonlocal_cell(&var_id) {
            // Cell-wrapped variable (either cell_var or nonlocal_var): read through cell
            let var_type = self
                .get_var_type(&var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Int);

            // Emit cell get operation
            let get_func = self.get_cell_get_func(&var_type);
            let result_local = self.emit_runtime_call(
                get_func,
                vec![mir::Operand::Local(cell_local)],
                var_type,
                mir_func,
            );

            Ok(mir::Operand::Local(result_local))
        } else {
            // Local variable: use the standard local mapping
            let local_type = self
                .get_var_type(&var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any);
            let local_id = self.get_or_create_local(var_id, local_type, mir_func);
            Ok(mir::Operand::Local(local_id))
        }
    }
}
