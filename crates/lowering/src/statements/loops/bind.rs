//! Unified `for TARGET in ITER:` lowering driven by `hir::BindingTarget`.
//!
//! Single entry point [`Lowering::lower_for_bind`] handles every binding shape
//! that the HIR can express on a `for`-loop target. It dispatches:
//!
//! * `Var(_)` with `range()` iter        → [`Lowering::lower_for_range`] (fast path).
//! * `Var(_)` with class iterator        → [`Lowering::lower_for_class_iterator`].
//! * `Var(_)` with general iterable      → [`Lowering::lower_for_iterable`].
//! * Flat `Tuple` of `Var`s with `enumerate()` iter → [`Lowering::lower_for_enumerate_optimized`].
//! * Flat `Tuple` of `Var`s, no `Starred`           → [`Lowering::lower_for_unpack_general`].
//! * Flat `Tuple` with one `Starred(Var)` → [`Lowering::lower_for_unpack_starred`].
//! * Anything else (nested patterns, attribute or subscript leaves, mixed
//!   shapes)            → general per-iteration loop emitted here, which
//!   calls [`Lowering::lower_binding_target`] on each item.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;
use crate::utils::{get_iterable_info, IterableKind};

impl<'a> Lowering<'a> {
    /// Lower `for TARGET in ITER:` for any [`hir::BindingTarget`] shape.
    pub(crate) fn lower_for_bind(
        &mut self,
        target: &hir::BindingTarget,
        iter: hir::ExprId,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_expr = &hir_module.exprs[iter];

        // Case 1: simple variable target.
        if let hir::BindingTarget::Var(target_var) = target {
            // range() fast path
            if let hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Range,
                args,
                ..
            } = &iter_expr.kind
            {
                return self.lower_for_range(
                    *target_var,
                    args,
                    body,
                    else_block,
                    hir_module,
                    mir_func,
                );
            }
            // class-iterator protocol
            let iter_type = self.get_type_of_expr_id(iter, hir_module);
            if let Type::Class { class_id, .. } = &iter_type {
                let has_iter = self
                    .get_class_info(class_id)
                    .and_then(|info| info.get_dunder_func("__iter__"))
                    .is_some();
                if has_iter {
                    return self.lower_for_class_iterator(
                        *target_var,
                        iter_expr,
                        &iter_type,
                        body,
                        else_block,
                        hir_module,
                        mir_func,
                    );
                }
            }
            // General iterable
            if let Some((kind, elem_type)) = get_iterable_info(&iter_type) {
                return self.lower_for_iterable(
                    *target_var,
                    iter,
                    kind,
                    elem_type,
                    body,
                    else_block,
                    hir_module,
                    mir_func,
                );
            }
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!("cannot iterate over type '{:?}'", iter_type),
                iter_expr.span,
            ));
        }

        // Cases 2+3: Tuple target → classify.
        if let hir::BindingTarget::Tuple { elts, .. } = target {
            if let Some(simple) = simple_var_unpack(elts) {
                match simple {
                    SimpleVarUnpack::Flat(targets) => {
                        // enumerate() fast path
                        if targets.len() == 2 {
                            if let hir::ExprKind::BuiltinCall {
                                builtin: hir::Builtin::Enumerate,
                                args: enum_args,
                                kwargs: enum_kwargs,
                            } = &iter_expr.kind
                            {
                                return self.lower_for_enumerate_optimized(
                                    &targets,
                                    enum_args,
                                    enum_kwargs,
                                    body,
                                    else_block,
                                    hir_module,
                                    mir_func,
                                );
                            }
                        }
                        // General tuple unpack
                        return self.lower_for_unpack_general(
                            &targets, iter, body, else_block, hir_module, mir_func,
                        );
                    }
                    SimpleVarUnpack::Starred {
                        before,
                        starred,
                        after,
                    } => {
                        return self.lower_for_unpack_starred(
                            &before, starred, &after, iter, body, else_block, hir_module, mir_func,
                        );
                    }
                }
            }
        }

        // Fall-through: general BindingTarget (nested, attr-leaf, index-leaf)
        self.lower_for_bind_general(target, iter, body, else_block, hir_module, mir_func)
    }

    /// General-purpose loop that materialises each iterable item into a
    /// temporary and binds it via [`Self::lower_binding_target`].
    fn lower_for_bind_general(
        &mut self,
        target: &hir::BindingTarget,
        iter_id: hir::ExprId,
        body: &[hir::StmtId],
        else_block: &[hir::StmtId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_expr = &hir_module.exprs[iter_id];
        let iter_type = self.get_type_of_expr_id(iter_id, hir_module);

        // Resolve element type and iteration kind. For unknown types we
        // could in principle fall back to the iterator protocol, but the
        // shapes that reach this path (nested or attribute leaves) only
        // come from the new BindingTarget surface; if the iterable type
        // is opaque, surface a clear error.
        let (kind, elem_type) = match get_iterable_info(&iter_type) {
            Some(info) => info,
            None => {
                return Err(pyaot_diagnostics::CompilerError::type_error(
                    format!(
                        "cannot iterate over type '{:?}' in for-loop with binding target",
                        iter_type
                    ),
                    iter_expr.span,
                ));
            }
        };

        // Iterator-backed iterables (dict/set/iterator/file) are not yet
        // supported by the general path. The legacy fast-paths handle them
        // for the simple cases; in the general case they would need a
        // protocol-style loop.
        if matches!(
            kind,
            IterableKind::Dict | IterableKind::Set | IterableKind::Iterator | IterableKind::File
        ) {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                format!(
                    "for-loop with non-trivial binding target over '{:?}' not yet supported",
                    iter_type
                ),
                iter_expr.span,
            ));
        }

        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: iter_local,
            src: iter_operand,
        });

        let len_func = match kind {
            IterableKind::List => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_LEN)
            }
            IterableKind::Tuple => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_LEN)
            }
            IterableKind::Str => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STR_LEN_INT)
            }
            IterableKind::Bytes => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BYTES_LEN)
            }
            _ => unreachable!("guarded above"),
        };
        let len_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: len_local,
            func: len_func,
            args: vec![mir::Operand::Local(iter_local)],
        });

        let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: idx_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });

        let header_bb = self.new_block();
        let body_bb = self.new_block();
        let increment_bb = self.new_block();
        let exit_bb = self.new_block();
        let else_bb = if !else_block.is_empty() {
            Some(self.new_block())
        } else {
            None
        };

        let header_id = header_bb.id;
        let body_id = body_bb.id;
        let increment_id = increment_bb.id;
        let exit_id = exit_bb.id;
        let normal_exit_id = else_bb.as_ref().map(|bb| bb.id).unwrap_or(exit_id);

        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        // Header: idx < len ?
        self.push_block(header_bb);
        let cond_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: cond_local,
            op: mir::BinOp::Lt,
            left: mir::Operand::Local(idx_local),
            right: mir::Operand::Local(len_local),
        });
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cond_local),
            then_block: body_id,
            else_block: normal_exit_id,
        };

        // Body: extract item and bind into target.
        self.push_block(body_bb);
        let get_func = match kind {
            IterableKind::List => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_GET)
            }
            IterableKind::Tuple => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_GET)
            }
            IterableKind::Str => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STR_GETCHAR)
            }
            IterableKind::Bytes => {
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_BYTES_GET)
            }
            _ => unreachable!("guarded above"),
        };
        let item_local = self.emit_runtime_call(
            get_func,
            vec![
                mir::Operand::Local(iter_local),
                mir::Operand::Local(idx_local),
            ],
            elem_type.clone(),
            mir_func,
        );

        self.lower_binding_target(
            target,
            mir::Operand::Local(item_local),
            &elem_type,
            hir_module,
            mir_func,
        )?;

        self.push_loop(increment_id, exit_id);
        for stmt_id in body {
            let stmt = &hir_module.stmts[*stmt_id];
            self.lower_stmt(stmt, hir_module, mir_func)?;
        }
        self.pop_loop();
        if !self.current_block_has_terminator() {
            self.current_block_mut().terminator = mir::Terminator::Goto(increment_id);
        }

        // Increment.
        self.push_block(increment_bb);
        let inc_idx = self.alloc_and_add_local(Type::Int, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: inc_idx,
            op: mir::BinOp::Add,
            left: mir::Operand::Local(idx_local),
            right: mir::Operand::Constant(mir::Constant::Int(1)),
        });
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: idx_local,
            src: mir::Operand::Local(inc_idx),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(header_id);

        if let Some(else_bb) = else_bb {
            self.push_block(else_bb);
            self.lower_loop_else(else_block, exit_id, hir_module, mir_func)?;
        }
        self.push_block(exit_bb);
        Ok(())
    }
}

