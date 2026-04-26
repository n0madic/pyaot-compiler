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

                // After §F.7c BigBang: rt_generator_send returns tagged Value bits
                // (the resume function boxes its yield). Unwrap for typed Int/Bool.
                let raw_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_SEND),
                    vec![obj_operand, value_operand],
                    Type::HeapAny,
                    mir_func,
                );

                let result_local = match elem_ty {
                    Type::Int => {
                        let dest = self.alloc_and_add_local(Type::Int, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnwrapValueInt {
                            dest,
                            src: mir::Operand::Local(raw_local),
                        });
                        dest
                    }
                    Type::Bool => {
                        let dest = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnwrapValueBool {
                            dest,
                            src: mir::Operand::Local(raw_local),
                        });
                        dest
                    }
                    _ => {
                        let dest = self.alloc_and_add_local(elem_ty.clone(), mir_func);
                        self.emit_instruction(mir::InstructionKind::Copy {
                            dest,
                            src: mir::Operand::Local(raw_local),
                        });
                        dest
                    }
                };

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
