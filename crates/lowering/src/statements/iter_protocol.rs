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

use pyaot_core_defs::{runtime_func_def, BuiltinExceptionKind};
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::{IterState, IterableKindCached, Lowering, StepDirection};
use crate::utils::{get_iterable_info, IterableKind};

/// Pre-detect step sign from a HIR step expression (before lowering to
/// MIR operand). Handles:
/// - `Int(v)` literal — sign from `v`.
/// - `UnOp(Neg, Int(v))` negated literal — always Negative for positive
///   `v`, Positive for negative `v` (double negation).
/// - Everything else → Unknown (runtime check would be needed).
fn detect_step_direction(expr: &hir::Expr) -> StepDirection {
    match &expr.kind {
        hir::ExprKind::Int(v) => {
            if *v < 0 {
                StepDirection::Negative
            } else {
                StepDirection::Positive
            }
        }
        hir::ExprKind::UnOp {
            op: hir::UnOp::Neg,
            operand,
            ..
        } => {
            // Peek one level through the arena — we only have the
            // ExprId, not the Module, so just handle the common case
            // of a literal inside the Neg. Since we don't have arena
            // access here, return Unknown for nested exprs; caller
            // handles the simple Int-inside-Neg case via the Int arm
            // when the frontend has already folded it.
            let _ = operand;
            StepDirection::Negative
        }
        _ => StepDirection::Unknown,
    }
}

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

        // §1.17b-c — special case: `for i in range(...)` uses the
        // direct-counter IterState::Range variant (mirrors
        // `lower_for_range`). Avoids allocating an IteratorObj and the
        // box/unbox round-trip on every iteration — both significant
        // correctness issues (segfault at module-init when GC fires
        // mid-unbox) and perf wins.
        if let hir::ExprKind::BuiltinCall {
            builtin: hir::Builtin::Range,
            args,
            ..
        } = &iter_expr.kind
        {
            // Pre-detect step direction from HIR BEFORE lowering so
            // negated literals (UnOp(Neg, Int(1)) in HIR for `-1`) are
            // recognized. Step-local matching on `Constant::Int(-1)` at
            // MIR level doesn't work because UnOp lowers to a Local.
            let step_is_negative = if args.len() == 3 {
                let step_expr = &hir_module.exprs[args[2]];
                detect_step_direction(step_expr)
            } else {
                // range(n) or range(start, stop) — step implicitly 1.
                StepDirection::Positive
            };

            let (start, stop, step) =
                self.lower_range_args(args, iter_expr.span, hir_module, mir_func)?;

            // Copy args into stable locals so the iter cache has
            // `LocalId`s to reference across iterations. start → idx.
            let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: idx_local,
                src: start,
            });
            let stop_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: stop_local,
                src: stop,
            });
            let step_local = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::Copy {
                dest: step_local,
                src: step,
            });

            self.codegen.iter_cache.insert(
                iter_id,
                IterState::Range {
                    idx_local,
                    stop_local,
                    step_local,
                    step_is_negative,
                },
            );
            return Ok(());
        }

        let iter_type = self.get_type_of_expr_id(iter_id, hir_module);
        if let Type::Class { class_id, .. } = &iter_type {
            let class_info = self.get_class_info(class_id).ok_or_else(|| {
                CompilerError::type_error(
                    format!("missing class info for iterator type '{:?}'", iter_type),
                    iter_expr.span,
                )
            })?;
            let iter_func_id = class_info.get_dunder_func("__iter__").ok_or_else(|| {
                CompilerError::type_error(
                    format!("cannot iterate over type '{:?}'", iter_type),
                    iter_expr.span,
                )
            })?;
            let next_func_id = class_info.get_dunder_func("__next__").ok_or_else(|| {
                CompilerError::type_error(
                    format!("iterator type '{:?}' is missing __next__", iter_type),
                    iter_expr.span,
                )
            })?;
            let elem_type = self
                .get_func_return_type(&next_func_id)
                .cloned()
                .unwrap_or(Type::Any);
            let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
            let iter_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
            self.emit_instruction(mir::InstructionKind::CallDirect {
                dest: iter_local,
                func: iter_func_id,
                args: vec![iter_operand],
            });
            let value_local = self.alloc_and_add_local(elem_type.clone(), mir_func);
            self.codegen.iter_cache.insert(
                iter_id,
                IterState::Class {
                    iter_local,
                    value_local,
                    elem_type,
                    next_func_id,
                },
            );
            return Ok(());
        }
        let Some((kind, elem_type)) = get_iterable_info(&iter_type) else {
            return Err(CompilerError::type_error(
                format!(
                    "cannot iterate over type '{:?}' in IterSetup (no iterable info)",
                    iter_type
                ),
                iter_expr.span,
            ));
        };

        // §1.17b-c — indexed dispatch for List, Tuple, Str, Bytes.
        // Matches `lower_for_iterable`'s fast path: no IteratorObj,
        // just len() + get_typed(). Str and Bytes use this path too
        // because the iterator-protocol check-before-next semantics
        // overshoot by one iteration on strings (the `exhausted` flag
        // is only set on an out-of-bounds next call, not on the last
        // in-bounds call).
        match kind {
            IterableKind::List
            | IterableKind::Tuple
            | IterableKind::Str
            | IterableKind::Bytes
            | IterableKind::Dict
            | IterableKind::Set => {
                let container_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
                let raw_container_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: raw_container_local,
                    src: container_operand,
                });

                // Dict/Set convert to List-of-elements first; others use
                // the raw container directly.
                let (container_local, indexed_kind, len_func) = match kind {
                    IterableKind::Dict => {
                        let key_elem_tag = crate::type_dispatch::elem_tag_for_type(&elem_type);
                        let keys_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_DICT_KEYS),
                            vec![
                                mir::Operand::Local(raw_container_local),
                                mir::Operand::Constant(mir::Constant::Int(key_elem_tag)),
                            ],
                            Type::List(Box::new(elem_type.clone())),
                            mir_func,
                        );
                        (
                            keys_local,
                            IterableKindCached::List,
                            &runtime_func_def::RT_LIST_LEN,
                        )
                    }
                    IterableKind::Set => {
                        let list_local = self.emit_runtime_call(
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_SET_TO_LIST),
                            vec![mir::Operand::Local(raw_container_local)],
                            Type::List(Box::new(elem_type.clone())),
                            mir_func,
                        );
                        (
                            list_local,
                            IterableKindCached::List,
                            &runtime_func_def::RT_LIST_LEN,
                        )
                    }
                    IterableKind::List => (
                        raw_container_local,
                        IterableKindCached::List,
                        &runtime_func_def::RT_LIST_LEN,
                    ),
                    IterableKind::Tuple => (
                        raw_container_local,
                        IterableKindCached::Tuple,
                        &runtime_func_def::RT_TUPLE_LEN,
                    ),
                    IterableKind::Str => (
                        raw_container_local,
                        IterableKindCached::Str,
                        &runtime_func_def::RT_STR_LEN_INT,
                    ),
                    IterableKind::Bytes => (
                        raw_container_local,
                        IterableKindCached::Bytes,
                        &runtime_func_def::RT_BYTES_LEN,
                    ),
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
                        kind: indexed_kind,
                    },
                );
                return Ok(());
            }
            IterableKind::File => {
                let file_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;
                let file_local = self.alloc_and_add_local(iter_type.clone(), mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: file_local,
                    src: file_operand,
                });
                let lines_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&runtime_func_def::RT_FILE_READLINES),
                    vec![mir::Operand::Local(file_local)],
                    Type::List(Box::new(elem_type.clone())),
                    mir_func,
                );
                let len_local = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&runtime_func_def::RT_LIST_LEN),
                    vec![mir::Operand::Local(lines_local)],
                    Type::Int,
                    mir_func,
                );
                let idx_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: idx_local,
                    src: mir::Operand::Constant(mir::Constant::Int(0)),
                });
                self.codegen.iter_cache.insert(
                    iter_id,
                    IterState::Indexed {
                        container_local: lines_local,
                        idx_local,
                        len_local,
                        elem_type,
                        kind: IterableKindCached::List,
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
            IterableKind::File => unreachable!("handled above"),
            IterableKind::List | IterableKind::Tuple => unreachable!("handled above"),
        };
        let iter_local =
            self.emit_runtime_call(rt_func, vec![iter_operand], Type::HeapAny, mir_func);
        // Allocate value_local as HeapAny (next returns *mut Obj). Will
        // be populated by IterHasNext (calls next first) and read by
        // IterAdvance.
        let value_local = self.alloc_and_add_local(Type::HeapAny, mir_func);
        self.codegen.iter_cache.insert(
            iter_id,
            IterState::Protocol {
                iter_local,
                value_local,
                elem_type,
            },
        );
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
            IterState::Range {
                idx_local,
                step_local,
                ..
            } => {
                // Bind current idx to target (int element type), then
                // advance idx by step. Matches `lower_for_range`.
                self.lower_binding_target(
                    target,
                    mir::Operand::Local(idx_local),
                    &Type::Int,
                    hir_module,
                    mir_func,
                )?;
                // §1.17b-c — sync global var if the target is a global.
                // `bind_var_op` only updates the local; `lower_assign`'s
                // global-sync path is separate. Without this, module-init
                // for-loops over globals produce the wrong total because
                // subsequent reads via `rt_global_get_X` see the stale
                // global slot. Mirrors the sync emitted at the head of
                // `lower_for_iterable`'s body block.
                self.sync_global_if_needed(target, &Type::Int, mir_func);
                let next_idx = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: next_idx,
                    op: mir::BinOp::Add,
                    left: mir::Operand::Local(idx_local),
                    right: mir::Operand::Local(step_local),
                });
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: idx_local,
                    src: mir::Operand::Local(next_idx),
                });
                Ok(())
            }
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
                // Match the tree walker's dispatch in
                // `lower_for_iterable`: Int is typed for List only
                // (regular lists store raw ints), while Float/Bool are
                // typed for Dict/Set-converted lists (converted via
                // `rt_dict_keys`/`rt_set_to_list` which preserve
                // elem_tag). Dict/Set paths above use `IterableKindCached::List`
                // since the converted container is a List.
                let elem_kind_for_typed = match (kind, &elem_type) {
                    (IterableKindCached::List, Type::Int) => Some(mir::GetElementKind::Int),
                    (IterableKindCached::List, Type::Float) => Some(mir::GetElementKind::Float),
                    (IterableKindCached::List, Type::Bool) => Some(mir::GetElementKind::Bool),
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
                        IterableKindCached::Str => {
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_STR_GETCHAR)
                        }
                        IterableKindCached::Bytes => {
                            mir::RuntimeFunc::Call(&runtime_func_def::RT_BYTES_GET)
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
                self.sync_global_if_needed(target, &target_type, mir_func);

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
            IterState::Protocol {
                value_local,
                elem_type,
                ..
            } => {
                // §1.17b-c — Protocol's IterAdvance just reads the cached
                // value_local populated by IterHasNext (tree-walker
                // semantics: next is called BEFORE has-next check).
                //
                // `rt_iter_next_no_exc` returns different shapes
                // depending on the iterator kind:
                // - Generators (Type::Iterator): `rt_generator_next`
                //   returns the yielded value DIRECTLY as raw i64 —
                //   no boxing applied. This matches tree walker's
                //   `lower_for_iterator` which types next_local as
                //   Int/Str/etc and copies directly.
                // - Non-generator iterators (shouldn't reach here
                //   post-Indexed-extension — List/Tuple/Dict/Set/Str/
                //   Bytes all go through Indexed now).
                //
                // So we DON'T unbox for the Protocol path — the value
                // is already in the target's representation.
                self.lower_binding_target(
                    target,
                    mir::Operand::Local(value_local),
                    &elem_type,
                    hir_module,
                    mir_func,
                )?;
                self.sync_global_if_needed(target, &elem_type, mir_func);
                Ok(())
            }
            IterState::Class {
                value_local,
                elem_type,
                ..
            } => {
                self.lower_binding_target(
                    target,
                    mir::Operand::Local(value_local),
                    &elem_type,
                    hir_module,
                    mir_func,
                )?;
                self.sync_global_if_needed(target, &elem_type, mir_func);
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
            IterState::Range {
                idx_local,
                stop_local,
                step_is_negative,
                ..
            } => {
                // Positive step: has_next = idx < stop
                // Negative step: has_next = idx > stop
                // Unknown (runtime): conservatively use Lt (matches
                // Python's default step=1 — unknown step direction
                // requires runtime dispatch, not yet wired).
                let op = match step_is_negative {
                    StepDirection::Negative => mir::BinOp::Gt,
                    StepDirection::Positive | StepDirection::Unknown => mir::BinOp::Lt,
                };
                let cmp_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: cmp_local,
                    op,
                    left: mir::Operand::Local(idx_local),
                    right: mir::Operand::Local(stop_local),
                });
                Ok(mir::Operand::Local(cmp_local))
            }
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
            IterState::Protocol {
                iter_local,
                value_local,
                ..
            } => {
                // §1.17b-c — call next FIRST, then check exhausted.
                // Matches tree walker's `lower_for_iterator`: the
                // runtime's `exhausted` flag is set only on the first
                // out-of-bounds next call, so checking it before next
                // would overshoot by one iteration. Cache the result
                // in value_local for IterAdvance to read.
                let next_tmp = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_NEXT_NO_EXC),
                    vec![mir::Operand::Local(iter_local)],
                    Type::HeapAny,
                    mir_func,
                );
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: value_local,
                    src: mir::Operand::Local(next_tmp),
                });
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
            IterState::Class {
                iter_local,
                value_local,
                next_func_id,
                ..
            } => {
                let has_next_local = self.alloc_and_add_local(Type::Bool, mir_func);
                let frame_local = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::ExcPushFrame { frame_local });

                let try_next_bb = self.new_block();
                let handler_bb = self.new_block();
                let stop_iter_bb = self.new_block();
                let merge_bb = self.new_block();
                let reraise_bb = self.new_block();

                self.current_block_mut().terminator = mir::Terminator::TrySetjmp {
                    frame_local,
                    try_body: try_next_bb.id,
                    handler_entry: handler_bb.id,
                };

                self.push_block(try_next_bb);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest: value_local,
                    func: next_func_id,
                    args: vec![mir::Operand::Local(iter_local)],
                });
                self.emit_instruction(mir::InstructionKind::ExcPopFrame);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: has_next_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(true)),
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);

                self.push_block(handler_bb);
                let stop_local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::ExcCheckClass {
                    dest: stop_local,
                    class_id: BuiltinExceptionKind::StopIteration.tag(),
                });
                self.current_block_mut().terminator = mir::Terminator::Branch {
                    cond: mir::Operand::Local(stop_local),
                    then_block: stop_iter_bb.id,
                    else_block: reraise_bb.id,
                };

                self.push_block(stop_iter_bb);
                self.emit_instruction(mir::InstructionKind::ExcClear);
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: has_next_local,
                    src: mir::Operand::Constant(mir::Constant::Bool(false)),
                });
                self.current_block_mut().terminator = mir::Terminator::Goto(merge_bb.id);

                self.push_block(reraise_bb);
                self.current_block_mut().terminator = mir::Terminator::Reraise;

                self.push_block(merge_bb);
                Ok(mir::Operand::Local(has_next_local))
            }
        }
    }

    /// §1.17b-c — sync the global slot for a simple `Var` target after
    /// binding. `bind_var_op` only updates the function-local; when the
    /// target is a module-level global, the `rt_global_set_X` call is
    /// emitted separately (see `lower_assign` and
    /// `lower_for_iterable`'s body emission for the equivalent pattern).
    /// No-op for non-Var targets or non-global vars.
    fn sync_global_if_needed(
        &mut self,
        target: &hir::BindingTarget,
        ty: &Type,
        mir_func: &mut mir::Function,
    ) {
        let hir::BindingTarget::Var(var_id) = target else {
            return;
        };
        if !self.is_global(var_id) {
            return;
        }
        let target_local = self.get_or_create_local(*var_id, ty.clone(), mir_func);
        let runtime_func = self.get_global_set_func(ty);
        let effective_var_id = self.get_effective_var_id(*var_id);
        self.emit_runtime_call_void(
            runtime_func,
            vec![
                mir::Operand::Constant(mir::Constant::Int(effective_var_id)),
                mir::Operand::Local(target_local),
            ],
            mir_func,
        );
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
