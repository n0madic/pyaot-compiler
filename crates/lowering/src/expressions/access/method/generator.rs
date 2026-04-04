//! Generator/iterator method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower generator/iterator method calls (send, close).
    pub(super) fn lower_generator_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        elem_ty: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match method_name {
            "send" => {
                // g.send(value) -> yielded value
                // Get value to send (default to 0/None if not provided)
                let value_operand = if !arg_operands.is_empty() {
                    arg_operands[0].clone()
                } else {
                    mir::Operand::Constant(mir::Constant::Int(0))
                };

                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_SEND),
                    vec![obj_operand, value_operand],
                    elem_ty.clone(),
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "close" => {
                // g.close() -> None
                self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_CLOSE),
                    vec![obj_operand],
                    Type::None,
                    mir_func,
                );

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            _ => {
                // Unknown generator method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
