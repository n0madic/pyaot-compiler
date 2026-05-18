//! Binary operation lowering: arithmetic, bitwise, string concat, collection ops

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

/// Minimum number of string operands to use StringBuilder pattern
/// Below this threshold, regular StrConcat is used (simpler and still efficient for 2 strings)
const STRING_BUILDER_THRESHOLD: usize = 3;

/// True iff `ty` is a `Union` whose variants include at least one `Class`.
fn union_contains_class(ty: &Type) -> bool {
    if let Type::Union(variants) = ty {
        variants.iter().any(|v| matches!(v, Type::Class { .. }))
    } else {
        false
    }
}

// Op → dunder-name mappings live on `hir::BinOp` itself
// (`forward_dunder` / `reflected_dunder`) so every consumer — binary-op
// dispatch, type planning, reductions — shares one source of truth.

impl<'a> Lowering<'a> {
    /// Collect all string operands from a left-associative concatenation chain.
    /// Returns true if the expression is a string add operation, false otherwise.
    /// Operands are collected left-to-right (evaluation order).
    fn collect_str_concat_chain(
        &mut self,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
        chain: &mut Vec<hir::ExprId>,
    ) -> bool {
        let expr = &hir_module.exprs[expr_id];

        // Check if this is a string add operation
        if let hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left,
            right,
        } = &expr.kind
        {
            let left_type = self.seed_expr_type(*left, hir_module);
            if matches!(left_type, Type::Str) {
                // Recursively collect from left side first (left-to-right evaluation)
                self.collect_str_concat_chain(*left, hir_module, chain);
                // Then add the right operand
                chain.push(*right);
                return true;
            }
        }

