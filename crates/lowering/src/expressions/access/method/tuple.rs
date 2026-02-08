//! Tuple method lowering

use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower tuple method calls.
    pub(super) fn lower_tuple_method(
        &mut self,
        obj_operand: mir::Operand,
        method_name: &str,
        arg_operands: Vec<mir::Operand>,
        arg_types: Vec<Type>,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        match method_name {
            "index" => {
                // .index(value) - returns index of first occurrence
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                let value_arg = arg_operands
                    .into_iter()
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                // Box the value since tuples store *mut Obj
                let value_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_value = self.box_value_for_union(value_arg, &value_type, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::TupleIndex,
                    args: vec![obj_operand, boxed_value],
                });

                Ok(mir::Operand::Local(result_local))
            }
            "count" => {
                // .count(value) - returns count of occurrences
                let result_local = self.alloc_and_add_local(Type::Int, mir_func);

                let value_arg = arg_operands
                    .into_iter()
                    .next()
                    .unwrap_or(mir::Operand::Constant(mir::Constant::None));
                // Box the value since tuples store *mut Obj
                let value_type = arg_types.first().cloned().unwrap_or(Type::Any);
                let boxed_value = self.box_value_for_union(value_arg, &value_type, mir_func);

                self.emit_instruction(mir::InstructionKind::RuntimeCall {
                    dest: result_local,
                    func: mir::RuntimeFunc::TupleCount,
                    args: vec![obj_operand, boxed_value],
                });

                Ok(mir::Operand::Local(result_local))
            }
            _ => {
                // Unknown tuple method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }
}
