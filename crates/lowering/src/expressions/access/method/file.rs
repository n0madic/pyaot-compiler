//! File method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower file method calls.
    pub(super) fn lower_file_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match method_name {
            "read" => {
                // .read() or .read(n) - read entire file or n bytes
                let result_local = self.alloc_and_add_local(Type::Str, mir_func);

                if arg_operands.is_empty() {
                    // .read() - read all
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::FileRead,
                        args: vec![obj_operand],
                    });
                } else {
                    // .read(n) - read n bytes
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::FileReadN,
                        args: vec![obj_operand, arg_operands[0].clone()],
                    });
                }

                Ok(mir::Operand::Local(result_local))
            }
            "readline" => {
                // .readline() - read single line
                let result_local = self.alloc_and_add_local(Type::Str, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::FileReadline,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "readlines" => {
                // .readlines() - read all lines as list
                let result_local =
                    self.alloc_and_add_local(Type::List(Box::new(Type::Str)), mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::FileReadlines,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "write" => {
                // .write(data) - write data to file, returns bytes written
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                let data_arg = crate::first_arg_or_none(arg_operands);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::FileWrite,
                    args: vec![obj_operand, data_arg],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "close" => {
                // .close() - close the file
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::FileClose,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "flush" => {
                // .flush() - flush the file buffer
                let result_local = self.alloc_and_add_local(Type::None, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::FileFlush,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "__enter__" => {
                // Context manager enter - returns self
                let result_local = self.alloc_and_add_local(Type::File, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::FileEnter,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "__exit__" => {
                // Context manager exit - closes file and returns False
                let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::FileExit,
                    args: vec![obj_operand],
                });

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // Unknown file method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
