//! §1.17b-c — Iterator dispatch for `StmtKind::IterSetup`,
//! `StmtKind::IterAdvance`, and `ExprKind::IterHasNext`, plus the
//! `ExprKind::MatchPattern` pattern-predicate lowering.
//!
//! Produced by the `cfg_build` bridge when lowering a tree `ForBind` into
//! its CFG form:
//!
//! ```text
//!   pre:        ...user stmts...; IterSetup(iter); Jump(header)
//!   header:     Branch(IterHasNext(iter), body, exit)
//!   body:       IterAdvance(iter, target); ...user body stmts...; Jump(header)
//!   exit:       ...
//! ```
//!
//! `IterSetup` must run exactly once in the pre-block (before the loop
//! enters the header). It selects a dispatch path based on the iterable
//! kind and caches the relevant locals in `CodeGenState::iter_cache`:
//!
//! - **Indexed** (List / Tuple): `rt_X_len` on setup, `idx` counter,
//!   `rt_X_get_typed` / `rt_X_get` on advance. Matches the tree walker's
//!   `lower_for_iterable` fast path — avoids the IteratorObj allocation
//!   and boxing round-trip for typed lists. `IterHasNext` becomes a
//!   plain `BinOp::Lt(idx, len)`.
//! - **Protocol** (Dict / Set / Str / Bytes / Range / Generator): create
//!   iterator via `rt_iter_X(iterable)` (or `rt_iter_range(start, stop,
//!   step)` for range). `IterHasNext` calls `rt_iter_is_exhausted` +
//!   NOT; `IterAdvance` calls `rt_iter_next_no_exc` + primitive unbox
//!   (Int/Float/Bool) + bind.

use pyaot_core_defs::runtime_func_def;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::{IterState, IterableKindCached, Lowering};
use crate::utils::{get_iterable_info, IterableKind};

