//! File I/O builtin lowering: open()

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower the open() builtin call
    pub(crate) fn lower_open(
        &mut self,
        args: &[hir::ExprId],
        kwargs: &[hir::KeywordArg],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // open(filename) or open(filename, mode) or open(filename, mode, encoding=...)
        if args.is_empty() {
            // No filename provided - this would be a TypeError in Python
            // Return None for now (the error would be caught at runtime)
            return Ok(mir::Operand::Constant(mir::Constant::None));
        }

        // Lower the filename argument
        let filename_expr = &hir_module.exprs[args[0]];
        let filename_op = self.lower_expr(filename_expr, hir_module, mir_func)?;

        // Lower the mode argument (default to "r" if not provided)
        let mode_op = if args.len() > 1 {
            let mode_expr = &hir_module.exprs[args[1]];
            self.lower_expr(mode_expr, hir_module, mir_func)?
        } else {
            self.make_str_constant("r", mir_func)
        };

        // Extract encoding= kwarg (default to null pointer = utf-8)
        let encoding_op = {
            let mut enc_op = None;
            for kwarg in kwargs {
                let name = self.resolve(kwarg.name);
                if name == "encoding" {
                    let enc_expr = &hir_module.exprs[kwarg.value];
                    enc_op = Some(self.lower_expr(enc_expr, hir_module, mir_func)?);
                }
            }
            // Use Int(0) as null pointer — None would lower to i8(0) which doesn't match i64 ABI
            enc_op.unwrap_or(mir::Operand::Constant(mir::Constant::Int(0)))
        };

        // Create local for result
        let result = self.alloc_and_add_local(Type::File, mir_func);

        // Call rt_file_open(filename, mode, encoding)
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result,
            func: mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_OPEN),
            args: vec![filename_op, mode_op, encoding_op],
        });

        Ok(mir::Operand::Local(result))
    }

    /// Helper: create a heap-allocated string constant
    fn make_str_constant(&mut self, s: &str, mir_func: &mut mir::Function) -> mir::Operand {
        let interned = self.intern(s);
        let local = self.alloc_and_add_local(Type::Str, mir_func);
        let const_local = self.alloc_stack_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::Const {
            dest: const_local,
            value: mir::Constant::Str(interned),
        });

        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: local,
            func: mir::RuntimeFunc::MakeStr,
            args: vec![mir::Operand::Local(const_local)],
        });

        mir::Operand::Local(local)
    }
}
