//! File I/O builtin lowering: open()

use pyaot_diagnostics::Result;
use pyaot_hir::{self as hir, ExprKind};
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

        // Determine text/binary from the mode literal so the returned local
        // carries the correct `Type::File(bool)`. The frontend (ast_to_hir)
        // does the same check for `expr.ty`; doing it again here keeps the
        // MIR local's declared type consistent with later method dispatch.
        let is_binary = {
            let mut b = false;
            if args.len() > 1 {
                if let ExprKind::Str(interned) = &hir_module.exprs[args[1]].kind {
                    b = self.interner.resolve(*interned).contains('b');
                }
            }
            for kwarg in kwargs {
                if self.resolve(kwarg.name) == "mode" {
                    if let ExprKind::Str(interned) = &hir_module.exprs[kwarg.value].kind {
                        b = self.interner.resolve(*interned).contains('b');
                    }
                }
            }
            b
        };

        // Call rt_file_open(filename, mode, encoding)
        let result = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_FILE_OPEN),
            vec![filename_op, mode_op, encoding_op],
            Type::File(is_binary),
            mir_func,
        );

        Ok(mir::Operand::Local(result))
    }

    /// Helper: create a heap-allocated string constant
    fn make_str_constant(&mut self, s: &str, mir_func: &mut mir::Function) -> mir::Operand {
        let interned = self.intern(s);
        let const_local = self.alloc_stack_local(Type::Str, mir_func);

        self.emit_instruction(mir::InstructionKind::Const {
            dest: const_local,
            value: mir::Constant::Str(interned),
        });

        let local = self.emit_runtime_call(
            mir::RuntimeFunc::MakeStr,
            vec![mir::Operand::Local(const_local)],
            Type::Str,
            mir_func,
        );

        mir::Operand::Local(local)
    }
}
