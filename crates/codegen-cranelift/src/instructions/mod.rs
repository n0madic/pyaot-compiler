//! Instruction compilation dispatch
//!
//! Organized into submodules by instruction category:
//! - `calls`: CallDirect, CallNamed, Call, CallVirtual, CallVirtualNamed, FuncAddr, BuiltinAddr
//! - `arithmetic`: BinOp, comparison helpers

pub(crate) mod arithmetic;
pub(crate) mod calls;

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_types::Type;

use crate::context::CodegenContext;
use crate::exceptions::{
    compile_exc_check_class, compile_exc_check_type, compile_exc_clear, compile_exc_end_handling,
    compile_exc_get_current, compile_exc_get_type, compile_exc_has_exception,
    compile_exc_pop_frame, compile_exc_push_frame, compile_exc_start_handling,
};
use crate::runtime_calls::compile_runtime_call;
use crate::utils::{declare_runtime_function, is_float_operand, load_operand};

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

        mir::InstructionKind::Copy { dest, src } => {
            compile_copy(builder, dest, src, ctx)?;
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

        mir::InstructionKind::UnOp { dest, op, operand } => {
            let operand_val = load_operand(builder, operand, ctx.symbols.var_map);
            let is_float = is_float_operand(operand, ctx.symbols.locals);
            let result = match op {
                mir::UnOp::Neg => {
                    if is_float {
                        builder.ins().fneg(operand_val)
                    } else {
                        builder.ins().ineg(operand_val)
                    }
                }
                mir::UnOp::Not => {
                    let val_type = builder.func.dfg.value_type(operand_val);
                    if val_type == cltypes::I8 {
                        let one = builder.ins().iconst(cltypes::I8, 1);
                        builder.ins().isub(one, operand_val)
                    } else {
                        let zero = builder.ins().iconst(cltypes::I64, 0);
                        builder.ins().icmp(
                            cranelift_codegen::ir::condcodes::IntCC::Equal,
                            operand_val,
                            zero,
                        )
                    }
                }
                mir::UnOp::Invert => builder.ins().bnot(operand_val),
            };
            ctx.store_result(builder, dest, result);
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
            let src_val = load_operand(builder, src, ctx.symbols.var_map);
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::F64));
            sig.returns.push(AbiParam::new(cltypes::I64));
            let func_id = declare_runtime_function(ctx.module, "rt_float_to_int", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let call = builder.ins().call(func_ref, &[src_val]);
            let result = builder.inst_results(call)[0];
            ctx.store_result(builder, dest, result);
        }
        mir::InstructionKind::BoolToInt { dest, src } => {
            let src_val = load_operand(builder, src, ctx.symbols.var_map);
            let result = builder.ins().uextend(cltypes::I64, src_val);
            ctx.store_result(builder, dest, result);
        }
        mir::InstructionKind::IntToFloat { dest, src } => {
            let src_val = load_operand(builder, src, ctx.symbols.var_map);
            let result = builder.ins().fcvt_from_sint(cltypes::F64, src_val);
            ctx.store_result(builder, dest, result);
        }
        mir::InstructionKind::FloatAbs { dest, src } => {
            let src_val = load_operand(builder, src, ctx.symbols.var_map);
            let result = builder.ins().fabs(src_val);
            ctx.store_result(builder, dest, result);
        }
        mir::InstructionKind::FloatBits { dest, src } => {
            let src_val = load_operand(builder, src, ctx.symbols.var_map);
            let result = builder.ins().bitcast(
                cltypes::I64,
                cranelift_codegen::ir::MemFlags::new(),
                src_val,
            );
            ctx.store_result(builder, dest, result);
        }
        mir::InstructionKind::IntBitsToFloat { dest, src } => {
            let src_val = load_operand(builder, src, ctx.symbols.var_map);
            let result = builder.ins().bitcast(
                cltypes::F64,
                cranelift_codegen::ir::MemFlags::new(),
                src_val,
            );
            ctx.store_result(builder, dest, result);
        }

        // GC instructions are handled at the function level (prologue/epilogue)
        mir::InstructionKind::GcPush { .. }
        | mir::InstructionKind::GcPop
        | mir::InstructionKind::GcAlloc { .. } => {}
    }
    Ok(())
}

/// Compile a Copy instruction with type coercion between MIR types.
fn compile_copy(
    builder: &mut FunctionBuilder,
    dest: &pyaot_utils::LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);

    let src_ty = match src {
        Operand::Local(local_id) => ctx.symbols.locals.get(local_id).map(|l| &l.ty),
        Operand::Constant(c) => Some(match c {
            mir::Constant::Int(_) => &Type::Int,
            mir::Constant::Float(_) => &Type::Float,
            mir::Constant::Bool(_) => &Type::Bool,
            mir::Constant::None => &Type::None,
            _ => &Type::Int,
        }),
    };
    let dest_ty = ctx.symbols.locals.get(dest).map(|l| &l.ty);

    use crate::utils::type_to_cranelift;
    let src_cl_ty = src_ty.map(type_to_cranelift).unwrap_or(cltypes::I64);
    let dest_cl_ty = dest_ty.map(type_to_cranelift).unwrap_or(cltypes::I64);

    let result_val = match (src_cl_ty, dest_cl_ty) {
        (t1, t2) if t1 == t2 => src_val,
        (cltypes::I8, cltypes::I64) => builder.ins().uextend(cltypes::I64, src_val),
        (cltypes::I64, cltypes::I8) => builder.ins().ireduce(cltypes::I8, src_val),
        (cltypes::F64, cltypes::I64) => {
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::F64));
            sig.returns.push(AbiParam::new(cltypes::I64));
            let func_id = declare_runtime_function(ctx.module, "rt_float_to_int", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let call = builder.ins().call(func_ref, &[src_val]);
            builder.inst_results(call)[0]
        }
        (cltypes::I64, cltypes::F64) => builder.ins().fcvt_from_sint(cltypes::F64, src_val),
        (cltypes::F64, cltypes::I8) => {
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::F64));
            sig.returns.push(AbiParam::new(cltypes::I64));
            let func_id = declare_runtime_function(ctx.module, "rt_float_to_int", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let call = builder.ins().call(func_ref, &[src_val]);
            let as_int = builder.inst_results(call)[0];
            builder.ins().ireduce(cltypes::I8, as_int)
        }
        (cltypes::I8, cltypes::F64) => {
            let as_int = builder.ins().uextend(cltypes::I64, src_val);
            builder.ins().fcvt_from_sint(cltypes::F64, as_int)
        }
        _ => {
            #[cfg(debug_assertions)]
            {
                if src_cl_ty != dest_cl_ty {
                    eprintln!(
                        "Warning: Unhandled type conversion {:?} -> {:?} (src: {:?}, dest: {:?})",
                        src_cl_ty, dest_cl_ty, src_ty, dest_ty
                    );
                }
            }
            src_val
        }
    };

    ctx.store_result(builder, dest, result_val);
    Ok(())
}
