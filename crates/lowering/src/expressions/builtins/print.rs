//! Print and input function lowering

use pyaot_core_defs::runtime_func_def;
use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir::{self as mir, PrintKind};
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower print(*args, sep=" ", end="\n")
    pub(super) fn lower_print(
        &mut self,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Extract sep, end, file, and flush from kwargs (defaults: " ", "\n", stdout, False)
        let mut sep_operand: Option<mir::Operand> = None;
        let mut end_operand: Option<mir::Operand> = None;
        let mut use_stderr = false;
        let mut need_flush = false;

        for kwarg in kwargs {
            let name = self.resolve(kwarg.name);
            match name {
                "sep" => {
                    let expr = &hir_module.exprs[kwarg.value];
                    sep_operand = Some(self.lower_expr(expr, hir_module, mir_func)?);
                }
                "end" => {
                    let expr = &hir_module.exprs[kwarg.value];
                    end_operand = Some(self.lower_expr(expr, hir_module, mir_func)?);
                }
                "file" => {
                    // Check if value is sys.stderr (simplified: any file= triggers stderr output)
                    // In a full implementation, we'd verify it's actually sys.stderr
                    // For now, treat any file parameter as requesting stderr output
                    let expr = &hir_module.exprs[kwarg.value];
                    if let hir::ExprKind::Attribute { attr, .. } = &expr.kind {
                        let attr_name = self.resolve(*attr);
                        if attr_name == "stderr" {
                            use_stderr = true;
                        }
                    }
                }
                "flush" => {
                    // Check if flush is True
                    let expr = &hir_module.exprs[kwarg.value];
                    if matches!(expr.kind, hir::ExprKind::Bool(true)) {
                        need_flush = true;
                    }
                }
                _ => {
                    // Unknown kwarg for print - ignore
                }
            }
        }

        let dummy_local = self.alloc_and_add_local(Type::None, mir_func);

        // Set stderr if needed
        if use_stderr {
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_SET_STDERR),
                args: vec![],
            });
        }

        for (i, arg_id) in args.iter().enumerate() {
            let arg_expr = &hir_module.exprs[*arg_id];
            let arg_operand = self.lower_expr(arg_expr, hir_module, mir_func)?;
            // Use get_expr_type for proper type inference, but also check the local's
            // actual storage type — Union locals store boxed pointers even when inference
            // narrows the type to a primitive.
            let mut arg_type = self.get_expr_type(arg_expr, hir_module);
            if let mir::Operand::Local(id) = &arg_operand {
                if let Some(local) = mir_func.locals.get(id) {
                    if local.ty.is_union() {
                        arg_type = local.ty.clone();
                    }
                }
            }

            // For exception instances, convert to string via rt_exc_instance_str, then print
            if matches!(&arg_type, Type::BuiltinException(_)) {
                let str_local = self.alloc_and_add_local(Type::Str, mir_func);
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: str_local,
                    func: mir::RuntimeFunc::ExcInstanceStr,
                    args: vec![arg_operand],
                });
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_STR_OBJ),
                    args: vec![mir::Operand::Local(str_local)],
                });
            }
            // For class instances, convert to string via __str__/__repr__ first,
            // then print the resulting string (matches CPython behavior)
            else if let Type::Class { class_id, .. } = &arg_type {
                let str_local = self.alloc_and_add_local(Type::Str, mir_func);
                if let Some(class_info) = self.get_class_info(class_id) {
                    if let Some(str_func) = class_info.get_dunder_func("__str__") {
                        self.emit_instruction(mir::InstructionKind::CallDirect {
                            dest: str_local,
                            func: str_func,
                            args: vec![arg_operand],
                        });
                    } else if let Some(repr_func) = class_info.get_dunder_func("__repr__") {
                        self.emit_instruction(mir::InstructionKind::CallDirect {
                            dest: str_local,
                            func: repr_func,
                            args: vec![arg_operand],
                        });
                    } else if class_info.is_exception_class {
                        // Exception class without __str__/__repr__ - extract message
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: str_local,
                            func: mir::RuntimeFunc::ExcInstanceStr,
                            args: vec![arg_operand],
                        });
                    } else {
                        self.emit_instruction(mir::InstructionKind::RuntimeCall {
                            dest: str_local,
                            func: mir::RuntimeFunc::Call(
                                &pyaot_core_defs::runtime_func_def::RT_OBJ_DEFAULT_REPR,
                            ),
                            args: vec![arg_operand],
                        });
                    }
                } else {
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: str_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_OBJ_DEFAULT_REPR,
                        ),
                        args: vec![arg_operand],
                    });
                }
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_STR_OBJ),
                    args: vec![mir::Operand::Local(str_local)],
                });
            } else if matches!(&arg_type, Type::None) {
                // PrintValue(None) stays as special variant (no argument)
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::PrintValue(PrintKind::None),
                    args: vec![],
                });
            } else {
                // Select descriptor based on type
                let print_def = crate::type_dispatch::select_print_func(&arg_type);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: dummy_local,
                    func: mir::RuntimeFunc::Call(print_def),
                    args: vec![arg_operand],
                });
            }

            // Print separator between arguments (not after last)
            if i < args.len() - 1 {
                if let Some(ref sep) = sep_operand {
                    // Custom separator - use StrObj for heap strings
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_STR_OBJ),
                        args: vec![sep.clone()],
                    });
                } else {
                    // Default separator (space)
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: dummy_local,
                        func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_SEP),
                        args: vec![],
                    });
                }
            }
        }

        // Print end string
        if let Some(ref end) = end_operand {
            // Custom end - use StrObj for heap strings
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_STR_OBJ),
                args: vec![end.clone()],
            });
        } else {
            // Default end (newline)
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_NEWLINE),
                args: vec![],
            });
        }

        // Restore stdout if stderr was used
        if use_stderr {
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_SET_STDOUT),
                args: vec![],
            });
        }

        // Flush if requested
        if need_flush {
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(&runtime_func_def::RT_PRINT_FLUSH),
                args: vec![],
            });
        }

        Ok(mir::Operand::Constant(mir::Constant::None))
    }

    /// Lower input(prompt="") -> str
    pub(super) fn lower_input(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Get prompt argument (default is empty string)
        let prompt_operand = if args.is_empty() {
            // Create empty string for default prompt
            let empty_str_local = self.alloc_and_add_local(Type::Str, mir_func);
            let empty = self.intern("");
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: empty_str_local,
                func: mir::RuntimeFunc::MakeStr,
                args: vec![mir::Operand::Constant(mir::Constant::Str(empty))],
            });
            mir::Operand::Local(empty_str_local)
        } else {
            let prompt_expr = &hir_module.exprs[args[0]];
            self.lower_expr(prompt_expr, hir_module, mir_func)?
        };

        let result_local = self.alloc_and_add_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result_local,
            func: mir::RuntimeFunc::Call(&runtime_func_def::RT_INPUT),
            args: vec![prompt_operand],
        });

        Ok(mir::Operand::Local(result_local))
    }
}
