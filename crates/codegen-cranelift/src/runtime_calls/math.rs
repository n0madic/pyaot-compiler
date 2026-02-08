//! Math operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::runtime_helpers::{compile_binary_runtime_call, compile_unary_runtime_call};

/// Compile a math-related runtime call
pub fn compile_math_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::PowFloat => {
            compile_binary_runtime_call(
                builder,
                "rt_pow_float",
                cltypes::F64,
                cltypes::F64,
                cltypes::F64,
                &args[0],
                &args[1],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::PowInt => {
            compile_binary_runtime_call(
                builder,
                "rt_pow_int",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::RoundToInt => {
            compile_unary_runtime_call(
                builder,
                "rt_round_to_int",
                cltypes::F64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::RoundToDigits => {
            compile_binary_runtime_call(
                builder,
                "rt_round_to_digits",
                cltypes::F64,
                cltypes::I64,
                cltypes::F64,
                &args[0],
                &args[1],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::IntToChr => {
            compile_unary_runtime_call(
                builder,
                "rt_int_to_chr",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true, // Returns heap object
            )?;
        }
        mir::RuntimeFunc::ChrToInt => {
            compile_unary_runtime_call(
                builder,
                "rt_chr_to_int",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        _ => unreachable!("Non-math function passed to compile_math_call"),
    }

    Ok(())
}
