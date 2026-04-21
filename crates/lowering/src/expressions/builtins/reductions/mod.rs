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

/// How the accumulator is seeded for a class-element `sum`.
///
/// - `Default` — no `start=` argument; pull the first element.
/// - `SameClass` — explicit start of the same class as the elements.
/// - `Primitive` — numeric start (`int` / `float` / `bool`); bootstrap
///   via `dispatch_class_binop(start + first_elem)` which routes to the
///   element's `__radd__` and promotes the accumulator to class-typed.
pub(crate) enum StartSeed {
    Default,
    SameClass(mir::Operand),
    Primitive { op: mir::Operand, ty: Type },
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
        let iterable_type = self.seed_expr_type(args[0], hir_module);
        let elem_ty = match &iterable_type {
            Type::List(t) | Type::Iterator(t) | Type::Set(t) => (**t).clone(),
            _ => return Ok(None),
        };
        let class_ty = match &elem_ty {
            Type::Class { .. } => elem_ty.clone(),
            _ => return Ok(None),
        };

        // Resolve `start=` kind:
        // - same-class instance → seed `acc` directly with it
        // - primitive (int / float / bool) → bootstrap via dispatch:
        //   `acc = primitive + first_elem` through `dispatch_class_binop`,
        //   which falls through to `first_elem.__radd__(primitive)` and
        //   returns a class-typed result. Subsequent iterations fold
        //   class + class via the forward dunder.
        // - anything else → give up; fall through to the numeric path.
        let start_operand = if args.len() > 1 {
            let start_ty = self.seed_expr_type(args[1], hir_module);
            let expr = &hir_module.exprs[args[1]];
            let op = self.lower_expr(expr, hir_module, mir_func)?;
            if start_ty == class_ty {
                StartSeed::SameClass(op)
            } else if matches!(start_ty, Type::Int | Type::Float | Type::Bool) {
                StartSeed::Primitive { op, ty: start_ty }
            } else {
                return Ok(None);
            }
        } else {
            StartSeed::Default
        };

        let iterable_operand =
            self.lower_expr_expecting(iterable_expr, None, hir_module, mir_func)?;

        // Materialise an iterator over the source so we don't care whether
        // the caller passed a list, set, or generator expression.
        let (iter_local, _iter_ty) =
            self.make_iter_from_operand(iterable_operand, &iterable_type, mir_func);

