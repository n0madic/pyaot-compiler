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
                        operand = self.box_primitive_if_needed(operand, &value_ty, mir_func);
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
