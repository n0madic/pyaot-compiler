//! Variable binding helpers for match statement lowering.
//!
//! Contains `emit_equality_check()`, `bind_pattern_variables()`, and
//! `get_or_create_local_for_var()`.

use pyaot_diagnostics::Result;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Emit an equality check between two operands
    pub(super) fn emit_equality_check(
        &mut self,
        left: mir::Operand,
        right: mir::Operand,
        ty: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let result_local = self.alloc_and_add_local(Type::Bool, mir_func);

        match ty {
            Type::Str => {
                // String comparison
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        mir::CompareKind::Str.runtime_func_def(mir::ComparisonOp::Eq),
                    ),
                    args: vec![left, right],
                });
            }
            Type::Int | Type::Bool | Type::Float => {
                // Primitive comparison
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: result_local,
                    op: mir::BinOp::Eq,
                    left,
                    right,
                });
            }
            _ => {
                // For other types, use object equality
                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::Call(
                        mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Eq),
                    ),
                    args: vec![left, right],
                });
            }
        }

        Ok(mir::Operand::Local(result_local))
    }

    /// Bind pattern variables to the subject (for wildcard/as patterns)
    pub(super) fn bind_pattern_variables(
        &mut self,
        pattern: &hir::Pattern,
        subject: mir::Operand,
        subject_type: &Type,
        mir_func: &mut mir::Function,
    ) -> Result<()> {
        match pattern {
            hir::Pattern::MatchAs { pattern, name } => {
                // Recursively bind inner pattern
                if let Some(inner) = pattern {
                    self.bind_pattern_variables(inner, subject.clone(), subject_type, mir_func)?;
                }

                // Bind name to subject
                if let Some(var_id) = name {
                    let local = self.get_or_create_local_for_var(*var_id, mir_func, subject_type);
                    self.emit_instruction(mir::InstructionKind::Copy {
                        dest: local,
                        src: subject,
                    });
                }
            }
            _ => {
                // Other patterns don't need direct binding here
                // (handled in generate_pattern_check)
            }
        }
        Ok(())
    }

    /// Get or create a local for a variable
    pub(super) fn get_or_create_local_for_var(
        &mut self,
        var_id: VarId,
        mir_func: &mut mir::Function,
        ty: &Type,
    ) -> pyaot_utils::LocalId {
        if let Some(local) = self.get_var_local(&var_id) {
            local
        } else {
            let local = self.alloc_and_add_local(ty.clone(), mir_func);
            self.insert_var_local(var_id, local);
            self.insert_var_type(var_id, ty.clone());
            local
        }
    }
}
