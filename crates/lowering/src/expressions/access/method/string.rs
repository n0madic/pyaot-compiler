//! String method lowering

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
                let result_local = self.alloc_and_add_local(Type::Bytes, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::StrEncode,
                    args: vec![obj_operand, encoding_arg],
                });
                return Ok(mir::Operand::Local(result_local));
            }
            _ => {}
        }

        // Simple methods that just need (obj, args...) -> result
        let (result_ty, runtime_func): (Type, mir::RuntimeFunc) = match method_name {
            "upper" => (Type::Str, mir::RuntimeFunc::StrUpper),
            "lower" => (Type::Str, mir::RuntimeFunc::StrLower),
            "strip" => (Type::Str, mir::RuntimeFunc::StrStrip),
            "startswith" => (Type::Bool, mir::RuntimeFunc::StrStartsWith),
            "endswith" => (Type::Bool, mir::RuntimeFunc::StrEndsWith),
            "find" => (Type::Int, mir::RuntimeFunc::StrFind),
            "rfind" => (Type::Int, mir::RuntimeFunc::StrRfind),
            "index" => (Type::Int, mir::RuntimeFunc::StrIndex),
            "rindex" => (Type::Int, mir::RuntimeFunc::StrRindex),
            "replace" => (Type::Str, mir::RuntimeFunc::StrReplace),
            "count" => (Type::Int, mir::RuntimeFunc::StrCount),
            "title" => (Type::Str, mir::RuntimeFunc::StrTitle),
            "capitalize" => (Type::Str, mir::RuntimeFunc::StrCapitalize),
            "swapcase" => (Type::Str, mir::RuntimeFunc::StrSwapcase),
            // Character predicates
            "isdigit" => (Type::Bool, mir::RuntimeFunc::StrIsDigit),
            "isalpha" => (Type::Bool, mir::RuntimeFunc::StrIsAlpha),
            "isalnum" => (Type::Bool, mir::RuntimeFunc::StrIsAlnum),
            "isspace" => (Type::Bool, mir::RuntimeFunc::StrIsSpace),
            "isupper" => (Type::Bool, mir::RuntimeFunc::StrIsUpper),
            "islower" => (Type::Bool, mir::RuntimeFunc::StrIsLower),
            "isascii" => (Type::Bool, mir::RuntimeFunc::StrIsAscii),
            // encode is handled above with default encoding
            // New string methods
            "removeprefix" => (Type::Str, mir::RuntimeFunc::StrRemovePrefix),
            "removesuffix" => (Type::Str, mir::RuntimeFunc::StrRemoveSuffix),
            "splitlines" => (
                Type::List(Box::new(Type::Str)),
                mir::RuntimeFunc::StrSplitLines,
            ),
            "partition" => (
                Type::Tuple(vec![Type::Str, Type::Str, Type::Str]),
                mir::RuntimeFunc::StrPartition,
            ),
            "rpartition" => (
                Type::Tuple(vec![Type::Str, Type::Str, Type::Str]),
                mir::RuntimeFunc::StrRpartition,
            ),
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

    /// Lower str.split(sep=None, maxsplit=-1) and str.rsplit(sep=None, maxsplit=-1)
    fn lower_str_split_variant(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // sep argument - use null pointer (0) to signal split on whitespace
        let sep_operand = arg_operands
            .first()
            .cloned()
            .unwrap_or(mir::Operand::Constant(mir::Constant::Int(0)));

        // maxsplit argument (-1 = no limit)
        let maxsplit_operand = arg_operands
            .get(1)
            .cloned()
            .unwrap_or(mir::Operand::Constant(mir::Constant::Int(-1)));

        let runtime_func = match method_name {
            "split" => mir::RuntimeFunc::StrSplit,
            "rsplit" => mir::RuntimeFunc::StrRsplit,
            _ => unreachable!(),
        };

        let result_local = self.alloc_and_add_local(Type::List(Box::new(Type::Str)), mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: runtime_func,
            args: vec![obj_operand, sep_operand, maxsplit_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower str.join(iterable)
    fn lower_str_join(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if arg_operands.is_empty() {
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::StrJoin,
            args: vec![obj_operand, arg_operands[0].clone()],
        });

        Ok(mir::Operand::Local(result_local))
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
            "lstrip" => mir::RuntimeFunc::StrLstrip,
            "rstrip" => mir::RuntimeFunc::StrRstrip,
            _ => unreachable!(),
        };

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: runtime_func,
            args: vec![obj_operand, chars_operand],
        });

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
            "center" => mir::RuntimeFunc::StrCenter,
            "ljust" => mir::RuntimeFunc::StrLjust,
            "rjust" => mir::RuntimeFunc::StrRjust,
            _ => unreachable!(),
        };

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: runtime_func,
            args: vec![obj_operand, width_operand, fillchar_operand],
        });

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

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::StrExpandTabs,
            args: vec![obj_operand, tabsize_operand],
        });

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

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::StrZfill,
            args: vec![obj_operand, width_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
