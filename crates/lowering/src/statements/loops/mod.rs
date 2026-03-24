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
use pyaot_types::Type;
use pyaot_utils::{BlockId, VarId};

use crate::context::Lowering;
use crate::utils::get_iterable_info;

impl<'a> Lowering<'a> {
    /// Lower a for loop statement
    pub(crate) fn lower_for(
        &mut self,
        target: VarId,
        iter: hir::ExprId,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_expr = &hir_module.exprs[iter];

        // Detect range() builtin call
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            args,
            ..
        } = &iter_expr.kind
        {
            self.lower_for_range(target, args, body, else_block, hir_module, mir_func)?;
        } else {
            // Handle iterables (list, tuple, dict, str, set, bytes)
            let iter_type = self.get_expr_type(iter_expr, hir_module);

            // Check for class with __iter__/__next__ (iterator protocol)
            if let Type::Class { class_id, .. } = &iter_type {
                let has_iter = self
                    .get_class_info(class_id)
                    .and_then(|info| info.iter_func)
                    .is_some();
                if has_iter {
                    return self.lower_for_class_iterator(
                        target, iter_expr, &iter_type, body, else_block, hir_module, mir_func,
                    );
                }
            }

            if let Some((kind, elem_type)) = get_iterable_info(&iter_type) {
                self.lower_for_iterable(
                    target, iter_expr, kind, elem_type, body, else_block, hir_module, mir_func,
                )?;
            } else {
                return Err(pyaot_diagnostics::CompilerError::type_error(
                    format!("cannot iterate over type '{:?}'", iter_type),
                    iter_expr.span,
                ));
            }
        }

        Ok(())
    }

    /// Lower a for loop with tuple unpacking: for a, b in items
    /// Dispatches to optimized enumerate path or general tuple unpack path
    pub(crate) fn lower_for_unpack(
        &mut self,
        targets: &[VarId],
        iter: hir::ExprId,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_expr = &hir_module.exprs[iter];

        // Detect enumerate() builtin call with 2 targets → optimized path
        if targets.len() == 2 {
            if let hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Enumerate,
                args: enum_args,
                kwargs: enum_kwargs,
            } = &iter_expr.kind
            {
                return self.lower_for_enumerate_optimized(
                    targets,
                    enum_args,
                    enum_kwargs,
                    body,
                    else_block,
                    hir_module,
                    mir_func,
                );
            }
        }

        // General tuple unpack: for a, b in list_of_tuples
        self.lower_for_unpack_general(targets, iter_expr, body, else_block, hir_module, mir_func)
    }

    /// Lower a for loop with starred unpacking: for first, *rest, last in items
    /// Public entry point that takes ExprId (delegates to implementation in starred_unpacking.rs)
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_for_unpack_starred_dispatch(
        &mut self,
        before_star: &[VarId],
        starred: Option<&VarId>,
        after_star: &[VarId],
        iter: hir::ExprId,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_expr = &hir_module.exprs[iter];
        self.lower_for_unpack_starred(
            before_star,
            starred.copied(),
            after_star,
            iter_expr,
            body,
            else_block,
            hir_module,
            mir_func,
        )
    }

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
