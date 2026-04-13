//! File method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower file method calls. `binary` comes from the `Type::File(bool)`
    /// of the receiver — in binary mode `.read()` / `.readline()` /
    /// `.readlines()` return `bytes` / `list[bytes]`, while text mode returns
    /// `str` / `list[str]`. The runtime `rt_file_*` implementations already
    /// branch on the file's binary flag; this only gets the static type right.
    pub(super) fn lower_file_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        binary: bool,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let str_or_bytes = if binary { Type::Bytes } else { Type::Str };
        match method_name {
            "read" => {
                // .read() or .read(n) - read entire file or n bytes
                let result_local = if arg_operands.is_empty() {
                    // .read() - read all
                    self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_READ),
                        vec![obj_operand],
                        str_or_bytes,
                        mir_func,
                    )
                } else {
                    // .read(n) - read n bytes
                    self.emit_runtime_call(
                        mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_READ_N),
                        vec![obj_operand, arg_operands[0].clone()],
                        str_or_bytes,
                        mir_func,
                    )
                };

                Ok(mir::Operand::Local(result_local))
            }
            "readline" => {
                // .readline() - read single line
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_READLINE),
                    vec![obj_operand],
                    str_or_bytes,
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "readlines" => {
                // .readlines() - read all lines as list
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_READLINES),
                    vec![obj_operand],
                    Type::List(Box::new(str_or_bytes)),
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "write" => {
                // .write(data) - write data to file, returns bytes written
                let data_arg = crate::first_arg_or_none(arg_operands);

                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_WRITE),
                    vec![obj_operand, data_arg],
                    Type::Int,
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "close" => {
                // .close() - close the file
                self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_CLOSE),
                    vec![obj_operand],
                    Type::None,
                    mir_func,
                );

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "flush" => {
                // .flush() - flush the file buffer
                self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_FLUSH),
                    vec![obj_operand],
                    Type::None,
                    mir_func,
                );

                Ok(mir::Operand::Constant(mir::Constant::None))
            }
            "__enter__" => {
                // Context manager enter - returns self
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_ENTER),
                    vec![obj_operand],
                    Type::File(binary),
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            "__exit__" => {
                // Context manager exit - closes file and returns False
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_EXIT),
                    vec![obj_operand],
                    Type::Bool,
                    mir_func,
                );

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // Unknown file method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
