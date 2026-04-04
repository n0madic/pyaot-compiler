//! Bytes method lowering

use pyaot_core_defs::runtime_func_def::*;
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
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_BYTES_DECODE),
                    vec![obj_operand, encoding_arg],
                    Type::Str,
                    mir_func,
                );
                return Ok(mir::Operand::Local(result_local));
            }
            "index" | "rindex" => {
                // index/rindex use unified rt_bytes_search with op_tag
                let op_tag: i64 = if method_name == "index" { 2 } else { 3 };
                let def = if method_name == "index" {
                    &RT_BYTES_INDEX
                } else {
                    &RT_BYTES_RINDEX
                };
                let mut all_args = vec![obj_operand];
                all_args.extend(arg_operands);
                all_args.push(mir::Operand::Constant(mir::Constant::Int(op_tag)));
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(def),
                    all_args,
                    Type::Int,
                    mir_func,
                );
                return Ok(mir::Operand::Local(result_local));
            }
            _ => {}
        }

        // Simple methods that just need (obj, args...) -> result
        let (result_ty, runtime_func): (Type, mir::RuntimeFunc) = match method_name {
            "startswith" => (Type::Bool, mir::RuntimeFunc::Call(&RT_BYTES_STARTS_WITH)),
            "endswith" => (Type::Bool, mir::RuntimeFunc::Call(&RT_BYTES_ENDS_WITH)),
            "find" => (Type::Int, mir::RuntimeFunc::Call(&RT_BYTES_FIND)),
            "rfind" => (Type::Int, mir::RuntimeFunc::Call(&RT_BYTES_RFIND)),
            "count" => (Type::Int, mir::RuntimeFunc::Call(&RT_BYTES_COUNT)),
            "replace" => (Type::Bytes, mir::RuntimeFunc::Call(&RT_BYTES_REPLACE)),
            "strip" => (Type::Bytes, mir::RuntimeFunc::Call(&RT_BYTES_STRIP)),
            "lstrip" => (Type::Bytes, mir::RuntimeFunc::Call(&RT_BYTES_LSTRIP)),
            "rstrip" => (Type::Bytes, mir::RuntimeFunc::Call(&RT_BYTES_RSTRIP)),
            "upper" => (Type::Bytes, mir::RuntimeFunc::Call(&RT_BYTES_UPPER)),
            "lower" => (Type::Bytes, mir::RuntimeFunc::Call(&RT_BYTES_LOWER)),
            _ => {
                // Unknown method
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        };

        // Build args: obj first, then method args
        let mut all_args = vec![obj_operand];
        all_args.extend(arg_operands);

        let result_local = self.emit_runtime_call(runtime_func, all_args, result_ty, mir_func);

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
            "split" => mir::RuntimeFunc::Call(&RT_BYTES_SPLIT),
            "rsplit" => mir::RuntimeFunc::Call(&RT_BYTES_RSPLIT),
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
            mir::RuntimeFunc::Call(&RT_BYTES_JOIN),
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

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&RT_BYTES_FROM_HEX),
            vec![arg_operands[0].clone()],
            Type::Bytes,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }
}
