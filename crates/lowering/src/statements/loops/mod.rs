//! For loop statement lowering
//!
//! Handles: For loops over range() and iterables (list, tuple, dict, str, set, bytes)
//!
//! This module is organized into submodules by loop type:
//! - `range`: Range loop lowering (for x in range(...))
//! - `iterable`: Iterable loop lowering (for x in list/tuple/dict/str/set/bytes)
//! - `enumerate`: Enumerate optimization (for i, v in enumerate(...))
//! - `unpacking`: Tuple unpacking (for a, b in list_of_tuples)
//! - `starred_unpacking`: Starred unpacking (for first, *rest, last in items)
//! - `iterator`: Iterator protocol (for x in generator)

mod bind;
mod class_iterator;
mod enumerate;
mod iterable;
mod iterator;
mod range;
mod starred_unpacking;
mod unpacking;

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_utils::BlockId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Helper to emit else block for for/while...else.
    /// Emits else block statements and jumps to exit_bb if else_block is non-empty.
    pub(crate) fn lower_loop_else(
        &mut self,
        else_block: &[hir::StmtId],
        exit_id: BlockId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        for stmt_id in else_block {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(exit_id);
        }
        Ok(())
    }
}
