//! §1.17b-c — Generic iterator protocol lowering for `StmtKind::IterSetup`,
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
//! enters the header). It calls the appropriate `rt_iter_X` runtime
//! function based on the iterable type and caches the resulting iterator
//! local in `CodeGenState::iter_cache` keyed by the `iter: ExprId`.
//!
//! `IterHasNext(iter)` and `IterAdvance{iter, target}` both read the
//! cached iterator local. They NEVER call `rt_iter_X` themselves — that
//! would reset the iterator each iteration.
//!
//! The runtime iterator protocol uses:
//! - `rt_iter_list` / `rt_iter_tuple` / `rt_iter_dict` / `rt_iter_set` /
//!   `rt_iter_str` / `rt_iter_bytes` / `rt_iter_generator` — setup (unary)
//! - `rt_iter_is_exhausted(iter) -> i8` — has-next predicate (boolean)
//! - `rt_iter_next_no_exc(iter) -> *mut Obj` — advance, returns the next
//!   element boxed as a heap pointer (raw primitives get boxed by the
//!   runtime via `box_if_raw_int_iterator`).

use pyaot_core_defs::runtime_func_def;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;
use crate::utils::{get_iterable_info, IterableKind};

impl<'a> Lowering<'a> {
    /// Lower `StmtKind::IterSetup { iter }` — call `rt_iter_X` on the
    /// iterable expression and cache the iterator local. Must run once
    /// in the pre-block before the for-loop header.
    pub(crate) fn lower_iter_setup(
        &mut self,
        iter_id: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        // If the cache already has this iter_expr (shouldn't happen in
        // well-formed CFGs — bridge emits exactly one IterSetup per
        // for-loop), skip. Defensive: treat as no-op.
        if self.codegen.iter_cache.contains_key(&iter_id) {
            return Ok(());
        }

        let iter_expr = &hir_module.exprs[iter_id];

        // §1.17b-c — special case: `for i in range(...)` uses
        // `rt_iter_range(start, stop, step)` which takes 3 i64 args,
        // NOT the generic (iterable → iterator) pattern. Detect and
        // handle before the generic dispatch below.
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
            self.codegen.iter_cache.insert(iter_id, iter_local);
            return Ok(());
        }

        let iter_type = self.get_type_of_expr_id(iter_id, hir_module);

        // Lower the iterable expression to an operand.
        let iter_operand = self.lower_expr(iter_expr, hir_module, mir_func)?;

        // Pick the appropriate rt_iter_X runtime function.
        let Some((kind, _elem_type)) = get_iterable_info(&iter_type) else {
            return Err(CompilerError::type_error(
                format!(
                    "cannot iterate over type '{:?}' in IterSetup (no iterable info)",
                    iter_type
                ),
                iter_expr.span,
            ));
        };

        let rt_func = match kind {
            IterableKind::List => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_LIST),
            IterableKind::Tuple => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_TUPLE),
            IterableKind::Dict => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_DICT),
            IterableKind::Set => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_SET),
            IterableKind::Str => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_STR),
            IterableKind::Bytes => mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_BYTES),
            IterableKind::Iterator => {
                // Generators / existing iterators — return as-is.
                mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_GENERATOR)
            }
            IterableKind::File => {
                return Err(CompilerError::type_error(
                    "IterSetup does not yet support file iteration — \
                     use `for line in f.readlines():` or fall back to tree \
                     lowering for now"
                        .to_string(),
                    iter_expr.span,
                ));
            }
        };

        // Emit iter setup call; the result is a heap iterator pointer.
        let iter_local =
            self.emit_runtime_call(rt_func, vec![iter_operand], Type::HeapAny, mir_func);

        // Cache the iterator local for subsequent IterHasNext / IterAdvance
        // with the same iter ExprId.
        self.codegen.iter_cache.insert(iter_id, iter_local);

        Ok(())
    }

    /// Lower `StmtKind::IterAdvance { iter, target }` — read the cached
    /// iterator local, call `rt_iter_next_no_exc` to advance, bind the
    /// result to `target`.
    pub(crate) fn lower_iter_advance(
        &mut self,
        iter_id: hir::ExprId,
        target: &hir::BindingTarget,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let iter_local = self
            .codegen
            .iter_cache
            .get(&iter_id)
            .copied()
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

        // Emit rt_iter_next_no_exc(iter_local) — returns *mut Obj (raw
        // primitives like int/bool are boxed by the runtime's
        // `box_if_raw_int_iterator`).
        let boxed_value_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_NEXT_NO_EXC),
            vec![mir::Operand::Local(iter_local)],
            Type::HeapAny,
            mir_func,
        );

        // Determine the element type from the iterable.
        // Special case for range(): its iter_expr is a BuiltinCall that
        // get_iterable_info can't classify — the element type is Int.
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

        // §1.17b-c — unbox the iter-next result for primitive types.
        // Runtime returns boxed Obj pointers for raw-int/bool iterators
        // (`box_if_raw_int_iterator`); unbox back to raw i64/f64/i8 so
        // the bind target (which is declared with the primitive type)
        // gets the correct representation.
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
            // Heap types (Str, List, Dict, Class, …) are pointers already.
            _ => mir::Operand::Local(boxed_value_local),
        };

        // Bind the value to the target via the unified binding-target path.
        self.lower_binding_target(target, value_operand, &elem_type, hir_module, mir_func)?;

        Ok(())
    }

    /// Lower `ExprKind::IterHasNext(iter)` — read the cached iterator local,
    /// call `rt_iter_is_exhausted`, NOT the result. Returns a bool operand.
    pub(crate) fn lower_iter_has_next(
        &mut self,
        iter_id: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let iter_local = self
            .codegen
            .iter_cache
            .get(&iter_id)
            .copied()
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

        // Emit rt_iter_is_exhausted(iter_local) → bool (i8).
        let exhausted_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&runtime_func_def::RT_ITER_IS_EXHAUSTED),
            vec![mir::Operand::Local(iter_local)],
            Type::Bool,
            mir_func,
        );

        // NOT it: has_next = !exhausted.
        let has_next_local = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::UnOp {
            dest: has_next_local,
            op: mir::UnOp::Not,
            operand: mir::Operand::Local(exhausted_local),
        });

        Ok(mir::Operand::Local(has_next_local))
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
    ///
    /// The CFG walker (S1.17b-c main loop) must ensure bindings are
    /// emitted in the case-body block head via one of:
    /// - Bridge-side: augment `cfg_build` to emit binding extraction as
    ///   HIR `Bind` statements at the head of each case body block.
    /// - Walker-side: post-process the Branch emission by calling
    ///   `generate_pattern_check` again in the success path to emit
    ///   bindings.
    ///
    /// Until one of those lands, `MatchPattern` lowering is correct for
    /// patterns that don't introduce new captures: `MatchValue`,
    /// `MatchSingleton`, and `MatchAs { pattern, name: None }` (wildcard).
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

        // Cache the subject in a local to avoid re-evaluation (matches the
        // semantics of `lower_match` which stores the subject once up-front).
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

        // Bindings are dropped — see doc comment. The bridge must emit
        // binding-extraction HIR stmts in the case-body block head for
        // full correctness.
        Ok(cond)
    }
}
