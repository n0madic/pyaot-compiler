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
            "index" => self.lower_tuple_search(
                obj_operand,
                arg_operands,
                arg_types,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_INDEX),
                mir_func,
            ),
            "count" => self.lower_tuple_search(
                obj_operand,
                arg_operands,
                arg_types,
                mir::RuntimeFunc::Call(&pyaot_core_defs::runtime_func_def::RT_TUPLE_COUNT),
                mir_func,
            ),
            _ => {
                // Unknown tuple method
                Ok(mir::Operand::Constant(mir::Constant::None))
            }
        }
    }

    /// Shared helper for tuple.index() and tuple.count() — both take a single
    /// value argument, conditionally box it, and call a runtime function.
    fn lower_tuple_search(
        &mut self,
        obj_operand: mir::Operand,
        arg_operands: Vec<mir::Operand>,
        arg_types: Vec<Type>,
        func: mir::RuntimeFunc,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let value_arg = crate::first_arg_or_none(arg_operands);
        // Only box the value if it's a heap type; raw types pass through directly.
        let value_type = arg_types.first().cloned().unwrap_or(Type::Any);
        let search_value = if value_type.is_heap() {
            self.box_primitive_if_needed(value_arg, &value_type, mir_func)
        } else {
            value_arg
        };

        let result_local =
            self.emit_runtime_call(func, vec![obj_operand, search_value], Type::Int, mir_func);

        Ok(mir::Operand::Local(result_local))
    }
}