impl<'a> Lowering<'a> {
    /// Lower `StmtKind::IterSetup { iter }` — pick the dispatch path
    /// (indexed for List/Tuple, protocol for everything else) and cache
    /// the relevant locals. Must run once in the pre-block.
    pub(crate) fn lower_iter_setup(
        &mut self,
        iter_id: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // Defensive: duplicate IterSetup means a bridge bug or user-code
        // pattern we haven't seen. No-op to keep first-winner semantics.
        if self.codegen.iter_cache.contains_key(&iter_id) {
            return Ok(());
        }

        let iter_expr = &hir_module.exprs[iter_id];

        // §1.17b-c — special case: `for i in range(...)` uses
        // `rt_iter_range(start, stop, step)` which takes 3 i64 args,
        // NOT the generic (iterable → iterator) pattern. Always goes
        // through the Protocol dispatch (range iterator yields raw
        // i64 values that `box_if_raw_int_iterator` boxes at
        // `rt_iter_next` time).
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            args,
            ..
        } = &iter_expr.kind
        {
            let (start, stop, step) =
                self.lower_range_args(args, iter_expr.span, hir_module, mir_func)?;
            let iter_local = self.emit_runtime_call(
                mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_RANGE),
                vec![start, stop, step],
                Type::HeapAny,
                mir_func,
            );
            self.codegen
                .iter_cache
                .insert(iter_id, IterState::Protocol { iter_local });
            return Ok(());
        }

        let iter_type = self.get_type_of_expr_id(iter_id, hir_module);
        let Some((kind, elem_type)) = get_iterable_info(&iter_type) else {
            return Err(CompilerError::type_error(
                format!(
                    "cannot iterate over type '{:?}' in IterSetup (no iterable info)",
                    iter_type
                ),
                iter_expr.span,
            ));
        };

        // §1.17b-c — indexed dispatch for List and Tuple. Matches
        // `lower_for_iterable`'s fast path: no IteratorObj, just len()
        // + get_typed().
        match kind {
            IterableKind::List | IterableKind::Tuple => {
                let container_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
                let container_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: container_local,
                    src: container_operand,
                });

                let len_func = match kind {
                    IterableKind::List => &runtime_func_def::RT_LIST_LEN,
                    IterableKind::Tuple => &runtime_func_def::RT_TUPLE_LEN,
                    _ => unreachable!(),
                };
                let len_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(len_func),
                    vec![mir::Operand::Local(container_local)],
                    Type::Int,
                    mir_func,
                );

                // Initialize idx = 0.
                let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: idx_local,
                    src: mir::Operand::Constant(mir::Constant::Int(0)),
                });

                self.codegen.iter_cache.insert(
                    iter_id,
                    IterState::Indexed {
                        container_local,
                        idx_local,
                        len_local,
                        elem_type,
                        kind: match kind {
                            IterableKind::List => IterableKindCached::List,
                            IterableKind::Tuple => IterableKindCached::Tuple,
                            _ => unreachable!(),
                        },
                    },
                );
                return Ok(());
            }
            _ => {}
        }

        // §1.17b-c — protocol dispatch for Dict / Set / Str / Bytes /
        // Iterator (generators). Use the generic iterator protocol.
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
        let rt_func = match kind {
            IterableKind::Dict => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_DICT),
            IterableKind::Set => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_SET),
            IterableKind::Str => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_STR),
            IterableKind::Bytes => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_BYTES),
            IterableKind::Iterator => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_GENERATOR),
            IterableKind::File => {
                return Err(CompilerError::type_error(
                    "IterSetup does not yet support file iteration — \
                     use `for line in f.readlines():` or fall back to tree \
                     lowering for now"
                        .to_string(),
                    iter_expr.span,
                ));
            }
            IterableKind::List | IterableKind::Tuple => unreachable!("handled above"),
        };
        let iter_local =
            self.emit_runtime_call(rt_func, vec![iter_operand], Type::HeapAny, mir_func);
        self.codegen
            .iter_cache
            .insert(iter_id, IterState::Protocol { iter_local });
        Ok(())
    }

    /// Lower `StmtKind::IterAdvance { iter, target }` — read the cached
    /// `IterState` and dispatch to the appropriate advance path.
    pub(crate) fn lower_iter_advance(
        &mut self,
        iter_id: hir::ExprId,
        target: &hir::BindingTarget,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let state = self
            .codegen
            .iter_cache
            .get(&iter_id)
            .cloned()
            .ok_or_else(|| {
                CompilerError::type_error(
                    format!(
                        "IterAdvance for expr {:?} without preceding IterSetup — \
                         CFG invariant violation",
                        iter_id
                    ),
                    hir_module.exprs[iter_id].span,
                )
            })?;

        match state {
            IterState::Indexed {
                container_local,
                idx_local,
                elem_type,
                kind,
                ..
            } => {
                // Emit typed get (primitive) or generic get (heap).
                // Mirrors the elem_kind_for_typed dispatch in
                // `lower_for_iterable`.
                let elem_kind_for_typed = match (kind, &elem_type) {
                    (IterableKindCached::List, Type::Int) => Some(mir::GetElementKind::Int),
                    _ => None,
                };

                let target_type = elem_type.clone();
                let value_local = self.alloc_and_add_local(target_type.clone(), mir_func);

                if let Some(elem_kind) = elem_kind_for_typed {
                    let kind_tag =
                        mir::Operand::Constant(mir::Constant::Int(elem_kind.to_tag() as i64));
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: value_local,
                        func: mir::RuntimeFunc::Call(&runtime_func_def::RT_LIST_GET_TYPED),
                        args: vec![
                            mir::Operand::Local(container_local),
                            mir::Operand::Local(idx_local),
                            kind_tag,
                        ],
                    });
                } else {
                    let get_func = match kind {
                        IterableKindCached::Tuple => {
                            crate::type_dispatch::tuple_get_func(&elem_type)
                        }
                        IterableKindCached::List => {
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_LIST_GET)
                        }
                    };
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: value_local,
                        func: get_func,
                        args: vec![
                            mir::Operand::Local(container_local),
                            mir::Operand::Local(idx_local),
                        ],
                    });
                }

                // Bind the value to the target BEFORE incrementing idx,
                // so the bound value corresponds to the pre-increment
                // index.
                self.lower_binding_target(
                    target,
                    mir::Operand::Local(value_local),
                    &target_type,
                    hir_module,
                    mir_func,
                )?;

                // Increment idx: idx = idx + 1.
                let next_idx = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: next_idx,
                    op: mir::BinOp::Add,
                    left: mir::Operand::Local(idx_local),
                    right: mir::Operand::Constant(mir::Constant::Int(1)),
                });
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: idx_local,
                    src: mir::Operand::Local(next_idx),
                });
                Ok(())
            }
            IterState::Protocol { iter_local } => {
                // rt_iter_next_no_exc(iter_local) — returns *mut Obj
                // (raw primitives like int/bool are boxed by the runtime's
                // `box_if_raw_int_iterator`).
                let boxed_value_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_NEXT_NO_EXC),
                    vec![mir::Operand::Local(iter_local)],
                    Type::HeapAny,
                    mir_func,
                );

                // Determine the element type. Special case for range():
                // its iter_expr is a BuiltinCall that get_iterable_info
                // can't classify — elem is Int.
                let iter_expr = &hir_module.exprs[iter_id];
                let elem_type = if matches!(
                    &iter_expr.kind,
                    hir::ExprKind::BuiltinCall {
                        builtin: hir::Builtin::Range,
                        ..
                    }
                ) {
                    Type::Int
                } else {
                    let iter_type = self.get_type_of_expr_id(iter_id, hir_module);
                    get_iterable_info(&iter_type)
                        .map(|(_, t)| t)
                        .unwrap_or(Type::Any)
                };

                // Unbox the iter-next result for primitive element types.
                let value_operand = match &elem_type {
                    Type::Int => {
                        let unboxed = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_UNBOX_INT),
                            vec![mir::Operand::Local(boxed_value_local)],
                            Type::Int,
                            mir_func,
                        );
                        mir::Operand::Local(unboxed)
                    }
                    Type::Float => {
                        let unboxed = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_UNBOX_FLOAT),
                            vec![mir::Operand::Local(boxed_value_local)],
                            Type::Float,
                            mir_func,
                        );
                        mir::Operand::Local(unboxed)
                    }
                    Type::Bool => {
                        let unboxed = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_UNBOX_BOOL),
                            vec![mir::Operand::Local(boxed_value_local)],
                            Type::Bool,
                            mir_func,
                        );
                        mir::Operand::Local(unboxed)
                    }
                    // Heap types (Str, List, Dict, Class, …) are already
                    // pointers.
                    _ => mir::Operand::Local(boxed_value_local),
                };

                self.lower_binding_target(target, value_operand, &elem_type, hir_module, mir_func)?;
                Ok(())
            }
        }
    }

    /// Lower `ExprKind::IterHasNext(iter)` — read the cached `IterState`
    /// and emit the dispatch-appropriate predicate. Returns a bool operand.
    pub(crate) fn lower_iter_has_next(
        &mut self,
        iter_id: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let state = self
            .codegen
            .iter_cache
            .get(&iter_id)
            .cloned()
            .ok_or_else(|| {
                CompilerError::type_error(
                    format!(
                        "IterHasNext for expr {:?} without preceding IterSetup — \
                         CFG invariant violation",
                        iter_id
                    ),
                    hir_module.exprs[iter_id].span,
                )
            })?;

        match state {
            IterState::Indexed {
                idx_local,
                len_local,
                ..
            } => {
                // has_next = idx < len
                let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: cmp_local,
                    op: mir::BinOp::Lt,
                    left: mir::Operand::Local(idx_local),
                    right: mir::Operand::Local(len_local),
                });
                Ok(mir::Operand::Local(cmp_local))
            }
            IterState::Protocol { iter_local } => {
                // has_next = !rt_iter_is_exhausted(iter_local)
                let exhausted_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_IS_EXHAUSTED),
                    vec![mir::Operand::Local(iter_local)],
                    Type::Bool,
                    mir_func,
                );
                let has_next_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::UnOp {
                    dest: has_next_local,
                    op: mir::UnOp::Not,
                    operand: mir::Operand::Local(exhausted_local),
                });
                Ok(mir::Operand::Local(has_next_local))
            }
        }
    }

    /// Lower `range()` arguments into `(start, stop, step)` i64 operands
    /// matching Python's range semantics:
    /// - `range(stop)` → start=0, stop=stop, step=1
    /// - `range(start, stop)` → step=1
    /// - `range(start, stop, step)` — all explicit
    fn lower_range_args(
        &mut self,
        args: &[hir::ExprId],
        span: pyaot_utils::Span,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<(mir::Operand, mir::Operand, mir::Operand)> {
        let zero = mir::Operand::Constant(mir::Constant::Int(0));
        let one = mir::Operand::Constant(mir::Constant::Int(1));
        match args.len() {
            1 => {
                let stop = self.lower_expr(&hir_module.exprs[args[0]], hir_module, mir_func)?;
                Ok((zero, stop, one))
            }
            2 => {
                let start = self.lower_expr(&hir_module.exprs[args[0]], hir_module, mir_func)?;
                let stop = self.lower_expr(&hir_module.exprs[args[1]], hir_module, mir_func)?;
                Ok((start, stop, one))
            }
            3 => {
                let start = self.lower_expr(&hir_module.exprs[args[0]], hir_module, mir_func)?;
                let stop = self.lower_expr(&hir_module.exprs[args[1]], hir_module, mir_func)?;
                let step = self.lower_expr(&hir_module.exprs[args[2]], hir_module, mir_func)?;
                Ok((start, stop, step))
            }
            n => Err(CompilerError::type_error(
                format!("range() takes 1-3 arguments, got {}", n),
                span,
            )),
        }
    }

    /// Lower `ExprKind::MatchPattern { subject, pattern }` — emit the
    /// pattern predicate and return a bool operand. Delegates to the
    /// existing `generate_pattern_check` which is the authoritative
    /// pattern-predicate implementation (used by `lower_match_cases`).
    ///
    /// **Binding limitation (follow-up)**: the bindings produced by
    /// `generate_pattern_check` (captured variables like `x` in `case
    /// Point(x, y)`) are intentionally dropped here. Emitting them in the
    /// current block (the "test block") would run them before the
    /// pattern-match outcome is known, causing spurious attribute/index
    /// errors on non-matching subjects. The correct placement is inside
    /// the case-body block (entered only on match success).
    pub(crate) fn lower_match_pattern(
        &mut self,
        subject: hir::ExprId,
        pattern: &hir::Pattern,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let subject_expr = &hir_module.exprs[subject];
        let subject_operand = self.lower_expr(subject_expr, hir_module, mir_func)?;
        let subject_type = self.get_type_of_expr_id(subject, hir_module);

        let subject_local = self.alloc_and_add_local(subject_type.clone(), mir_func);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: subject_local,
            src: subject_operand,
        });

        let (cond, _bindings) = self.generate_pattern_check(
            pattern,
            mir::Operand::Local(subject_local),
            &subject_type,
            hir_module,
            mir_func,
        )?;

        Ok(cond)
    }
}
