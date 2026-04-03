//! Slice expression lowering: obj[start:end:step]

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a slice expression: obj[start:end:step]
    pub(in crate::expressions) fn lower_slice(
        &mut self,
        obj: hir::ExprId,
        start: &Option<hir::ExprId>,
        end: &Option<hir::ExprId>,
        step: &Option<hir::ExprId>,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let obj_expr = &hir_module.exprs[obj];
        let obj_operand = self.lower_expr(obj_expr, hir_module, mir_func)?;
        // Use get_expr_type for proper type inference
        let obj_type = self.get_expr_type(obj_expr, hir_module);

        // Default values for slice: use sentinel values for unspecified start/end
        // i64::MIN for unspecified start, i64::MAX for unspecified end
        // Runtime will apply correct defaults based on step direction
        let start_operand = if let Some(start_id) = start {
            let start_expr = &hir_module.exprs[*start_id];
            self.lower_expr(start_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::Int(i64::MIN))
        };

        // For end, we use i64::MAX to mean "unspecified"
        // The runtime will apply correct default based on step direction
        let end_operand = if let Some(end_id) = end {
            let end_expr = &hir_module.exprs[*end_id];
            self.lower_expr(end_expr, hir_module, mir_func)?
        } else {
            mir::Operand::Constant(mir::Constant::Int(i64::MAX))
        };

        let result_local = self.alloc_and_add_local(obj_type.clone(), mir_func);

        match obj_type {
            Type::Str => {
                if let Some(step_id) = step {
                    // Slice with step
                    let step_expr = &hir_module.exprs[*step_id];
                    let step_operand = self.lower_expr(step_expr, hir_module, mir_func)?;
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::StrSliceStep,
                        args: vec![obj_operand, start_operand, end_operand, step_operand],
                    });
                } else {
                    // Simple slice without step
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::StrSlice,
                        args: vec![obj_operand, start_operand, end_operand],
                    });
                }
            }
            Type::List(_) => {
                if let Some(step_id) = step {
                    // Slice with step
                    let step_expr = &hir_module.exprs[*step_id];
                    let step_operand = self.lower_expr(step_expr, hir_module, mir_func)?;
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_LIST_SLICE_STEP,
                        ),
                        args: vec![obj_operand, start_operand, end_operand, step_operand],
                    });
                } else {
                    // Simple slice without step
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_LIST_SLICE,
                        ),
                        args: vec![obj_operand, start_operand, end_operand],
                    });
                }
            }
            Type::Tuple(_) => {
                if let Some(step_id) = step {
                    // Slice with step
                    let step_expr = &hir_module.exprs[*step_id];
                    let step_operand = self.lower_expr(step_expr, hir_module, mir_func)?;
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_TUPLE_SLICE_STEP,
                        ),
                        args: vec![obj_operand, start_operand, end_operand, step_operand],
                    });
                } else {
                    // Simple slice without step
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_TUPLE_SLICE,
                        ),
                        args: vec![obj_operand, start_operand, end_operand],
                    });
                }
            }
            Type::Bytes => {
                if let Some(step_id) = step {
                    // Slice with step
                    let step_expr = &hir_module.exprs[*step_id];
                    let step_operand = self.lower_expr(step_expr, hir_module, mir_func)?;
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BYTES_SLICE_STEP,
                        ),
                        args: vec![obj_operand, start_operand, end_operand, step_operand],
                    });
                } else {
                    // Simple slice without step
                    self.emit_instruction(mir::InstructionKind::RuntimeCall {
                        dest: result_local,
                        func: mir::RuntimeFunc::Call(
                            &pyaot_core_defs::runtime_func_def::RT_BYTES_SLICE,
                        ),
                        args: vec![obj_operand, start_operand, end_operand],
                    });
                }
            }
            _ => {
                return Ok(mir::Operand::Constant(mir::Constant::None));
            }
        }

        Ok(mir::Operand::Local(result_local))
    }
}
