//! Type conversion functions lowering: str(), int(), float(), bool(), bytes(), chr(), ord()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower str(x)
    pub(super) fn lower_str(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // str() with no args returns empty string ""
            let result_local = self.alloc_and_add_local(Type::Str, mir_func);
            let empty = self.intern("");
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::MakeStr,
                args: vec![mir::Operand::Constant(mir::Constant::Str(empty))],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        // For Union types, use runtime dispatch since actual type is determined at runtime
        if arg_type.is_union() {
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_OBJ_TO_STR),
                args: vec![arg_operand],
            });
        } else {
            match arg_type {
                Type::Str => {
                    // str(str) -> returns the same string (copy for now)
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: result_local,
                        src: arg_operand,
                    });
                }
                Type::Int => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(mir::ConversionTypeKind::convert_def(
                            mir::ConversionTypeKind::Int,
                            mir::ConversionTypeKind::Str,
                        )),
                        args: vec![arg_operand],
                    });
                }
                Type::Float => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(mir::ConversionTypeKind::convert_def(
                            mir::ConversionTypeKind::Float,
                            mir::ConversionTypeKind::Str,
                        )),
                        args: vec![arg_operand],
                    });
                }
                Type::Bool => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(mir::ConversionTypeKind::convert_def(
                            mir::ConversionTypeKind::Bool,
                            mir::ConversionTypeKind::Str,
                        )),
                        args: vec![arg_operand],
                    });
                }
                Type::None => {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(mir::ConversionTypeKind::convert_def(
                            mir::ConversionTypeKind::None,
                            mir::ConversionTypeKind::Str,
                        )),
                        args: vec![],
                    });
                }
                Type::BuiltinException(_) => {
                    // str(exception) returns the message from .args tuple
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::ExcInstanceStr,
                        args: vec![arg_operand],
                    });
                }
                Type::Class { class_id, .. } => {
                    // Check for __str__ or __repr__ methods
                    if let Some(class_info) = self.get_class_info(&class_id) {
                        // Try __str__ first
                        if let Some(str_func) = class_info.get_dunder_func("__str__") {
                            self.emit_instruction(mir::InstructionKind::CallDirect {
                                dest: result_local,
                                func: str_func,
                                args: vec![arg_operand],
                            });
                        }
                        // Fallback to __repr__ if __str__ not defined
                        else if let Some(repr_func) = class_info.get_dunder_func("__repr__") {
                            self.emit_instruction(mir::InstructionKind::CallDirect {
                                dest: result_local,
                                func: repr_func,
                                args: vec![arg_operand],
                            });
                        }
                        // For exception classes without __str__/__repr__, extract message
                        else if class_info.is_exception_class {
                            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                                dest: result_local,
                                func: mir::RuntimeFunc::ExcInstanceStr,
                                args: vec![arg_operand],
                            });
                        }
                        // Use default repr for non-exception classes
                        else {
                            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                                dest: result_local,
                                func: mir::RuntimeFunc::Call(
                                    &pyaot_core_defs::runtime_func_def::RT_OBJ_DEFAULT_REPR,
                                ),
                                args: vec![arg_operand],
                            });
                        }
                    } else {
                        // Class not found - use default repr
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: result_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_OBJ_DEFAULT_REPR,
                            ),
                            args: vec![arg_operand],
                        });
                    }
                }
                _ => {
                    // For other types (list, tuple, dict, set, bytes, etc.),
                    // use runtime dispatch which reads the type tag from the object header
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_OBJ_TO_STR,
                        ),
                        args: vec![arg_operand],
                    });
                }
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower int(x)
    pub(super) fn lower_int(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // int() with no args returns 0
            return Ok(mir::Operand::Constant(mir::Constant::Int(0)));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        let result_local = self.alloc_and_add_local(Type::Int, mir_func);

        match arg_type {
            Type::Int => {
                // int(int) -> returns the same value (copy)
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::Float => {
                // int(float) -> truncate to zero using FloatToInt instruction
                self.emit_instruction(mir::InstructionKind::FloatToInt {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::Bool => {
                // int(bool) -> True=1, False=0 (zero-extend i8 to i64)
                self.emit_instruction(mir::InstructionKind::BoolToInt {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::Str => {
                // int(str, base=10) -> parse string to integer with optional base
                // Check if base parameter is provided
                if args.len() > 1 {
                    // int(str, base) - use StrToIntWithBase
                    let base_expr = &hir_module.exprs[args[1]];
                    let base_operand = self.lower_expr(base_expr, hir_module, mir_func)?;

                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_STR_TO_INT_WITH_BASE,
                        ),
                        args: vec![arg_operand, base_operand],
                    });
                } else {
                    // int(str) - use default base 10
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(mir::ConversionTypeKind::convert_def(
                            mir::ConversionTypeKind::Str,
                            mir::ConversionTypeKind::Int,
                        )),
                        args: vec![arg_operand],
                    });
                }
            }
            Type::Class { class_id, .. } => {
                // int(obj) -> call __int__ dunder if defined
                if let Some(int_func) = self
                    .get_class_info(&class_id)
                    .and_then(|ci| ci.get_dunder_func("__int__"))
                {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: int_func,
                        args: vec![arg_operand],
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: result_local,
                        value: mir::Constant::Int(0),
                    });
                }
            }
            _ => {
                // For other types, return 0 as fallback
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: result_local,
                    value: mir::Constant::Int(0),
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower float(x)
    pub(super) fn lower_float(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // float() with no args returns 0.0
            return Ok(mir::Operand::Constant(mir::Constant::Float(0.0)));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        let result_local = self.alloc_and_add_local(Type::Float, mir_func);

        match arg_type {
            Type::Float => {
                // float(float) -> returns the same value (copy)
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::Int => {
                // float(int) -> convert i64 to f64
                self.emit_instruction(mir::InstructionKind::IntToFloat {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::Bool => {
                // float(bool) -> True=1.0, False=0.0
                // First convert bool to int, then int to float
                let temp_int = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BoolToInt {
                    dest: temp_int,
                    src: arg_operand,
                });
                self.emit_instruction(mir::InstructionKind::IntToFloat {
                    dest: result_local,
                    src: mir::Operand::Local(temp_int),
                });
            }
            Type::Str => {
                // float(str) -> parse string to float (can raise ValueError)
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(mir::ConversionTypeKind::convert_def(
                        mir::ConversionTypeKind::Str,
                        mir::ConversionTypeKind::Float,
                    )),
                    args: vec![arg_operand],
                });
            }
            Type::Class { class_id, .. } => {
                // float(obj) -> call __float__ dunder if defined
                if let Some(float_func) = self
                    .get_class_info(&class_id)
                    .and_then(|ci| ci.get_dunder_func("__float__"))
                {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: float_func,
                        args: vec![arg_operand],
                    });
                } else {
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: result_local,
                        value: mir::Constant::Float(0.0),
                    });
                }
            }
            _ => {
                // For other types, return 0.0 as fallback
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: result_local,
                    value: mir::Constant::Float(0.0),
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower bool(x) - test truthiness
    pub(super) fn lower_bool(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        if args.is_empty() {
            // bool() with no args returns False
            return Ok(mir::Operand::Constant(mir::Constant::Bool(false)));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        match arg_type {
            Type::Bool => {
                // bool(bool) -> returns the same value (copy)
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            Type::Int => {
                // bool(int) -> False if 0, True otherwise
                // result = (arg != 0)
                let zero = mir::Operand::Constant(mir::Constant::Int(0));
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir::BinOp::NotEq,
                    left: arg_operand,
                    right: zero,
                });
            }
            Type::Float => {
                // bool(float) -> False if 0.0, True otherwise
                let zero = mir::Operand::Constant(mir::Constant::Float(0.0));
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir::BinOp::NotEq,
                    left: arg_operand,
                    right: zero,
                });
            }
            Type::Str => {
                self.emit_collection_bool_via_len(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STR_LEN_INT),
                    arg_operand,
                    result_local,
                    mir_func,
                );
            }
            Type::List(_) => {
                self.emit_collection_bool_via_len(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN),
                    arg_operand,
                    result_local,
                    mir_func,
                );
            }
            Type::Tuple(_) => {
                self.emit_collection_bool_via_len(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_LEN),
                    arg_operand,
                    result_local,
                    mir_func,
                );
            }
            Type::Dict(_, _) => {
                self.emit_collection_bool_via_len(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_LEN),
                    arg_operand,
                    result_local,
                    mir_func,
                );
            }
            Type::None => {
                // bool(None) -> False
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: result_local,
                    value: mir::Constant::Bool(false),
                });
            }
            Type::Class { class_id, .. } => {
                // bool(obj) -> call __bool__ dunder, fall back to __len__, default True
                if let Some(bool_func) = self
                    .get_class_info(&class_id)
                    .and_then(|ci| ci.get_dunder_func("__bool__"))
                {
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: bool_func,
                        args: vec![arg_operand],
                    });
                } else if let Some(len_func) = self
                    .get_class_info(&class_id)
                    .and_then(|ci| ci.get_dunder_func("__len__"))
                {
                    // Python: __len__() != 0 used for truthiness if __bool__ not defined
                    let len_local = self.alloc_and_add_local(Type::Int, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: len_local,
                        func: len_func,
                        args: vec![arg_operand],
                    });
                    self.emit_instruction(mir::InstructionKind::BinOp {
                        dest: result_local,
                        op: mir::BinOp::NotEq,
                        left: mir::Operand::Local(len_local),
                        right: mir::Operand::Constant(mir::Constant::Int(0)),
                    });
                } else {
                    // No __bool__ or __len__: instances are truthy by default
                    self.emit_instruction(mir::InstructionKind::Const {
                        dest: result_local,
                        value: mir::Constant::Bool(true),
                    });
                }
            }
            _ => {
                // For other types, return True as fallback
                self.emit_instruction(mir::InstructionKind::Const {
                    dest: result_local,
                    value: mir::Constant::Bool(true),
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower bytes() - create bytes object
    pub(super) fn lower_bytes(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let result_local = self.alloc_and_add_local(Type::Bytes, mir_func);

        if args.is_empty() {
            // bytes() with no args returns empty bytes
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: result_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_MAKE_BYTES_ZERO,
                ),
                args: vec![mir::Operand::Constant(mir::Constant::Int(0))],
            });
            return Ok(mir::Operand::Local(result_local));
        }

        let arg_expr = &hir_module.exprs[args[0]];
        let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
        let arg_type = self.get_expr_type(arg_expr, hir_module);

        match arg_type {
            Type::Int => {
                // bytes(n) -> create bytes of n zeros
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_MAKE_BYTES_ZERO,
                    ),
                    args: vec![arg_operand],
                });
            }
            Type::List(_) => {
                // bytes([65, 66, 67]) -> create bytes from list of integers
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_MAKE_BYTES_FROM_LIST,
                    ),
                    args: vec![arg_operand],
                });
            }
            Type::Str => {
                // bytes(str, encoding) -> create bytes from string
                // For simplicity, we assume UTF-8 encoding
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_MAKE_BYTES_FROM_STR,
                    ),
                    args: vec![arg_operand],
                });
            }
            Type::Bytes => {
                // bytes(bytes) -> copy bytes
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: arg_operand,
                });
            }
            _ => {
                // Fallback: return empty bytes
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        &pyaot_core_defs::runtime_func_def::RT_MAKE_BYTES_ZERO,
                    ),
                    args: vec![mir::Operand::Constant(mir::Constant::Int(0))],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower chr(i: int) -> str
    pub(super) fn lower_chr(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "chr", self.call_span())?;

        let i_expr = &hir_module.exprs[args[0]];
        let i_operand = self.lower_expr(i_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_INT_TO_CHR),
            args: vec![i_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }

    /// Lower ord(s: str) -> int
    pub(super) fn lower_ord(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        self.require_exact_args(args, 1, "ord", self.call_span())?;

        let s_expr = &hir_module.exprs[args[0]];
        let s_operand = self.lower_expr(s_expr, hir_module, mir_func)?;

        let result_local = self.alloc_and_add_local(Type::Int, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_CHR_TO_INT),
            args: vec![s_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
