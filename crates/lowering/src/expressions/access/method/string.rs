//! String method lowering

use pyaot_core_defs::runtime_func_def;
use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower string method calls.
    pub(super) fn lower_str_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Handle methods with special argument processing
        match method_name {
            "split" | "rsplit" => {
                return self.lower_str_split_variant(
                    obj_operand,
                    method_name,
                    arg_operands,
                    mir_func,
                );
            }
            "join" => {
                return self.lower_str_join(obj_operand, arg_operands, mir_func);
            }
            "lstrip" | "rstrip" => {
                return self.lower_str_strip_variant(
                    obj_operand,
                    method_name,
                    arg_operands,
                    mir_func,
                );
            }
            "center" | "ljust" | "rjust" => {
                return self.lower_str_padding(obj_operand, method_name, arg_operands, mir_func);
            }
            "zfill" => {
                return self.lower_str_zfill(obj_operand, arg_operands, mir_func);
            }
            "expandtabs" => {
                return self.lower_str_expandtabs(obj_operand, arg_operands, mir_func);
            }
            "encode" => {
                // encode(encoding="utf-8") - provide default encoding if not given
                // Use Constant::None as the default so the runtime treats it as "use UTF-8"
                let encoding_arg = arg_operands
                    .first()
                    .cloned()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                let result_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ENCODE),
                    vec![obj_operand, encoding_arg],
                    Type::Bytes,
                    mir_func,
                );
                return Ok(mir::Operand::Local(result_local));
            }
            _ => {}
        }

        // Handle find/rfind/index/rindex: need op_tag as 3rd argument
        if let Some((op_tag, def)) = match method_name {
            "find" => Some((mir::SearchOp::Find.to_tag(), &runtime_func_def::RT_STR_FIND)),
            "rfind" => Some((
                mir::SearchOp::Rfind.to_tag(),
                &runtime_func_def::RT_STR_RFIND,
            )),
            "index" => Some((
                mir::SearchOp::Index.to_tag(),
                &runtime_func_def::RT_STR_INDEX,
            )),
            "rindex" => Some((
                mir::SearchOp::Rindex.to_tag(),
                &runtime_func_def::RT_STR_RINDEX,
            )),
            _ => None,
        } {
            let mut all_args = vec![obj_operand];
            all_args.extend(arg_operands);
            all_args.push(mir::Operand::Constant(mir::Constant::Int(op_tag as i64)));
            let result_local =
                self.emit_runtime_call(mir::RuntimeFunc::Call(def), all_args, Type::Int, mir_func);
            return Ok(mir::Operand::Local(result_local));
        }

        // Simple methods that just need (obj, args...) -> result
        let (result_ty, runtime_func): (Type, mir::RuntimeFunc) = match method_name {
            "upper" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_UPPER),
            ),
            "lower" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_LOWER),
            ),
            "strip" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_STRIP),
            ),
            "startswith" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_STARTSWITH),
            ),
            "endswith" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ENDSWITH),
            ),
            "replace" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_REPLACE),
            ),
            "count" => (
                Type::Int,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_COUNT),
            ),
            "title" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_TITLE),
            ),
            "capitalize" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_CAPITALIZE),
            ),
            "swapcase" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_SWAPCASE),
            ),
            // Character predicates
            "isdigit" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ISDIGIT),
            ),
            "isalpha" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ISALPHA),
            ),
            "isalnum" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ISALNUM),
            ),
            "isspace" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ISSPACE),
            ),
            "isupper" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ISUPPER),
            ),
            "islower" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ISLOWER),
            ),
            "isascii" => (
                Type::Bool,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ISASCII),
            ),
            // encode is handled above with default encoding
            // New string methods
            "removeprefix" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_REMOVEPREFIX),
            ),
            "removesuffix" => (
                Type::Str,
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_REMOVESUFFIX),
            ),
            "splitlines" => (
                Type::List(Box::new(Type::Str)),
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_SPLITLINES),
            ),
            "partition" => (
                Type::Tuple(vec![Type::Str, Type::Str, Type::Str]),
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_PARTITION),
            ),
            "rpartition" => (
                Type::Tuple(vec![Type::Str, Type::Str, Type::Str]),
                mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_RPARTITION),
            ),
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

    /// Lower str.split(sep=None, maxsplit=-1) and str.rsplit(sep=None, maxsplit=-1)
    fn lower_str_split_variant(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let runtime_func = match method_name {
            "split" => mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_SPLIT),
            "rsplit" => mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_RSPLIT),
            _ => unreachable!(),
        };
        self.lower_split_variant_impl(obj_operand, arg_operands, runtime_func, Type::Str, mir_func)
    }

    /// Lower str.join(iterable)
    fn lower_str_join(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.lower_join_impl(
            obj_operand,
            arg_operands,
            mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_JOIN),
            Type::Str,
            mir_func,
        )
    }

    /// Lower str.lstrip(chars=None) and str.rstrip(chars=None)
    fn lower_str_strip_variant(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // chars argument - use null pointer (0) to signal strip whitespace
        let chars_operand = arg_operands
            .first()
            .cloned()
            .unwrap_or(mir::Operand::Constant(mir::Constant::Int(0)));

        let runtime_func = match method_name {
            "lstrip" => mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_LSTRIP),
            "rstrip" => mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_RSTRIP),
            _ => unreachable!(),
        };

        let result_local = self.emit_runtime_call(
            runtime_func,
            vec![obj_operand, chars_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower str.center(width, fillchar=' '), str.ljust(width, fillchar=' '), str.rjust(width, fillchar=' ')
    fn lower_str_padding(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if arg_operands.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let width_operand = arg_operands[0].clone();

        // fillchar argument - use null pointer (0) to signal default space
        let fillchar_operand = arg_operands
            .get(1)
            .cloned()
            .unwrap_or(mir::Operand::Constant(mir::Constant::Int(0)));

        let runtime_func = match method_name {
            "center" => mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_CENTER),
            "ljust" => mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_LJUST),
            "rjust" => mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_RJUST),
            _ => unreachable!(),
        };

        let result_local = self.emit_runtime_call(
            runtime_func,
            vec![obj_operand, width_operand, fillchar_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower str.expandtabs(tabsize=8)
    fn lower_str_expandtabs(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // tabsize argument (default 8)
        let tabsize_operand = arg_operands
            .first()
            .cloned()
            .unwrap_or(mir::Operand::Constant(mir::Constant::Int(8)));

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_EXPANDTABS),
            vec![obj_operand, tabsize_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower str.zfill(width)
    fn lower_str_zfill(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if arg_operands.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let width_operand = arg_operands[0].clone();

        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_ZFILL),
            vec![obj_operand, width_operand],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }
}
