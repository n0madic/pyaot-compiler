//! Instruction compilation dispatch
//!
//! Organized into submodules by instruction category:
//! - `arithmetic`: BinOp, UnOp, comparison helpers
//! - `calls`: CallDirect, CallNamed, Call, CallVirtual, CallVirtualNamed, FuncAddr, BuiltinAddr
//! - `copy`: Copy with type coercion
//! - `conversions`: FloatToInt, BoolToInt, IntToFloat, FloatAbs, FloatBits, IntBitsToFloat

pub(crate) mod arithmetic;
pub(crate) mod calls;
pub(crate) mod conversions;
pub(crate) mod copy;

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::InstBuilder;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir as mir;

use crate::context::CodegenContext;
use crate::exceptions::{
    compile_exc_check_class, compile_exc_check_type, compile_exc_clear, compile_exc_end_handling,
    compile_exc_get_current, compile_exc_get_type, compile_exc_has_exception,
    compile_exc_pop_frame, compile_exc_push_frame, compile_exc_start_handling,
};
use crate::runtime_calls::compile_runtime_call;

/// Compile a single MIR instruction to Cranelift IR
pub fn compile_instruction(
    builder: &mut FunctionBuilder,
    inst: &mir::Instruction,
    ctx: &mut CodegenContext,
) -> Result<()> {
    match &inst.kind {
        mir::InstructionKind::Const { dest, value } => {
            let val = match value {
                mir::Constant::Int(i) => builder.ins().iconst(cltypes::I64, *i),
                mir::Constant::Float(f) => builder.ins().f64const(*f),
                mir::Constant::Bool(b) => builder.ins().iconst(cltypes::I8, *b as i64),
                mir::Constant::None => builder.ins().iconst(cltypes::I8, 0),
                mir::Constant::Str(_) | mir::Constant::Bytes(_) => {
                    return Err(pyaot_diagnostics::CompilerError::codegen_error(format!(
                        "Const instruction with heap-allocated value {:?} — this should use a RuntimeCall",
                        value
                    )));
                }
            };
            ctx.store_result(builder, dest, val);
        }

        mir::InstructionKind::BinOp {
            dest,
            op,
            left,
            right,
        } => {
            arithmetic::compile_binop(builder, dest, op, left, right, ctx)?;
        }

        mir::InstructionKind::UnOp { dest, op, operand } => {
            arithmetic::compile_unop(builder, dest, op, operand, ctx);
        }

        mir::InstructionKind::Copy { dest, src } => {
            copy::compile_copy(builder, dest, src, ctx)?;
        }

        mir::InstructionKind::CallDirect { dest, func, args } => {
            calls::compile_call_direct(builder, dest, func, args, ctx)?;
        }

        mir::InstructionKind::CallNamed { dest, name, args } => {
            calls::compile_call_named(builder, dest, name, args, ctx)?;
        }

        mir::InstructionKind::Call { dest, func, args } => {
            calls::compile_call_indirect(builder, dest, func, args, ctx)?;
        }

        mir::InstructionKind::CallVirtual {
            dest,
            obj,
            slot,
            args,
        } => {
            calls::compile_call_virtual(builder, dest, obj, *slot, args, ctx)?;
        }

        mir::InstructionKind::CallVirtualNamed {
            dest,
            obj,
            name_hash,
            args,
        } => {
            calls::compile_call_virtual_named(builder, dest, obj, *name_hash, args, ctx)?;
        }

        mir::InstructionKind::FuncAddr { dest, func } => {
            calls::compile_func_addr(builder, dest, func, ctx)?;
        }

        mir::InstructionKind::BuiltinAddr { dest, builtin } => {
            calls::compile_builtin_addr(builder, dest, builtin, ctx)?;
        }

        mir::InstructionKind::RuntimeCall { dest, func, args } => {
            compile_runtime_call(builder, *dest, func, args, ctx)?;
        }

        // Exception handling instructions — delegate to exceptions module
        mir::InstructionKind::ExcPushFrame { frame_local } => {
            compile_exc_push_frame(builder, frame_local, ctx)?;
        }
        mir::InstructionKind::ExcPopFrame => {
            compile_exc_pop_frame(builder, ctx)?;
        }
        mir::InstructionKind::ExcGetType { dest } => {
            compile_exc_get_type(builder, dest, ctx)?;
        }
        mir::InstructionKind::ExcClear => {
            compile_exc_clear(builder, ctx)?;
        }
        mir::InstructionKind::ExcHasException { dest } => {
            compile_exc_has_exception(builder, dest, ctx)?;
        }
        mir::InstructionKind::ExcGetCurrent { dest } => {
            compile_exc_get_current(builder, dest, ctx)?;
        }
        mir::InstructionKind::ExcCheckType { dest, type_tag } => {
            compile_exc_check_type(builder, dest, *type_tag, ctx)?;
        }
        mir::InstructionKind::ExcCheckClass { dest, class_id } => {
            compile_exc_check_class(builder, dest, *class_id, ctx)?;
        }
        mir::InstructionKind::ExcStartHandling => {
            compile_exc_start_handling(builder, ctx)?;
        }
        mir::InstructionKind::ExcEndHandling => {
            compile_exc_end_handling(builder, ctx)?;
        }

        // Type conversion instructions
        mir::InstructionKind::FloatToInt { dest, src } => {
            conversions::compile_float_to_int(builder, dest, src, ctx)?;
        }
        mir::InstructionKind::BoolToInt { dest, src } => {
            conversions::compile_bool_to_int(builder, dest, src, ctx);
        }
        mir::InstructionKind::IntToFloat { dest, src } => {
            conversions::compile_int_to_float(builder, dest, src, ctx);
        }
        mir::InstructionKind::FloatAbs { dest, src } => {
            conversions::compile_float_abs(builder, dest, src, ctx);
        }
        mir::InstructionKind::FloatBits { dest, src } => {
            conversions::compile_float_bits(builder, dest, src, ctx);
        }
        mir::InstructionKind::IntBitsToFloat { dest, src } => {
            conversions::compile_int_bits_to_float(builder, dest, src, ctx);
        }

        // GC instructions are handled at the function level (prologue/epilogue)
        mir::InstructionKind::GcPush { .. }
        | mir::InstructionKind::GcPop
        | mir::InstructionKind::GcAlloc { .. } => {}
    }
    Ok(())
}
