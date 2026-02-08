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
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // open(filename) or open(filename, mode)
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
            // Create a constant "r" string
            let mode_str = self.intern("r");
            let mode_local = self.alloc_and_add_local(Type::Str, mir_func);

            // Make the string constant first (not a GC root since it's just a static pointer)
            let const_local = self.alloc_stack_local(Type::Str, mir_func);

            self.emit_instruction(mir::InstructionKind::Const {
                dest: const_local,
                value: mir::Constant::Str(mode_str),
            });

            // Allocate string on heap
            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: mode_local,
                func: mir::RuntimeFunc::MakeStr,
                args: vec![mir::Operand::Local(const_local)],
            });

            mir::Operand::Local(mode_local)
        };

        // Create local for result
        let result = self.alloc_and_add_local(Type::File, mir_func);

        // Call rt_file_open
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: result,
            func: mir::RuntimeFunc::FileOpen,
            args: vec![filename_op, mode_op],
        });

        Ok(mir::Operand::Local(result))
    }
}
