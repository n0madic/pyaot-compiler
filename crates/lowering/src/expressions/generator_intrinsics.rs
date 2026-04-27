//! Lowering for generator intrinsic expressions
//!
//! Each `GeneratorIntrinsic` variant maps 1:1 to a runtime function call.
//! These intrinsics are created by the generator desugaring pass and are
//! never present in the original HIR from the frontend.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a generator intrinsic expression to a MIR RuntimeCall.
    pub(crate) fn lower_generator_intrinsic(
        &mut self,
        intrinsic: &hir::GeneratorIntrinsic,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        use pyaot_core_defs::runtime_func_def::*;

        match intrinsic {
            hir::GeneratorIntrinsic::Create {
                func_id,
                num_locals,
            } => {
                let result_ty = expr
                    .ty
                    .clone()
                    .unwrap_or_else(|| Type::Iterator(Box::new(Type::Any)));
                let dest = self.emit_runtime_call_gc(
                    mir::RuntimeFunc::Call(&RT_MAKE_GENERATOR),
                    vec![
                        mir::Operand::Constant(mir::Constant::Int(*func_id as i64)),
                        mir::Operand::Constant(mir::Constant::Int(*num_locals as i64)),
                    ],
                    result_ty,
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::GetState(gen_expr_id) => {
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let dest = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_GENERATOR_GET_STATE),
                    vec![gen_op],
                    Type::Int,
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::SetState { gen, state } => {
                let gen_op = self.lower_expr(&hir_module.exprs[*gen], hir_module, mir_func)?;
                let dest = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_GENERATOR_SET_STATE),
                    vec![gen_op, mir::Operand::Constant(mir::Constant::Int(*state))],
                    Type::Int,
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::GetLocal { gen, idx } => {
                let gen_op = self.lower_expr(&hir_module.exprs[*gen], hir_module, mir_func)?;
                // §F.7b: locals are uniformly tagged Values. Load the raw
                // Value bits and unbox to the typed representation:
                // - Float: rt_unbox_float (boxed FloatObj pointer)
                // - Int: UnwrapValueInt (inline tag arithmetic)
                // - Bool: UnwrapValueBool (inline tag arithmetic)
                // - None/heap/Any: pass the Value bits through unchanged
                let read_ty = expr.ty.clone().unwrap_or(Type::Int);
                let loaded = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_GENERATOR_GET_LOCAL),
                    vec![
                        gen_op,
                        mir::Operand::Constant(mir::Constant::Int(*idx as i64)),
                    ],
                    Type::HeapAny,
                    mir_func,
                );
                let result = self.unbox_if_needed(mir::Operand::Local(loaded), &read_ty, mir_func);
                Ok(result)
            }

            hir::GeneratorIntrinsic::SetLocal { gen, idx, value } => {
                let gen_op = self.lower_expr(&hir_module.exprs[*gen], hir_module, mir_func)?;
                let val_op = self.lower_expr(&hir_module.exprs[*value], hir_module, mir_func)?;
                // §F.7b: every value stored into a generator local must be a
                // properly-tagged Value so the GC can walk locals uniformly
                // via `Value::is_ptr()`. Use `emit_value_slot` to
                // produce the tagged Value for all primitive types:
                // - Int  → ValueFromInt  (inline tag arithmetic, no alloc)
                // - Bool → ValueFromBool (inline tag arithmetic, no alloc)
                // - Float → rt_box_float (heap-allocated FloatObj pointer)
                // - None → rt_box_none (singleton NoneObj pointer)
                // - Heap/Any → pass through (already a tagged Value/pointer)
                let value_ty = self.seed_expr_type(*value, hir_module);
                let stored_op = self.emit_value_slot(val_op, &value_ty, mir_func);
                let dest = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_GENERATOR_SET_LOCAL),
                    vec![
                        gen_op,
                        mir::Operand::Constant(mir::Constant::Int(*idx as i64)),
                        stored_op,
                    ],
                    Type::Int,
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::SetExhausted(gen_expr_id) => {
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let dest = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_GENERATOR_SET_EXHAUSTED),
                    vec![gen_op],
                    Type::Int,
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::IsExhausted(gen_expr_id) => {
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let dest = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_GENERATOR_IS_EXHAUSTED),
                    vec![gen_op],
                    Type::Bool,
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::GetSentValue(gen_expr_id) => {
                // §P.2.2: rt_generator_send wraps the inbound value via
                // emit_value_slot at the lowering boundary so the
                // sent_value slot stores well-formed Value bits (so the GC
                // doesn't see raw scalars as pointer-shaped non-objects).
                // The read side here unwraps based on the expression's
                // declared / inferred type — symmetric with `g.send()`'s
                // result handling in `lower_generator_method`.
                let gen_op =
                    self.lower_expr(&hir_module.exprs[*gen_expr_id], hir_module, mir_func)?;
                let raw = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_GENERATOR_GET_SENT_VALUE),
                    vec![gen_op],
                    Type::HeapAny,
                    mir_func,
                );
                let elem_ty = expr.ty.clone().unwrap_or(Type::Int);
                let result = self.unbox_if_needed(mir::Operand::Local(raw), &elem_ty, mir_func);
                Ok(result)
            }

            hir::GeneratorIntrinsic::IterNextNoExc(iter_expr_id) => {
                let iter_op =
                    self.lower_expr(&hir_module.exprs[*iter_expr_id], hir_module, mir_func)?;
                // After §F.7c BigBang: list/dict/tuple/set iterators return tagged
                // Value bits. Read into HeapAny, then unwrap Int/Bool for typed callers.
                let iter_ty = self.seed_expr_type(*iter_expr_id, hir_module);
                let dest_ty = crate::utils::get_iterable_info(&iter_ty)
                    .map(|(_k, elem)| elem)
                    .or_else(|| expr.ty.clone())
                    .unwrap_or(Type::Int);
                let raw = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_ITER_NEXT_NO_EXC),
                    vec![iter_op],
                    Type::HeapAny,
                    mir_func,
                );
                let dest = match &dest_ty {
                    Type::Int => {
                        let d = self.alloc_and_add_local(Type::Int, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnwrapValueInt {
                            dest: d,
                            src: mir::Operand::Local(raw),
                        });
                        d
                    }
                    Type::Bool => {
                        let d = self.alloc_and_add_local(Type::Bool, mir_func);
                        self.emit_instruction(mir::InstructionKind::UnwrapValueBool {
                            dest: d,
                            src: mir::Operand::Local(raw),
                        });
                        d
                    }
                    _ => {
                        let d = self.alloc_and_add_local(dest_ty.clone(), mir_func);
                        self.emit_instruction(mir::InstructionKind::Copy {
                            dest: d,
                            src: mir::Operand::Local(raw),
                        });
                        d
                    }
                };
                Ok(mir::Operand::Local(dest))
            }

            hir::GeneratorIntrinsic::IterIsExhausted(iter_expr_id) => {
                let iter_op =
                    self.lower_expr(&hir_module.exprs[*iter_expr_id], hir_module, mir_func)?;
                let dest = self.emit_runtime_call(
                    mir::RuntimeFunc::Call(&RT_ITER_IS_EXHAUSTED),
                    vec![iter_op],
                    Type::Bool,
                    mir_func,
                );
                Ok(mir::Operand::Local(dest))
            }
        }
    }
}
