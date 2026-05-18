//! Control flow statement lowering
//!
//! Handles: Return, If, While, Break, Continue

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a return statement
    pub(crate) fn lower_return(
        &mut self,
        value_expr: Option<&hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        let return_operand = if let Some(expr_id) = value_expr {
            let expr = &hir_module.exprs[*expr_id];
            // Type check: validate return value against function return type
            if let Some(ref ret_ty) = self.symbols.current_func_return_type.clone() {
                self.check_expr_type(*expr_id, ret_ty, hir_module);
            }
            // Bidirectional: propagate function return type into expression
            let expected = self.symbols.current_func_return_type.clone();
            let mut operand = self.lower_expr_expecting(expr, expected, hir_module, mir_func)?;
            // Area E §E.7 — when the function's signature return type is a
            // Union containing `NotImplementedT` (e.g. comparison dunders
            // returning `bool | NotImplementedT`) and the actual value is
            // a raw primitive (`Bool`/`Int`/`Float`/`None`), box it so
            // the Cranelift-level signature (heap pointer) matches.
            if let Some(ret_ty) = self.symbols.current_func_return_type.clone() {
                if ret_ty.is_union()
                    && matches!(&ret_ty, Type::Union(members) if members.contains(&Type::NotImplementedT))
                {
                    let value_ty = self.seed_expr_type(*expr_id, hir_module);
                    if matches!(value_ty, Type::Bool | Type::Int | Type::Float | Type::None) {
                        operand = self.emit_value_slot(operand, &value_ty, mir_func);
                    }
                }
            }
            // Phase 2 explicit numeric tower conversions: when the return
            // operand's type differs from the function return type within
            // the numeric tower (`bool ⊂ int ⊂ float`), emit explicit
            // conversion MIR ops so the MIR is consistent with the
            // signature. Codegen previously inserted these conversions
            // implicitly at the Cranelift level (see compile_terminator
            // in codegen-cranelift::terminators lines 142-163); making
            // them explicit in MIR honours strong typing and lets the
            // verifier's `check_terminator` use stricter rules.
            //
            // This runs unconditionally (both phase4_return_abi_flipped and
            // non-flipped) because BoxValue requires the src operand to be
            // Raw(F64) when src_type=Float — it does not perform numeric
            // promotion itself. Without this conversion, returning an Int
            // from a `-> float` phase4-flipped function emits
            // BoxValue { src: Raw(I64), src_type: Float } which the verifier
            // rejects ("src Raw(I64) doesn't match expected Raw(F64)").
            if let Some(ret_ty) = self.symbols.current_func_return_type.clone() {
                let operand_ty = self.operand_type(&operand, mir_func);
                operand =
                    self.emit_numeric_promotion_if_needed(operand, &operand_ty, &ret_ty, mir_func);
            }
            // Phase 4 Commit 4 — return-ABI flip: callees marked
            // `phase4_return_abi_flipped` ship every Return value as a
            // tagged `Value`. The body computed `operand` against the
            // *declared* primitive return type (`current_func_return_type`),
            // so we box it here. `BoxValue` is a no-op for already-tagged
            // operands (the codegen guard handles the Float case), so
            // routing through the same instruction is safe even if some
            // path on the body already produces a tagged Value.
            // Numeric promotion (above) runs first so the operand type
            // matches src_type before BoxValue is emitted.
            if mir_func.phase4_return_abi_flipped {
                if let Some(ref ret_ty) = self.symbols.current_func_return_type.clone() {
                    if matches!(ret_ty, Type::Int | Type::Bool | Type::Float) {
                        operand = self.emit_value_slot(operand, ret_ty, mir_func);
                    }
                }
            }
            Some(operand)
        } else {
            None
        };
        self.current_block_mut().terminator = mir::Terminator::Return(return_operand);
        Ok(())
    }

    /// Insert explicit MIR conversions for numeric tower promotion when
    /// the operand's HIR type differs from the expected return type.
    /// Mirrors what codegen used to do implicitly at Cranelift level
    /// (uextend / fcvt_from_sint / box).
    fn emit_numeric_promotion_if_needed(
        &mut self,
        operand: mir::Operand,
        operand_ty: &Type,
        ret_ty: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        match (operand_ty, ret_ty) {
            (Type::Bool, Type::Int) => {
                let dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BoolToInt { dest, src: operand });
                mir::Operand::Local(dest)
            }
            (Type::Int, Type::Float) => {
                let dest = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::IntToFloat { dest, src: operand });
                mir::Operand::Local(dest)
            }
            (Type::Bool, Type::Float) => {
                let int_dest = self.alloc_and_add_local(Type::Int, mir_func);
                self.emit_instruction(mir::InstructionKind::BoolToInt {
                    dest: int_dest,
                    src: operand,
                });
                let float_dest = self.alloc_and_add_local(Type::Float, mir_func);
                self.emit_instruction(mir::InstructionKind::IntToFloat {
                    dest: float_dest,
                    src: mir::Operand::Local(int_dest),
                });
                mir::Operand::Local(float_dest)
            }
            _ => operand,
        }
    }

    /// Lower a break statement
    pub(crate) fn lower_break(&mut self) {
        // Jump to the exit block of the innermost loop
        if let Some((_continue_target, break_target)) = self.current_loop() {
            self.current_block_mut().terminator = mir::Terminator::Goto(break_target);
        } else {
            panic!("internal error: break outside loop should be caught by semantic analysis");
        }
    }

    /// Lower a continue statement
    pub(crate) fn lower_continue(&mut self) {
        // Jump to the header block of the innermost loop
        if let Some((continue_target, _break_target)) = self.current_loop() {
            self.current_block_mut().terminator = mir::Terminator::Goto(continue_target);
        } else {
            panic!("internal error: continue outside loop should be caught by semantic analysis");
        }
    }

    /// Convert a condition to boolean for use in if/while/assert branch conditions.
    ///
    /// Delegates to `convert_to_bool()` which handles all types correctly:
    /// Bool → as-is, Int → !=0, Float → !=0.0, Str/List/Dict/Tuple/Set → len>0,
    /// None → false, Union/Any → rt_is_truthy().
    pub(crate) fn emit_truthiness_conversion_if_needed(
        &mut self,
        cond_operand: mir::Operand,
        cond_type: &Type,
        mir_func: &mut mir::Function,
    ) -> mir::Operand {
        if matches!(cond_type, Type::Bool) {
            cond_operand
        } else {
            self.convert_to_bool(cond_operand, cond_type, mir_func)
        }
    }
}
