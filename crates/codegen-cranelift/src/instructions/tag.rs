//! Value tag boxing/unboxing instructions
//!
//! Handles ValueFromInt, UnwrapValueInt, ValueFromBool, and UnwrapValueBool.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::InstBuilder;
use cranelift_frontend::FunctionBuilder;
use pyaot_core_defs::tag::{BOOL_SHIFT, BOOL_TAG, INT_SHIFT, INT_TAG};
use pyaot_mir::Operand;
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::load_operand;

/// Compile ValueFromInt: `(src << 3) | 1`
pub(crate) fn compile_value_from_int(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let shifted = builder.ins().ishl_imm(src_val, INT_SHIFT as i64);
    let tagged = builder.ins().bor_imm(shifted, INT_TAG as i64);
    ctx.store_result(builder, dest, tagged);
}

/// Compile UnwrapValueInt: arithmetic right shift `(v as i64) >> 3`
pub(crate) fn compile_unwrap_value_int(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let result = builder.ins().sshr_imm(src_val, INT_SHIFT as i64);
    ctx.store_result(builder, dest, result);
}

/// Compile ValueFromBool: zero-extend i8 to i64, then `(b << 3) | 3`
pub(crate) fn compile_value_from_bool(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let extended = builder.ins().uextend(cltypes::I64, src_val);
    let shifted = builder.ins().ishl_imm(extended, BOOL_SHIFT as i64);
    let tagged = builder.ins().bor_imm(shifted, BOOL_TAG as i64);
    ctx.store_result(builder, dest, tagged);
}

/// Compile UnwrapValueBool: `((v >> 3) & 1) as i8`
pub(crate) fn compile_unwrap_value_bool(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    ctx: &mut CodegenContext,
) {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    // Type inference may narrow a HeapAny Protocol-iter value_local to Bool (I8)
    // because the iterator's element type is Bool.  The tagged bit-pattern for
    // Bool fits in I8 (true=11, false=3), so the shift+mask still works, but
    // Cranelift rejects `ireduce.i8 <i8_val>` (requires ≥ I16 input).
    // When src is already I8, band_imm already yields I8 — skip ireduce.
    let src_type = builder.func.dfg.value_type(src_val);
    let widened = if src_type == cltypes::I8 {
        builder.ins().uextend(cltypes::I64, src_val)
    } else {
        src_val
    };
    let shifted = builder.ins().ushr_imm(widened, BOOL_SHIFT as i64);
    let masked = builder.ins().band_imm(shifted, 1i64);
    let result = builder.ins().ireduce(cltypes::I8, masked);
    ctx.store_result(builder, dest, result);
}
