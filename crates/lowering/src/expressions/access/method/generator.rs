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
                let result_local = self.alloc_and_add_local(elem_ty.clone(), mir_func);

                // Get value to send (default to 0/None if not provided)
                let value_operand = if !arg_operands.is_empty() {
                    arg_operands[0].clone()
                } else {
                    mir::Operand::Constant(mir::Constant::Int(0))
                };

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_GENERATOR_SEND,
                    ),
                    args: vec![obj_operand, value_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "close" => {
                // g.close() -> None
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_GENERATOR_CLOSE,
                    ),
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            _ => {
                // Unknown generator method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
