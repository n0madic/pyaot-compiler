//! Boxing/Unboxing operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::runtime_helpers::{
    compile_box_primitive, compile_nullary_runtime_call, compile_unary_runtime_call,
};

/// Compile a boxing/unboxing-related runtime call
pub fn compile_boxing_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::BoxInt => {
            compile_box_primitive(builder, "rt_box_int", cltypes::I64, &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::BoxBool => {
            compile_box_primitive(builder, "rt_box_bool", cltypes::I8, &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::BoxFloat => {
            compile_box_primitive(builder, "rt_box_float", cltypes::F64, &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::BoxNone => {
            compile_nullary_runtime_call(builder, "rt_box_none", cltypes::I64, dest, ctx, true)?;
        }
        mir::RuntimeFunc::UnboxFloat => {
            compile_unary_runtime_call(
                builder,
                "rt_unbox_float",
                cltypes::I64,
                cltypes::F64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::UnboxInt => {
            compile_unary_runtime_call(
                builder,
                "rt_unbox_int",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::UnboxBool => {
            compile_unary_runtime_call(
                builder,
                "rt_unbox_bool",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        _ => unreachable!("Non-boxing function passed to compile_boxing_call"),
    }
    Ok(())
}
