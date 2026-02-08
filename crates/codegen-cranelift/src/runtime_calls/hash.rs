//! Hash operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::runtime_helpers::compile_unary_runtime_call;

/// Compile a hash-related runtime call
pub fn compile_hash_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::HashInt => {
            compile_unary_runtime_call(
                builder,
                "rt_hash_int",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::HashStr => {
            compile_unary_runtime_call(
                builder,
                "rt_hash_str",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::HashBool => {
            compile_unary_runtime_call(
                builder,
                "rt_hash_bool",
                cltypes::I8,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::IdObj => {
            compile_unary_runtime_call(
                builder,
                "rt_id_obj",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::HashTuple => {
            compile_unary_runtime_call(
                builder,
                "rt_hash_tuple",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        _ => unreachable!("Non-hash function passed to compile_hash_call"),
    }

    Ok(())
}