        self.lower_reduction_class_fold(
            iter_local,
            &class_ty,
            start_operand,
            ReducerKind::Add,
            hir_module,
            mir_func,
        )
        .map(Some)
    }

    /// Fold body shared by every class-element reduction. Seeds the
    /// accumulator per [`StartSeed`], then iterates via
    /// `RT_ITER_NEXT_NO_EXC` / `RT_GENERATOR_IS_EXHAUSTED` calling
    /// [`Self::dispatch_class_binop`] for every element so the §3.3.8
    /// state machine applies to each fold step.
    fn lower_reduction_class_fold(
        &mut self,
        iter_local: pyaot_utils::LocalId,
        class_ty: &Type,
        start: StartSeed,
        reducer: ReducerKind,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Accumulator is a heap-typed local (class instance) — must be a
        // GC root so the intermediate values produced by each dunder call
        // are visible to the shadow-stack walker.
        let acc_local = self.alloc_gc_local(class_ty.clone(), mir_func);

        let header_bb = self.new_block();
        let header_bb_id = header_bb.id;
        let body_bb = self.new_block();
        let body_bb_id = body_bb.id;
        let exit_bb = self.new_block();
        let exit_bb_id = exit_bb.id;

        match start {
            StartSeed::SameClass(op) => {
                // Direct seed: start instance is already class-typed.
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: acc_local,
                    src: op,
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);
            }
            StartSeed::Default => {
                self.seed_acc_from_first_elem(
                    iter_local,
                    class_ty,
                    acc_local,
                    header_bb_id,
                    exit_bb_id,
                    mir_func,
                );
            }
            StartSeed::Primitive { op, ty } => {
                // Bootstrap via `dispatch_class_binop(primitive + first_elem)`
                // which routes through `first_elem.__radd__(primitive)` and
                // returns a class-typed result. After this the accumulator
                // is class-typed and the fold continues as usual.
                self.seed_acc_from_primitive_plus_first(
                    iter_local,
                    class_ty,
                    acc_local,
                    op,
                    &ty,
                    reducer,
                    header_bb_id,
                    exit_bb_id,
                    hir_module,
                    mir_func,
                );
            }
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

    /// Seed `acc_local` with the first element of the iterator. Branches
    /// to `header_bb_id` on success; on empty iterable, writes a null
    /// placeholder into `acc_local` and branches to `exit_bb_id`.
    #[allow(clippy::too_many_arguments)]
    fn seed_acc_from_first_elem(
        &mut self,
        iter_local: pyaot_utils::LocalId,
        class_ty: &Type,
        acc_local: pyaot_utils::LocalId,
        header_bb_id: pyaot_utils::BlockId,
        exit_bb_id: pyaot_utils::BlockId,
        mir_func: &mut mir::Function,
    ) {
        let first_bb = self.new_block();
        let first_bb_id = first_bb.id;
        self.current_block_mut().terminator = mir::Terminator::Goto(first_bb_id);
        self.push_block(first_bb);
        let first_val = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            class_ty.clone(),
            mir_func,
        );
        let first_exhausted = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );
        let raise_bb = self.new_block();
        let raise_bb_id = raise_bb.id;
        let seed_bb = self.new_block();
        let seed_bb_id = seed_bb.id;
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(first_exhausted),
            then_block: raise_bb_id,
            else_block: seed_bb_id,
        };
        // Empty iterable with default `start=0` — null placeholder.
        self.push_block(raise_bb);
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

    /// Bootstrap the accumulator from `primitive + first_elem` — used
    /// when `sum(list, 0)` is called with an int/float `start` and a
    /// class element type. Routes through `dispatch_class_binop` so the
    /// §3.3.8 dispatch (including reflected `__radd__`) applies to the
    /// first element, promoting the accumulator to class-typed.
    ///
    /// On empty iterable: emit null placeholder and jump to exit
    /// (CPython would return the `start` value unchanged; we're forced
    /// into the class-typed accumulator slot, so this is a best-effort
    /// parity — documented in INSIGHTS).
    #[allow(clippy::too_many_arguments)]
    fn seed_acc_from_primitive_plus_first(
        &mut self,
        iter_local: pyaot_utils::LocalId,
        class_ty: &Type,
        acc_local: pyaot_utils::LocalId,
        primitive_op: mir::Operand,
        primitive_ty: &Type,
        reducer: ReducerKind,
        header_bb_id: pyaot_utils::BlockId,
        exit_bb_id: pyaot_utils::BlockId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) {
        let first_bb = self.new_block();
        let first_bb_id = first_bb.id;
        self.current_block_mut().terminator = mir::Terminator::Goto(first_bb_id);
        self.push_block(first_bb);
        let first_val = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            class_ty.clone(),
            mir_func,
        );
        let first_exhausted = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );
        let raise_bb = self.new_block();
        let raise_bb_id = raise_bb.id;
        let seed_bb = self.new_block();
        let seed_bb_id = seed_bb.id;
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(first_exhausted),
            then_block: raise_bb_id,
            else_block: seed_bb_id,
        };
        // Empty iterable with primitive start: can't represent the
        // primitive in a class-typed slot. Null placeholder.
        self.push_block(raise_bb);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: acc_local,
            src: mir::Operand::Constant(mir::Constant::Int(0)),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(exit_bb_id);

        // Seed: acc = primitive + first_elem via dispatch_class_binop.
        // With a primitive left operand the dispatch skips subclass-first
        // and forward-on-left (no class), falls to the right operand's
        // reflected dunder (`first_elem.__radd__(primitive)`) — returns
        // class-typed.
        self.push_block(seed_bb);
        let bootstrap = self
            .dispatch_class_binop(
                reducer.binop(),
                primitive_op,
                primitive_ty,
                mir::Operand::Local(first_val),
                class_ty,
                class_ty,
                hir_module,
                mir_func,
            )
            .unwrap_or(mir::Operand::Local(first_val));
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: acc_local,
            src: bootstrap,
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);
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

    /// `min(iterable)` / `max(iterable)` over an iterable whose element
    /// type is a user class. Dispatches the comparison through the
    /// rich-comparison dunders:
    ///
    /// - `min`: prefers `elem.__lt__(best)`; falls back to `best.__gt__(elem)`.
    /// - `max`: prefers `elem.__gt__(best)`; falls back to `best.__lt__(elem)`.
    ///
    /// Fold pattern: seed with first element; for each remaining elem,
    /// call the dunder; if truthy, replace the running best.
    ///
    /// Returns `Ok(None)` if the caller should fall through (non-class
    /// element, or no suitable comparison dunder is defined on the class).
    pub(in crate::expressions::builtins) fn try_lower_minmax_class_elem(
        &mut self,
        arg: hir::ExprId,
        is_min: bool,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<Option<mir::Operand>> {
        let iterable_type = self.seed_expr_type(arg, hir_module);
        let elem_ty = match &iterable_type {
            Type::List(t) | Type::Iterator(t) | Type::Set(t) => (**t).clone(),
            Type::Tuple(types) => {
                // Only homogeneous tuples can be treated as class-iterables;
                // mixed-element tuples fall through to the primitive path.
                let Some(first) = types.first() else {
                    return Ok(None);
                };
                if types.iter().any(|t| t != first) {
                    return Ok(None);
                }
                first.clone()
            }
            _ => return Ok(None),
        };
        let class_ty = match &elem_ty {
            Type::Class { .. } => elem_ty.clone(),
            _ => return Ok(None),
        };

        // Resolve the comparison dunder pair. For min we prefer `__lt__`
        // on the left (elem), fallback to `__gt__` on right (best); for
        // max we flip. If neither side has a suitable dunder, give up
        // and return None so the caller can surface a clearer error.
        let class_id = match &class_ty {
            Type::Class { class_id, .. } => *class_id,
            _ => unreachable!(),
        };
        let class_info = match self.get_class_info(&class_id) {
            Some(ci) => ci,
            None => return Ok(None),
        };
        let (primary, primary_swapped, fallback, fallback_swapped) = if is_min {
            // min: elem < best. Forward = elem.__lt__(best); swap-fallback = best.__gt__(elem).
            (
                class_info.get_dunder_func("__lt__"),
                false,
                class_info.get_dunder_func("__gt__"),
                true,
            )
        } else {
            // max: elem > best. Forward = elem.__gt__(best); swap-fallback = best.__lt__(elem).
            (
                class_info.get_dunder_func("__gt__"),
                false,
                class_info.get_dunder_func("__lt__"),
                true,
            )
        };
        let (cmp_func, swap_args) = match (primary, fallback) {
            (Some(f), _) => (f, primary_swapped),
            (None, Some(f)) => (f, fallback_swapped),
            (None, None) => return Ok(None),
        };

        let iterable_expr = &hir_module.exprs[arg];
        let iterable_operand =
            self.lower_expr_expecting(iterable_expr, None, hir_module, mir_func)?;
        let (iter_local, _iter_ty) =
            self.make_iter_from_operand(iterable_operand, &iterable_type, mir_func);

        self.lower_minmax_class_fold(
            iter_local, &class_ty, cmp_func, swap_args, is_min, hir_module, mir_func,
        )
        .map(Some)
    }

    /// Fold loop shared by `min` / `max` on user classes. `swap_args`
    /// controls the call-site order: `false` → `dunder(elem, best)`,
    /// `true` → `dunder(best, elem)` (for the swapped-fallback path).
    /// `is_min` selects the ValueError message on empty iterables.
    #[allow(clippy::too_many_arguments)]
    fn lower_minmax_class_fold(
        &mut self,
        iter_local: pyaot_utils::LocalId,
        class_ty: &Type,
        cmp_func: pyaot_utils::FuncId,
        swap_args: bool,
        is_min: bool,
        _hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let best_local = self.alloc_gc_local(class_ty.clone(), mir_func);

        let seed_bb = self.new_block();
        let seed_bb_id = seed_bb.id;
        let header_bb = self.new_block();
        let header_bb_id = header_bb.id;
        let body_bb = self.new_block();
        let body_bb_id = body_bb.id;
        let update_bb = self.new_block();
        let update_bb_id = update_bb.id;
        let continue_bb = self.new_block();
        let continue_bb_id = continue_bb.id;
        let exit_bb = self.new_block();
        let exit_bb_id = exit_bb.id;

        // Entry: seed `best` with the first element.
        self.current_block_mut().terminator = mir::Terminator::Goto(seed_bb_id);
        self.push_block(seed_bb);
        let first_val = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            class_ty.clone(),
            mir_func,
        );
        let first_exhausted = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_GENERATOR_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );
        // Empty iterable → ValueError, matching CPython (§G.12).
        let raise_bb = self.new_block();
        let raise_bb_id = raise_bb.id;
        let seed_ok_bb = self.new_block();
        let seed_ok_bb_id = seed_ok_bb.id;
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(first_exhausted),
            then_block: raise_bb_id,
            else_block: seed_ok_bb_id,
        };
        self.push_block(raise_bb);
        let msg = if is_min {
            "min() arg is an empty sequence"
        } else {
            "max() arg is an empty sequence"
        };
        let msg_interned = self.interner.intern(msg);
        self.current_block_mut().terminator = mir::Terminator::Raise {
            exc_type: pyaot_core_defs::exceptions::BuiltinExceptionKind::ValueError.tag(),
            message: Some(mir::Operand::Constant(mir::Constant::Str(msg_interned))),
            cause: None,
            suppress_context: false,
        };

        self.push_block(seed_ok_bb);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: best_local,
            src: mir::Operand::Local(first_val),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);

        // Loop header: fetch next, check exhausted.
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

        // Body: call cmp dunder; branch on result.
        self.push_block(body_bb);
        let cmp_result = self.alloc_and_add_local(Type::Bool, mir_func);
        let (left, right) = if swap_args {
            (
                mir::Operand::Local(best_local),
                mir::Operand::Local(elem_local),
            )
        } else {
            (
                mir::Operand::Local(elem_local),
                mir::Operand::Local(best_local),
            )
        };
        self.emit_instruction(mir::InstructionKind::CallDirect {
            dest: cmp_result,
            func: cmp_func,
            args: vec![left, right],
        });
        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(cmp_result),
            then_block: update_bb_id,
            else_block: continue_bb_id,
        };

        // Update branch: best := elem.
        self.push_block(update_bb);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: best_local,
            src: mir::Operand::Local(elem_local),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(continue_bb_id);

        // Continue: back to header.
        self.push_block(continue_bb);
        self.current_block_mut().terminator = mir::Terminator::Goto(header_bb_id);

        self.push_block(exit_bb);
        Ok(mir::Operand::Local(best_local))
    }
}