        // Not a string add - this is a leaf node, add it to the chain
        chain.push(expr_id);
        false
    }

    /// Lower a binary operation expression.
    pub(in crate::expressions) fn lower_binop(
        &mut self,
        op: hir::BinOp,
        left: hir::ExprId,
        right: hir::ExprId,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Get operand types using get_expr_type for better inference
        let left_expr = &hir_module.exprs[left];
        let left_hir_ty = self.seed_expr_type(left, hir_module);

        // Check for string concatenation chain optimization
        if matches!(op, hir::BinOp::Add) && matches!(left_hir_ty, Type::Str) {
            // Collect the full chain starting from the current expression
            let mut chain = Vec::new();

            // Get the current expression's ID from the HIR module
            // We need to reconstruct the chain from the current BinOp
            self.collect_str_concat_chain(left, hir_module, &mut chain);
            chain.push(right);

            // If we have 3+ operands, use StringBuilder
            if chain.len() >= STRING_BUILDER_THRESHOLD {
                return self.lower_str_concat_with_builder(&chain, hir_module, mir_func);
            }
        }

        // Standard lowering path
        let left_op = self.lower_expr(left_expr, hir_module, mir_func)?;
        let right_expr = &hir_module.exprs[right];
        let right_op = self.lower_expr(right_expr, hir_module, mir_func)?;

        let left_ty = self.operand_type(&left_op, mir_func);
        let right_ty = self.operand_type(&right_op, mir_func);

        // Infer result type based on operand types
        let result_ty = if matches!(left_ty, Type::Class { .. }) {
            // Class with arithmetic dunders returns the class type
            left_ty.clone()
        } else if matches!(right_ty, Type::Class { .. }) {
            // Reverse dunder case: right operand is a class, result is that class type
            right_ty.clone()
        } else if matches!(left_ty, Type::Str) {
            Type::Str // String operations return strings
        } else if matches!(left_ty, Type::Bytes) && matches!(op, hir::BinOp::Add | hir::BinOp::Mul)
        {
            Type::Bytes // Bytes operations return bytes
        } else if matches!(op, hir::BinOp::Div) {
            // Python 3: true division always returns float
            Type::Float
        } else if matches!(left_ty, Type::Float) || matches!(right_ty, Type::Float) {
            // Float + anything numeric = Float
            Type::Float
        } else if matches!(left_ty, Type::Int) && matches!(right_ty, Type::Int) {
            Type::Int
        } else {
            expr.ty.clone().unwrap_or(Type::Any)
        };
        let result_local = self.alloc_and_add_local(result_ty.clone(), mir_func);

        // Check for string operations
        if matches!(left_ty, Type::Str) {
            match op {
                hir::BinOp::Add => {
                    // String concatenation (2 operands - use simple concat)
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_STR_CONCAT,
                        ),
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                hir::BinOp::Mul => {
                    // String multiplication: "abc" * 3
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_STR_MUL,
                        ),
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                _ => {}
            }
        }

        // Check for bytes operations (concatenation, repetition)
        if matches!(left_ty, Type::Bytes) {
            match op {
                hir::BinOp::Add => {
                    // Bytes concatenation
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BYTES_CONCAT,
                        ),
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                hir::BinOp::Mul => {
                    // Bytes repetition: b"abc" * 3
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BYTES_REPEAT,
                        ),
                        args: vec![left_op, right_op],
                    });
                    return Ok(mir::Operand::Local(result_local));
                }
                _ => {}
            }
        }

        // Check for list concatenation (+) and repetition (*)
        if let Some(elem_ty) = left_ty.list_elem() {
            if matches!(op, hir::BinOp::Add) {
                let list_result = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_CONCAT),
                    vec![left_op, right_op],
                    Type::list_of(elem_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(list_result));
            }
            if matches!(op, hir::BinOp::Mul) && matches!(right_ty, Type::Int | Type::Bool) {
                let list_result = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_REPEAT),
                    vec![left_op, right_op],
                    Type::list_of(elem_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(list_result));
            }
        }
        // Reflected: int * list — same runtime, swap operands.
        if let Some(elem_ty) = right_ty.list_elem() {
            if matches!(op, hir::BinOp::Mul) && matches!(left_ty, Type::Int | Type::Bool) {
                let list_result = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_LIST_REPEAT),
                    vec![right_op, left_op],
                    Type::list_of(elem_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(list_result));
            }
        }

        // Check for dict merge operation (|)
        if let Some((key_ty, value_ty)) = left_ty.dict_kv() {
            if matches!(op, hir::BinOp::BitOr) {
                let dict_result = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_DICT_MERGE),
                    vec![left_op, right_op],
                    Type::dict_of(key_ty.clone(), value_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(dict_result));
            }
        }

        // Check for set operations (|, &, -, ^)
        if let Some(elem_ty) = left_ty.set_elem() {
            let set_func = match op {
                hir::BinOp::BitOr => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_UNION,
                )),
                hir::BinOp::BitAnd => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_INTERSECTION,
                )),
                hir::BinOp::Sub => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_DIFFERENCE,
                )),
                hir::BinOp::BitXor => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_SET_SYMMETRIC_DIFFERENCE,
                )),
                _ => None,
            };
            if let Some(runtime_func) = set_func {
                let set_result = self.emit_runtime_call(
                    runtime_func,
                    vec![left_op, right_op],
                    Type::set_of(elem_ty.clone()),
                    mir_func,
                );
                return Ok(mir::Operand::Local(set_result));
            }
        }

        // Class dispatch: subclass-first (§3.3.8), forward dunder with NI
        // fallback, reflected dunder. Extracted so Area C reductions can
        // reuse the exact same state machine for `sum`/`min`/`max` on
        // user classes without duplicating boxing / NI fallback logic.
        if let Some(op_) = self.dispatch_class_binop(
            op,
            left_op.clone(),
            &left_ty,
            right_op.clone(),
            &right_ty,
            &result_ty,
            hir_module,
            mir_func,
        ) {
            return Ok(op_);
        }

        // Check if operands are stored as Union (boxed pointers), even if inference
        // narrowed the type. The storage type determines the runtime representation.
        let left_is_union = left_ty.is_union()
            || matches!(&left_op, mir::Operand::Local(id) if mir_func.locals.get(id).is_some_and(|l| l.ty.is_union()));
        let right_is_union = right_ty.is_union()
            || matches!(&right_op, mir::Operand::Local(id) if mir_func.locals.get(id).is_some_and(|l| l.ty.is_union()));
        // Class operand without a matching dunder reached this point because
        // `dispatch_class_binop` returned `None` (the class lacks the
        // forward dunder and the other operand is not a reflective Class).
        // Falling through to a raw `mir::BinOp` would panic in codegen
        // (`Class` lowers to an i64 pointer ABI; primitive arith expects
        // f64/i64 numeric values). Route through the runtime helper
        // instead — it now dispatches to user-defined dunders via
        // `DUNDER_FUNC_REGISTRY` (see `runtime/src/ops/dunder_dispatch.rs`)
        // and raises a precise TypeError if no dunder is defined, matching
        // CPython's §3.3.8 behaviour. Safe because the runtime helper
        // boxes any primitive operand via `emit_value_slot` first.
        let left_is_class = matches!(left_ty, Type::Class { .. });
        let right_is_class = matches!(right_ty, Type::Class { .. });
        // Load-bearing `HeapAny` vs `Any` distinction. `HeapAny`
        // operands carry a tagged `Value` at runtime (INT/BOOL tag,
        // NONE, or heap pointer to FloatObj/Class/etc.) — route them
        // through `rt_obj_*` which dispatches on `Value::tag()`.
        // `Any` is NOT yet guaranteed tagged: the lambda return-flip
        // closes the closure-trampoline source for non-address-taken
        // lambdas, but two sources still leak raw bits into
        // `Type::Any` slots:
        //
        //   * Address-taken closure dispatch via `emit_closure_call`
        //     (decorator-factory / curried-chain). Inner funcs are
        //     marked `phase4_unsafe` by `mark_address_taken_funcrefs`,
        //     so they are NOT return-flipped — their bodies still
        //     return raw primitive bits.
        //   * Devirtualization / inlining narrowing of wrapper-call
        //     dest locals that were typed `Type::Any` at the call
        //     boundary but never went through a `BoxValue` insertion.
        //
        // Verified by attempting collapse: SEGVs in `runtime_match`,
        // `runtime_decorator_factory`, `runtime_decorator_factory_optimized`.
        // After F.1 HeapAny deletion, "guaranteed-tagged Any" is detected via
        // mir_ty: only locals allocated via alloc_and_add_local get an explicit
        // mir_ty = Some(Tagged). Variable locals (ty: Any, mir_ty: None) may
        // carry raw primitive bits from legacy trampolines and must NOT be
        // routed through rt_obj_* — they use raw BinOp instead.
        let left_is_any = self.operand_is_guaranteed_tagged(&left_op, &left_ty, mir_func);
        let right_is_any = self.operand_is_guaranteed_tagged(&right_op, &right_ty, mir_func);

        // Phase 4 ext: HOF lambda user-params arrive tagged from the HOF
        // runtime (sorted_tagged / map_tagged / reduce_tagged). When such a
        // param is used directly in a BinOp without a prologue UnboxValue
        // (because lambda_user_param_flip didn't fire on unannotated params),
        // the operand carries tagged Value bits but the BinOp expects Raw.
        // Route through `rt_obj_*` (which dispatches on Value::tag at
        // runtime) by treating the param-operand as if it were HeapAny.
        // Restricted to lambda-typed callees so non-HOF Tagged operands
        // (closure trampoline raw-bit leaks documented in feedback notes)
        // still hit the raw-BinOp path.
        let is_lambda_callee = mir_func.is_lambda_like();
        let operand_is_lambda_tagged_param = |op: &mir::Operand| -> bool {
            if !is_lambda_callee {
                return false;
            }
            let mir::Operand::Local(id) = op else {
                return false;
            };
            // Param locals are recorded as the first N locals in mir_func.params.
            let is_param = mir_func.params.iter().any(|p| p.id == *id);
            if !is_param {
                return false;
            }
            mir_func
                .locals
                .get(id)
                .is_some_and(|l| matches!(l.resolved_mir_type(), pyaot_mir::MirType::Tagged))
        };
        let left_is_lambda_param_tagged = operand_is_lambda_tagged_param(&left_op);
        let right_is_lambda_param_tagged = operand_is_lambda_tagged_param(&right_op);

        // Union / Any arithmetic: operands are already boxed pointers — use runtime dispatch
        if left_is_union
            || right_is_union
            || left_is_class
            || right_is_class
            || left_is_any
            || right_is_any
            || left_is_lambda_param_tagged
            || right_is_lambda_param_tagged
        {
            let obj_func = match op {
                hir::BinOp::Add => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_ADD,
                )),
                hir::BinOp::Sub => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_SUB,
                )),
                hir::BinOp::Mul => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_MUL,
                )),
                hir::BinOp::Div => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_DIV,
                )),
                hir::BinOp::FloorDiv => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_FLOORDIV,
                )),
                hir::BinOp::Mod => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_MOD,
                )),
                hir::BinOp::Pow => Some(mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_OBJ_POW,
                )),
                _ => None, // Bitwise ops not supported on Union (yet)
            };

            if let Some(rt_func) = obj_func {
                // Union, Any and HeapAny operands are already in tagged-Value
                // representation at runtime; only primitive (Int / Bool /
                // Float / Str / etc.) operands need to be boxed via
                // `emit_value_slot` before passing to `rt_obj_*`.
                //
                // Storage-Uniform invariant guard: when the operand is a
                // Local declared `Type::Any` / `Type::Any`, downstream
                // WPA passes might otherwise narrow it to a primitive
                // (e.g. monomorphization specialising a wrapper template
                // where the result of `result = func(*args)` resolves to
                // Int via a flipped callee). Such narrowing would leave the
                // `rt_obj_*` call expecting tagged Value but receiving raw
                // primitive bits, causing a SEGV. Mark these operand Locals
                // as `abi_immutable = true` so WPA preserves their Any /
                // HeapAny type — they are ABI-bound at this dispatch site.
                if left_is_any {
                    if let mir::Operand::Local(id) = &left_op {
                        if let Some(l) = mir_func.locals.get_mut(id) {
                            l.abi_immutable = true;
                        }
                    }
                }
                if right_is_any {
                    if let mir::Operand::Local(id) = &right_op {
                        if let Some(l) = mir_func.locals.get_mut(id) {
                            l.abi_immutable = true;
                        }
                    }
                }
                let boxed_left = if left_is_union || left_is_any {
                    left_op
                } else {
                    self.emit_value_slot(left_op, &left_ty, mir_func)
                };
                let boxed_right = if right_is_union || right_is_any {
                    right_op
                } else {
                    self.emit_value_slot(right_op, &right_ty, mir_func)
                };
                // Default result type for primitive Union arithmetic is the
                // numeric tower as a boxed value. When either operand Union
                // includes a `Class` variant, runtime dispatch may now route
                // through user-defined dunders (see
                // `runtime/src/ops/dunder_dispatch.rs`) and return a class
                // instance — the local must be typed widely enough to keep
                // downstream attribute access correct, so propagate the
                // input Union into the result type in that case. When an
                // operand is `Any`/`HeapAny`, the runtime dispatch may
                // return any boxed Value (including a class instance), so
                // we widen to `HeapAny` to keep downstream lowering correct.
                let union_result_ty = if left_is_any || right_is_any {
                    Type::Any
                } else if union_contains_class(&left_ty) || union_contains_class(&right_ty) {
                    let mut variants: Vec<Type> = vec![Type::Int, Type::Float];
                    if let Type::Union(left_variants) = &left_ty {
                        for v in left_variants {
                            if matches!(v, Type::Class { .. }) && !variants.contains(v) {
                                variants.push(v.clone());
                            }
                        }
                    }
                    if let Type::Union(right_variants) = &right_ty {
                        for v in right_variants {
                            if matches!(v, Type::Class { .. }) && !variants.contains(v) {
                                variants.push(v.clone());
                            }
                        }
                    }
                    Type::Union(variants)
                } else {
                    Type::Union(vec![Type::Int, Type::Float])
                };
                let union_result = self.emit_tagged_runtime_call(
                    rt_func,
                    vec![boxed_left, boxed_right],
                    union_result_ty,
                    mir_func,
                );
                // Reconcile static and runtime types: when the
                // class-aware `binop_result_type` (the type-planning
                // `expr.ty` for this BinOp) narrowed the result to a
                // primitive `Float`/`Int`, but the runtime dispatch
                // returns a tagged `Value` (i64), insert an unbox so
                // the operand's cranelift type matches the narrowed
                // static type. Without this, downstream lowering that
                // reads `seed_expr_type` (e.g. `lower_abs` emitting
                // `FloatAbs` on a `Float`-typed BinOp) would feed
                // an i64 operand into an f64-expecting instruction.
                // Note on the static / runtime type mismatch: the
                // class-aware `binop_result_type` may narrow the
                // *static* result to `Float` (by dropping a
                // `Class[Self]` Union variant whose dispatched dunder
                // is missing), but the runtime dispatch above still
                // returns a tagged `Value` (i64). Coercion is NOT
                // inserted here — the caller's chosen consumer (e.g.
                // `lower_abs`'s `FloatAbs` arm) is responsible for
                // checking its operand's MIR type and unboxing via
                // `rt_unbox_float` when the seed-derived static type
                // narrows past it. Inserting a coercion here would
                // produce an f64 operand that downstream Union-typed
                // slots (`union_y: int | float = ...`) would then
                // need to re-box, defeating the purpose.
                return Ok(mir::Operand::Local(union_result));
            }
        }

        // MatMul (@) is only supported via class dunders — no primitive meaning
        if matches!(op, hir::BinOp::MatMul) {
            return Err(pyaot_diagnostics::CompilerError::type_error(
                "unsupported operand type(s) for @: only classes with __matmul__ support this operator".to_string(),
                expr.span,
            ));
        }

        let mir_op = match op {
            hir::BinOp::Add => mir::BinOp::Add,
            hir::BinOp::Sub => mir::BinOp::Sub,
            hir::BinOp::Mul => mir::BinOp::Mul,
            hir::BinOp::Div => mir::BinOp::Div,
            hir::BinOp::FloorDiv => mir::BinOp::FloorDiv,
            hir::BinOp::Mod => mir::BinOp::Mod,
            hir::BinOp::Pow => mir::BinOp::Pow,
            // Bitwise operators
            hir::BinOp::BitAnd => mir::BinOp::BitAnd,
            hir::BinOp::BitOr => mir::BinOp::BitOr,
            hir::BinOp::BitXor => mir::BinOp::BitXor,
            hir::BinOp::LShift => mir::BinOp::LShift,
            hir::BinOp::RShift => mir::BinOp::RShift,
            hir::BinOp::MatMul => unreachable!("MatMul handled above"),
        };

        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: result_local,
            op: mir_op,
            left: left_op,
            right: right_op,
        });
        Ok(mir::Operand::Local(result_local))
    }

    /// Lower a string concatenation chain using StringBuilder for O(n) complexity.
    ///
    /// For `a + b + c + d`:
    /// 1. Create StringBuilder with estimated capacity
    /// 2. Append each string operand in evaluation order
    /// 3. Finalize to produce the result string
    fn lower_str_concat_with_builder(
        &mut self,
        chain: &[hir::ExprId],
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        // Estimate capacity: sum of string literal lengths + heuristic for non-literals
        let mut estimated_capacity: i64 = 0;
        for &expr_id in chain {
            let expr = &hir_module.exprs[expr_id];
            if let hir::ExprKind::Str(s) = &expr.kind {
                let str_content = self.interner.resolve(*s);
                estimated_capacity += str_content.len() as i64;
            } else {
                // Heuristic: assume 20 bytes for non-literal strings
                estimated_capacity += 20;
            }
        }

        // Create StringBuilder with estimated capacity (Using Str type for GC root tracking)
        let builder_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_MAKE_STRING_BUILDER),
            vec![mir::Operand::Constant(mir::Constant::Int(
                estimated_capacity,
            ))],
            Type::Str,
            mir_func,
        );

        // Append each string to the builder
        let dummy_local = self.alloc_and_add_local(Type::Int, mir_func); // For void call result
        for &expr_id in chain {
            let expr = &hir_module.exprs[expr_id];
            let str_op = self.lower_expr(expr, hir_module, mir_func)?;

            self.emit_instruction(mir::InstructionKind::RuntimeCall {
                dest: dummy_local,
                func: mir::RuntimeFunc::Call(
                    &pyaot_core_defs::runtime_func_def::RT_STRING_BUILDER_APPEND,
                ),
                args: vec![mir::Operand::Local(builder_local), str_op],
            });
        }

        // Finalize and return the result string
        let result_local = self.emit_runtime_call(
            mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_STRING_BUILDER_TO_STR),
            vec![mir::Operand::Local(builder_local)],
            Type::Str,
            mir_func,
        );

        Ok(mir::Operand::Local(result_local))
    }

    /// Emit the §3.3.8 fallback control flow:
    /// CPython §3.3.8 operator dunder dispatch state machine for user
    /// class operands. Shared between [`Lowering::lower_binop`] and the
    /// Area C reduction helpers (`sum`/`min`/`max` over user classes).
    ///
    /// Order of precedence:
    /// 1. **Subclass-first** — if `right` is a strict subclass of `left`,
    ///    try its reflected dunder first.
    /// 2. **Forward dunder on `left`** — standard `left.__op__(right)`;
    ///    if the forward dunder may return `NotImplemented` and `right`
    ///    has a reflected dunder, emit the compare+branch fallback.
    /// 3. **Reflected dunder on `right`** — when `left` doesn't define
    ///    the forward dunder (e.g. primitive left operand).
    ///
    /// Returns `Some(result)` if any dunder path matched, `None` to let
    /// the caller fall through to primitive / runtime dispatch.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_class_binop(
        &mut self,
        op: hir::BinOp,
        left_op: mir::Operand,
        left_ty: &Type,
        right_op: mir::Operand,
        right_ty: &Type,
        result_ty: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Option<mir::Operand> {
        let fwd_name = op.forward_dunder();
        let rev_name = op.reflected_dunder();

        // §3.3.8 subclass-first: `right` is a strict subclass of `left` →
        // reflected on right wins over forward on left. Only user classes
        // participate.
        if let (Type::Class { class_id: l_id, .. }, Type::Class { class_id: r_id, .. }) =
            (left_ty, right_ty)
        {
            if l_id != r_id && self.is_proper_subclass(*r_id, *l_id) {
                if let Some(rfunc_id) = self
                    .get_class_info(r_id)
                    .and_then(|ci| ci.get_dunder_func(rev_name))
                {
                    let boxed_left = self.box_dunder_arg_if_needed(
                        left_op.clone(),
                        left_ty,
                        rfunc_id,
                        1,
                        hir_module,
                        mir_func,
                    );
                    let dest = self.alloc_dunder_result(rfunc_id, result_ty, hir_module, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest,
                        func: rfunc_id,
                        args: vec![right_op.clone(), boxed_left],
                    });
                    return Some(mir::Operand::Local(dest));
                }
            }
        }

        // Forward dunder on left class.
        if let Type::Class { class_id, .. } = left_ty {
            if let Some(func_id) = self
                .get_class_info(class_id)
                .and_then(|ci| ci.get_dunder_func(fwd_name))
            {
                let boxed_right = self.box_dunder_arg_if_needed(
                    right_op.clone(),
                    right_ty,
                    func_id,
                    1,
                    hir_module,
                    mir_func,
                );
                let dest = self.alloc_dunder_result(func_id, result_ty, hir_module, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest,
                    func: func_id,
                    args: vec![left_op.clone(), boxed_right],
                });
                // NI fallback: if forward may return `NotImplemented` and
                // right has a reflected dunder, emit compare+branch.
                if self.func_may_return_not_implemented(func_id, hir_module) {
                    let reflected_func = match right_ty {
                        Type::Class { class_id: r_id, .. } => self
                            .get_class_info(r_id)
                            .and_then(|ci| ci.get_dunder_func(rev_name)),
                        _ => None,
                    };
                    if let Some(rfunc_id) = reflected_func {
                        let final_local = self.emit_not_implemented_fallback(
                            dest, rfunc_id, right_op, left_op, left_ty, result_ty, hir_module,
                            mir_func,
                        );
                        return Some(mir::Operand::Local(final_local));
                    }
                }
                return Some(mir::Operand::Local(dest));
            }
        }

        // Reflected dunder on right class (e.g. `2 + V()` → `V.__radd__(2)`).
        if let Type::Class { class_id, .. } = right_ty {
            if let Some(func_id) = self
                .get_class_info(class_id)
                .and_then(|ci| ci.get_dunder_func(rev_name))
            {
                let boxed_left = self
                    .box_dunder_arg_if_needed(left_op, left_ty, func_id, 1, hir_module, mir_func);
                let dest = self.alloc_dunder_result(func_id, result_ty, hir_module, mir_func);
                self.emit_instruction(mir::InstructionKind::CallDirect {
                    dest,
                    func: func_id,
                    args: vec![right_op, boxed_left],
                });
                return Some(mir::Operand::Local(dest));
            }
        }

        None
    }

    /// ```ignore
    /// if forward_result is NotImplemented:
    ///     final = right.__rop__(left)
    /// else:
    ///     final = forward_result
    /// ```
    /// Returns the `final` local id (typed as `result_ty`).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::expressions) fn emit_not_implemented_fallback(
        &mut self,
        forward_result: pyaot_utils::LocalId,
        reflected_func: pyaot_utils::FuncId,
        right_op: mir::Operand,
        left_op: mir::Operand,
        left_ty: &Type,
        result_ty: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> pyaot_utils::LocalId {
        // Materialize the NotImplemented singleton for identity comparison.
        let ni_local = self.alloc_and_add_local(Type::NotImplementedT, mir_func);
        self.emit_instruction(mir::InstructionKind::RuntimeCall {
            dest: ni_local,
            func: mir::RuntimeFunc::Call(
                &pyaot_core_defs::runtime_func_def::RT_NOT_IMPLEMENTED_SINGLETON,
            ),
            args: vec![],
        });

        // Compare forward_result == NotImplemented (pointer equality).
        let is_ni = self.alloc_and_add_local(Type::Bool, mir_func);
        self.emit_instruction(mir::InstructionKind::BinOp {
            dest: is_ni,
            op: mir::BinOp::Eq,
            left: mir::Operand::Local(forward_result),
            right: mir::Operand::Local(ni_local),
        });

        // Output local — receives the merged value from both branches.
        // Use HeapAny (→ Tagged mir_ty) because the two branches may produce
        // different concrete Heap shapes (e.g. forward __mul__ returns Str while
        // reflected __rmul__ also returns Str, but result_ty was Class(LhsType)).
        // Tagged accepts any heap-pointer-shaped source in the verifier.
        let final_local = self.alloc_and_add_local(Type::Any, mir_func);

        let reflected_bb = self.new_block();
        let cont_bb = self.new_block();
        let else_bb = self.new_block();

        let reflected_id = reflected_bb.id;
        let cont_id = cont_bb.id;
        let else_id = else_bb.id;

        self.current_block_mut().terminator = mir::Terminator::Branch {
            cond: mir::Operand::Local(is_ni),
            then_block: reflected_id,
            else_block: else_id,
        };

        // `then` branch: dispatch reflected dunder.
        self.push_block(reflected_bb);
        let boxed_left = self.box_dunder_arg_if_needed(
            left_op,
            left_ty,
            reflected_func,
            1,
            hir_module,
            mir_func,
        );
        let refl_result = self.alloc_dunder_result(reflected_func, result_ty, hir_module, mir_func);
        self.emit_instruction(mir::InstructionKind::CallDirect {
            dest: refl_result,
            func: reflected_func,
            args: vec![right_op, boxed_left],
        });
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: final_local,
            src: mir::Operand::Local(refl_result),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(cont_id);

        // `else` branch: forward result is the final value.
        self.push_block(else_bb);
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: final_local,
            src: mir::Operand::Local(forward_result),
        });
        self.current_block_mut().terminator = mir::Terminator::Goto(cont_id);

        self.push_block(cont_bb);
        final_local
    }

    /// Allocate the destination local for a user dunder call using the
    /// dunder's *actual* return type when known — instead of the outer
    /// `result_ty` heuristic that assumes numeric dunders return `Self`.
    /// Required because comparison dunders return `bool`, `__str__` returns
    /// `str`, user-defined dunders may legitimately return any type, etc.
    pub(in crate::expressions) fn alloc_dunder_result(
        &mut self,
        func_id: pyaot_utils::FuncId,
        fallback_ty: &Type,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> pyaot_utils::LocalId {
        let ret_ty = self
            .get_func_return_type(&func_id)
            .cloned()
            .or_else(|| {
                hir_module
                    .func_defs
                    .get(&func_id)
                    .and_then(|f| f.return_type.clone())
            })
            .unwrap_or_else(|| fallback_ty.clone());
        self.alloc_and_add_local(ret_ty, mir_func)
    }

    /// When a user dunder declares a polymorphic `other` parameter (typically
    /// `Union[Self, int, float, bool]` from unannotated numeric dunders, or
    /// `Any` from unannotated comparison dunders), the function signature at
    /// Cranelift level expects a heap pointer. Primitive arguments (int, bool,
    /// float, None) must be boxed before the call.
    ///
    /// `param_idx` is the 0-based index of the parameter in the target
    /// function's signature. Returns the operand unchanged when the
    /// parameter is concrete or when the argument is already heap-typed.
    pub(crate) fn box_dunder_arg_if_needed(
        &mut self,
        operand: mir::Operand,
        arg_ty: &Type,
        func_id: pyaot_utils::FuncId,
        param_idx: usize,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        // Box args for Any/Union/HeapAny-typed params (tagged Value consumers).
        let needs_box = hir_module
            .func_defs
            .get(&func_id)
            .and_then(|f| f.params.get(param_idx))
            .and_then(|p| p.ty.as_ref())
            .is_some_and(|t| matches!(t, Type::Any | Type::Union(_)));
        if needs_box {
            self.emit_value_slot(operand, arg_ty, mir_func)
        } else {
            operand
        }
    }
}