/// Recognises a flat `Tuple { elts }` whose every leaf is `Var(VarId)` —
/// optionally with one `Starred(Var)` slot. Returns `None` for any nested
/// pattern, attribute/subscript leaf, or multi-starred pattern (caught
/// earlier by the validator), forcing the general path.
fn simple_var_unpack(elts: &[hir::BindingTarget]) -> Option<SimpleVarUnpack> {
    let mut before: Vec<VarId> = Vec::new();
    let mut starred: Option<VarId> = None;
    let mut after: Vec<VarId> = Vec::new();
    let mut seen_star = false;

    for elt in elts {
        match elt {
            hir::BindingTarget::Var(vid) => {
                if seen_star {
                    after.push(*vid);
                } else {
                    before.push(*vid);
                }
            }
            hir::BindingTarget::Starred { inner, .. } => {
                if seen_star {
                    return None; // validator should have rejected this
                }
                if let hir::BindingTarget::Var(vid) = inner.as_ref() {
                    starred = Some(*vid);
                    seen_star = true;
                } else {
                    return None; // nested or non-Var inside starred
                }
            }
            _ => return None,
        }
    }

    if seen_star {
        Some(SimpleVarUnpack::Starred {
            before,
            starred,
            after,
        })
    } else {
        Some(SimpleVarUnpack::Flat(before))
    }
}

enum SimpleVarUnpack {
    Flat(Vec<VarId>),
    Starred {
        before: Vec<VarId>,
        starred: Option<VarId>,
        after: Vec<VarId>,
    },
}
