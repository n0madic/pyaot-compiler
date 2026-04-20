//! Variable binding helpers for match statement lowering.
//!
//! Contains `emit_equality_check()`, `bind_pattern_variables()`,
//! `get_or_create_local_for_var()`, and `emit_pattern_var_assign()`.

use pyaot_diagnostics::Result;
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
        let result_local = match ty {
            Type::Str => {
                // String comparison
                self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        mir::CompareKind::Str.runtime_func_def(mir::ComparisonOp::Eq),
                    ),
                    vec![left, right],
                    Type::Bool,
                    mir_func,
                )
            }
            Type::Int | Type::Bool | Type::Float => {
                // Primitive comparison
                let local = self.alloc_and_add_local(Type::Bool, mir_func);
                self.emit_instruction(mir::InstructionKind::BinOp {
                    dest: local,
                    op: mir::BinOp::Eq,
                    left,
                    right,
                });
                local
            }
            _ => {
                // For other types, use object equality
                self.emit_runtime_call(
                    mir::RuntimeFunc::Call(
                        mir::CompareKind::Obj.runtime_func_def(mir::ComparisonOp::Eq),
                    ),
                    vec![left, right],
                    Type::Bool,
                    mir_func,
                )
            }
        };

        Ok(mir::Operand::Local(result_local))
    }

    /// Get or create a local for a variable. For globals, also registers the
    /// type in `global_var_types`. Does NOT emit the GlobalSet call — use
    /// `emit_pattern_var_assign` when you need to actually write the value.
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

    /// Assign `operand` to a pattern-bound variable. Handles both locals and
    /// module-level globals: for globals, emits the runtime GlobalSet call so
    /// the value is visible outside the match statement.
    pub(super) fn emit_pattern_var_assign(
        &mut self,
        var_id: VarId,
        operand: mir::Operand,
        ty: &Type,
        mir_func: &mut mir::Function,
    ) {
        let local = self.get_or_create_local_for_var(var_id, mir_func, ty);
        // Always update the type map so global_var_types reflects the match-bound type
        // even if the local was already registered with a different/absent type.
        self.insert_var_type(var_id, ty.clone());
        self.emit_instruction(mir::InstructionKind::Copy {
            dest: local,
            src: operand.clone(),
        });

        if self.is_global(&var_id) {
            let runtime_func = self.get_global_set_func(ty);
            let effective_var_id = self.get_effective_var_id(var_id);
            self.emit_runtime_call_void(
                runtime_func,
                vec![
                    mir::Operand::Constant(mir::Constant::Int(effective_var_id)),
                    operand,
                ],
                mir_func,
            );
        }
    }
}
