//! Bytes method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower bytes method calls.
    pub(super) fn lower_bytes_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Handle methods with special argument processing
        match method_name {
            "split" | "rsplit" => {
                return self.lower_bytes_split_variant(
                    obj_operand,
                    method_name,
                    arg_operands,
                    mir_func,
                );
            }
            "join" => {
                return self.lower_bytes_join(obj_operand, arg_operands, mir_func);
            }
            "fromhex" => {
                return self.lower_bytes_from_hex(arg_operands, mir_func);
            }
            "decode" => {
                // decode(encoding="utf-8") - provide default encoding if not given
                let encoding_arg = arg_operands
                    .first()
                    .cloned()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::Int(0)));
                let result_local = self.alloc_and_add_local(Type::Str, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::BytesDecode,
                    args: vec![obj_operand, encoding_arg],
                });
                return Ok(mir::Operand::Local(result_local));
            }
            _ => {}
        }

        // Simple methods that just need (obj, args...) -> result
        let (result_ty, runtime_func): (Type, mir::RuntimeFunc) = match method_name {
            "startswith" => (Type::Bool, mir::RuntimeFunc::BytesStartsWith),
            "endswith" => (Type::Bool, mir::RuntimeFunc::BytesEndsWith),
            "find" => (Type::Int, mir::RuntimeFunc::BytesFind),
            "rfind" => (Type::Int, mir::RuntimeFunc::BytesRfind),
            "index" => (Type::Int, mir::RuntimeFunc::BytesIndex),
            "rindex" => (Type::Int, mir::RuntimeFunc::BytesRindex),
            "count" => (Type::Int, mir::RuntimeFunc::BytesCount),
            "replace" => (Type::Bytes, mir::RuntimeFunc::BytesReplace),
            "strip" => (Type::Bytes, mir::RuntimeFunc::BytesStrip),
            "lstrip" => (Type::Bytes, mir::RuntimeFunc::BytesLstrip),
            "rstrip" => (Type::Bytes, mir::RuntimeFunc::BytesRstrip),
            "upper" => (Type::Bytes, mir::RuntimeFunc::BytesUpper),
            "lower" => (Type::Bytes, mir::RuntimeFunc::BytesLower),
            _ => {
                // Unknown method
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        };

        let result_local = self.alloc_and_add_local(result_ty, mir_func);

        // Build args: obj first, then method args
        let mut all_args = vec![obj_operand];
        all_args.extend(arg_operands);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: runtime_func,
            args: all_args,
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower bytes.split(sep=None, maxsplit=-1) and bytes.rsplit(sep=None, maxsplit=-1)
    fn lower_bytes_split_variant(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let runtime_func = match method_name {
            "split" => mir::RuntimeFunc::BytesSplit,
            "rsplit" => mir::RuntimeFunc::BytesRsplit,
            _ => unreachable!(),
        };
        self.lower_split_variant_impl(
            obj_operand,
            arg_operands,
            runtime_func,
            Type::Bytes,
            mir_func,
        )
    }

    /// Lower bytes.join(iterable)
    fn lower_bytes_join(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.lower_join_impl(
            obj_operand,
            arg_operands,
            mir::RuntimeFunc::BytesJoin,
            Type::Bytes,
            mir_func,
        )
    }

    /// Lower bytes.fromhex(string) - static method
    fn lower_bytes_from_hex(
        &mut self,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if arg_operands.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let result_local = self.alloc_and_add_local(Type::Bytes, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::BytesFromHex,
            args: vec![arg_operands[0].clone()],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
