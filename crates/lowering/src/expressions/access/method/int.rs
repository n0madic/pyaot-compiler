//! Integer method lowering (`int` / `bool`)
//!
//! `bool` is an `int` subtype in Python, so boolean receivers route here too
//! (`True.bit_length() == 1`). The receiver operand is a raw scalar (i64 for
//! `int`, i8 for `bool`); `bit_length` / `bit_count` take an i64 param, so a
//! `bool` operand is zero-extended to i64 first.

use pyaot_core_defs::runtime_func_def;
use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Lower `int`/`bool` method calls. Returns `Constant::None` for an
    /// unrecognized method (matching the generic fallback), so adding a new
    /// int method is a single arm here plus its runtime function.
    pub(super) fn lower_int_method(
        &mut self,
        obj_operand: mir::Operand,
        obj_type: &Type,
        method_name: &str,
        mir_func: &mut mir::Function,
    ) -> Result<mir::Operand> {
        let def = match method_name {
            "bit_length" => &runtime_func_def::RT_INT_BIT_LENGTH,
            "bit_count" => &runtime_func_def::RT_INT_BIT_COUNT,
            // `(5).conjugate()` / `int(...)` identity-like methods return the
            // value unchanged; the receiver is already the raw int.
            "conjugate" | "__int__" | "__index__" | "__trunc__" => return Ok(obj_operand),
            _ => return Ok(mir::Operand::Constant(mir::Constant::None)),
        };

        // The runtime functions take an i64. A `bool` receiver is a raw i8 —
        // widen it (zero-extend) so codegen passes a full-width argument.
        let arg = if matches!(obj_type, Type::Bool) {
            let widened = self.alloc_and_add_local(Type::Int, mir_func);
            self.emit_instruction(mir::InstructionKind::BoolToInt {
                dest: widened,
                src: obj_operand,
            });
            mir::Operand::Local(widened)
        } else {
            obj_operand
        };

        let result_local =
            self.emit_runtime_call(mir::RuntimeFunc::Call(def), vec![arg], Type::Int, mir_func);
        Ok(mir::Operand::Local(result_local))
    }
}
