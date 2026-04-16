//! Area C §C.3 — unified reduction helper for `sum()` / `min()` / `max()`
//! when the iterable's element type is a user class.
//!
//! For primitive elements the existing fast paths in `math::arithmetic::
//! lower_sum` and `math::minmax::lower_minmax_builtin` remain authoritative
//! — direct `BinOp::Add` / `BinOp::Lt` on i64 / f64 stays the most
//! efficient representation. When the element type is `Type::Class`, the
//! reduction must dispatch through the operator dunder protocol
//! (`__add__` / `__radd__` for `sum`, `__lt__` / `__gt__` for `min`/`max`)
//! — that's this module's job.
//!
//! The fold loop itself is deliberately simple: iterate via the standard
//! `RT_ITER_NEXT_NO_EXC` / `RT_GENERATOR_IS_EXHAUSTED` protocol (so
//! generator-expression sources work the same as lists), seed the
//! accumulator with the first element (CPython's `start=0` shortcut for
//! class elements — see docstring below), and call
//! [`Lowering::dispatch_class_binop`] (Area B machinery, extracted in
//! commit 3 of Area C) for every subsequent element. The full dispatch
//! state machine (subclass-first → forward → `NotImplemented` fallback →
//! reflected) is reused verbatim.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

/// Which reduction operation we're lowering. Only `Add` is wired up from
/// a built-in today (`sum`); `Mul`/`Min`/`Max` are future-compat.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReducerKind {
    Add,
    #[allow(dead_code)]
    Mul,
    #[allow(dead_code)]
    Min,
    #[allow(dead_code)]
    Max,
}

impl ReducerKind {
    /// Binary op used to dispatch the dunder for the class-element fold.
    /// `min`/`max` are future-compat placeholders — their comparison
    /// actually goes through `CmpOp` (not `BinOp`), so the extracted
    /// `dispatch_class_binop` helper isn't directly reusable for them.
    /// They map to the closest `BinOp` for now and will be rewired when
    /// needed.
    fn binop(self) -> hir::BinOp {
        match self {
            ReducerKind::Add => hir::BinOp::Add,
            ReducerKind::Mul => hir::BinOp::Mul,
            ReducerKind::Min | ReducerKind::Max => hir::BinOp::Add,
        }
    }
}

impl<'a> Lowering<'a> {
    /// `sum()` over an iterable whose element type is a user class.
    ///
    /// Seeds the accumulator with the **first element** of the iterable
    /// instead of CPython's default `0`, then folds the rest via
    /// `acc.__add__(elem)` / dispatched through the Area B state machine.
    /// This mirrors CPython's own short-circuit: `0 + V(x)` would raise
    /// `NotImplemented` → then try `V(x).__radd__(0)` → `V(x)`; our
    /// shortcut skips the zero entirely. The accumulator type stays
    /// `class_ty` throughout, so no Union-boxing is needed.
    ///
    /// Returns `Ok(None)` if the caller should fall through to the
    /// existing numeric fast path (non-class element, or `start=` was
    /// provided and differs from the element type).
    pub(in crate::expressions::builtins) fn try_lower_sum_class_elem(
        &mut self,
        args: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Option<mir::Operand>> {
        if args.is_empty() {
            return Ok(None);
        }
        let iterable_expr = &hir_module.exprs[args[0]];
        let iterable_type = self.get_type_of_expr_id(args[0], hir_module);
        let elem_ty = match &iterable_type {
            Type::List(t) | Type::Iterator(t) | Type::Set(t) => (**t).clone(),
            _ => return Ok(None),
        };
        let class_ty = match &elem_ty {
            Type::Class { .. } => elem_ty.clone(),
            _ => return Ok(None),
        };

        // If `start=` was provided, fall back to Union-accumulator path —
        // currently unimplemented. User can pass an explicit instance of
        // the class (e.g. `sum(monies, Money(100))`) and then `start_ty`
        // equals `class_ty`, so the shortcut still applies.
        let start_op = if args.len() > 1 {
            let start_ty = self.get_type_of_expr_id(args[1], hir_module);
            if start_ty != class_ty {
                return Ok(None);
            }
            let expr = &hir_module.exprs[args[1]];
            Some(self.lower_expr(expr, hir_module, mir_func)?)
        } else {
            None
        };

        let iterable_operand = self.lower_expr(iterable_expr, hir_module, mir_func)?;

        // Materialise an iterator over the source so we don't care whether
        // the caller passed a list, set, or generator expression.
        let (iter_local, _iter_ty) =
            self.make_iter_from_operand(iterable_operand, &iterable_type, mir_func);

        self.lower_reduction_class_fold(
            iter_local,
            &class_ty,
            start_op,
            ReducerKind::Add,
            hir_module,
            mir_func,
        )
        .map(Some)
    }

    /// Fold body shared by every class-element reduction. Iterates via
    /// `RT_ITER_NEXT_NO_EXC` / `RT_GENERATOR_IS_EXHAUSTED`; seeds the
    /// accumulator with `start` (if provided) or the first element; then
    /// for each remaining element calls [`Self::dispatch_class_binop`]
    /// so the full §3.3.8 state machine applies.
    fn lower_reduction_class_fold(
        &mut self,
        iter_local: pyaot_utils::LocalId,
        class_ty: &Type,
        start: Option<mir::Operand>,
        reducer: ReducerKind,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Accumulator is a heap-typed local (class instance) — must be a
        // GC root so the intermediate values produced by each dunder call
        // are visible to the shadow-stack walker.
        let acc_local = self.alloc_gc_local(class_ty.clone(), mir_func);

        let first_bb = self.new_block();
        let first_bb_id = first_bb.id;
        let header_bb = self.new_block();
        let header_bb_id = header_bb.id;
        let body_bb = self.new_block();
        let body_bb_id = body_bb.id;
        let exit_bb = self.new_block();
        let exit_bb_id = exit_bb.id;

        // Entry: seed accumulator. If `start` was provided, use it; else
        // pull the first element from the iterator.
        if let Some(start_op) = start {
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: acc_local,
                src: start_op,
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);
        } else {
            self.current_block_mut().terminator = mir::Terminator::Goto(first_bb_id);
            self.push_block(first_bb);
            let first_val = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
                vec![mir::Operand::Local(iter_local)],
                class_ty.clone(),
                mir_func,
            );
            let first_exhausted = self.emit_runtime_call(
                mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED,
                ),
                vec![mir::Operand::Local(iter_local)],
                Type::Bool,
                mir_func,
            );
            // Empty iterable, no start provided → CPython's default is
            // `0`. Since no class element was produced, emit a 0-typed
            // fallback. The caller never hits this path because it
            // requires a user-class accumulator; document the edge case.
            let raise_empty_bb = self.new_block();
            let raise_empty_bb_id = raise_empty_bb.id;
            let seed_bb = self.new_block();
            let seed_bb_id = seed_bb.id;
            self.current_block_mut().terminator = mir::Terminator::Branch {
                cond: mir::Operand::Local(first_exhausted),
                then_block: raise_empty_bb_id,
                else_block: seed_bb_id,
            };

