//! Unary operation lowering: negation, boolean not, bitwise invert, unary plus

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower a unary operation expression.
    pub(in crate::expressions) fn lower_unop(
        &mut self,
        op: hir::UnOp,
        operand: hir::ExprId,
        hir_module: &hir::Module,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let operand_expr = &hir_module.exprs[operand];
        let operand_op = self.lower_expr(operand_expr, hir_module, mir_func)?;

        // Determine result type based on operation and operand type
        let operand_ty = self.get_expr_type(operand_expr, hir_module);
        let result_type = match op {
            hir::UnOp::Not => Type::Bool,         // not always returns bool
            hir::UnOp::Neg => operand_ty.clone(), // neg preserves operand type
            hir::UnOp::Invert => Type::Int,       // bitwise NOT always returns Int
            hir::UnOp::Pos => operand_ty.clone(), // unary plus preserves type
        };

        let result_local = self.alloc_and_add_local(result_type.clone(), mir_func);

        // Check for class type with unary dunders
        if let Type::Class { class_id, .. } = &operand_ty {
            let dunder_name = match op {
                hir::UnOp::Neg => "__neg__",
                hir::UnOp::Not => "__bool__",
                hir::UnOp::Pos => "__pos__",
                hir::UnOp::Invert => "__invert__",
            };
            let dunder_func = self
                .get_class_info(class_id)
                .and_then(|ci| ci.get_dunder_func(dunder_name));

            if let Some(func_id) = dunder_func {
                if matches!(op, hir::UnOp::Not) {
                    // __bool__ returns bool, then negate
                    let bool_local = self.alloc_and_add_local(Type::Bool, mir_func);
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: bool_local,
                        func: func_id,
                        args: vec![operand_op],
                    });
                    self.emit_instruction(mir::InstructionKind::UnOp {
                        dest: result_local,
                        op: mir::UnOp::Not,
                        operand: mir::Operand::Local(bool_local),
                    });
                } else {
                    // __neg__ returns same type
                    self.emit_instruction(mir::InstructionKind::CallDirect {
                        dest: result_local,
                        func: func_id,
                        args: vec![operand_op],
                    });
                }
                return Ok(mir::Operand::Local(result_local));
            }
        }

        let mir_op = match op {
            hir::UnOp::Neg => mir::UnOp::Neg,
            hir::UnOp::Not => mir::UnOp::Not,
            hir::UnOp::Invert => mir::UnOp::Invert,
            hir::UnOp::Pos => {
                // For primitives, +x is identity (no-op copy)
                self.emit_instruction(mir::InstructionKind::Copy {
                    dest: result_local,
                    src: operand_op,
                });
                return Ok(mir::Operand::Local(result_local));
            }
        };

        // For Not operation, we need to convert the operand to bool first
        // if it's not already a boolean (e.g., Union types need rt_is_truthy)
        let final_operand = if matches!(op, hir::UnOp::Not) {
            self.convert_to_bool(operand_op, &operand_ty, mir_func)
        } else {
            operand_op
        };

        self.emit_instruction(mir::InstructionKind::UnOp {
            dest: result_local,
            op: mir_op,
            operand: final_operand,
        });
        Ok(mir::Operand::Local(result_local))
    }
}
