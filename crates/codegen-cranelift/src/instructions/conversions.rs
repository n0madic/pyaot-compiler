//! Type conversion instructions
//!
//! Handles FloatToInt, BoolToInt, IntToFloat, FloatAbs, FloatBits, and IntBitsToFloat.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::Operand;
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::{declare_runtime_function, load_operand};

/// Compile FloatToInt: call rt_float_to_int(src) -> i64
pub(crate) fn compile_float_to_int(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) -> Result<()> {
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
    Ok(())
}

/// Compile BoolToInt: uextend i8 -> i64
pub(crate) fn compile_bool_to_int(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let result = builder.ins().uextend(cltypes::I64, src_val);
    ctx.store_result(builder, dest, result);
}

/// Compile IntToFloat: fcvt_from_sint i64 -> f64
pub(crate) fn compile_int_to_float(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let result = builder.ins().fcvt_from_sint(cltypes::F64, src_val);
    ctx.store_result(builder, dest, result);
}

/// Compile FloatAbs: fabs f64 -> f64
pub(crate) fn compile_float_abs(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let result = builder.ins().fabs(src_val);
    ctx.store_result(builder, dest, result);
}

/// Compile FloatBits: bitcast f64 -> i64
pub(crate) fn compile_float_bits(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let result = builder
        .ins()
        .bitcast(cltypes::I64, MemFlags::new(), src_val);
    ctx.store_result(builder, dest, result);
}

/// Compile IntBitsToFloat: bitcast i64 -> f64
pub(crate) fn compile_int_bits_to_float(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let result = builder
        .ins()
        .bitcast(cltypes::F64, MemFlags::new(), src_val);
    ctx.store_result(builder, dest, result);
}