            self.push_block(raise_empty_bb);
            // Empty class-element iterable with default `start=0` — we
            // can't type-safely return the class default. Copy a null
            // pointer (treated as `None` by the runtime). Users who
            // expect this path should pass an explicit start.
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: acc_local,
                src: mir::Operand::Constant(mir::Constant::Int(0)),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(exit_bb_id);

            self.push_block(seed_bb);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: acc_local,
                src: mir::Operand::Local(first_val),
            });
            self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);
        }

        // Loop header: next(), check exhausted.
        self.push_block(header_bb);
        let elem_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            class_ty.clone(),
            mir_func,
        );
        let exhausted_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(exhausted_local),
            then_block: exit_bb_id,
            else_block: body_bb_id,
        };

        // Loop body: acc = dispatch_class_binop(acc, elem).
        self.push_block(body_bb);
        let new_acc = self
            .dispatch_class_binop(
                reducer.binop(),
                mir::Operand::Local(acc_local),
                class_ty,
                mir::Operand::Local(elem_local),
                class_ty,
                class_ty,
                hir_module,
                mir_func,
            )
            .unwrap_or(mir::Operand::Local(acc_local));
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: acc_local,
            src: new_acc,
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);

        self.push_block(exit_bb);
        Ok(mir::Operand::Local(acc_local))
    }

    /// Materialise an iterator over an arbitrary iterable operand. For a
    /// `Type::Iterator` source the operand already *is* an iterator; for
    /// List/Set sources we allocate the iterator via the runtime.
    fn make_iter_from_operand(
        &mut self,
        operand: mir::Operand,
        iterable_ty: &Type,
        mir_func: &mut mir::Function,
    ) -> (pyaot_utils::LocalId, Type) {
        let iter_ty = Type::Iterator(Box::new(iterable_ty.clone()));

        if matches!(iterable_ty, Type::Iterator(_)) {
            let iter_local = self.alloc_gc_local(iterable_ty.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: iter_local,
                src: operand,
            });
            return (iter_local, iterable_ty.clone());
        }

        let source_kind = match iterable_ty {
            Type::List(_) => mir::IterSourceKind::List,
            Type::Set(_) => mir::IterSourceKind::Set,
            Type::Tuple(_) => mir::IterSourceKind::Tuple,
            _ => mir::IterSourceKind::List,
        };
        let iter_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(source_kind.iterator_def(mir::IterDirection::Forward)),
            vec![operand],
            iter_ty.clone(),
            mir_func,
        );
        (iter_local, iter_ty)
    }
}
