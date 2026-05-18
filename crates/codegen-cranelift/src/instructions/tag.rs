//! Value tag boxing/unboxing instructions — `BoxValue` and `UnboxValue`.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_core_defs::tag::{BOOL_SHIFT, BOOL_TAG, INT_SHIFT, INT_TAG};
use pyaot_diagnostics::Result;
use pyaot_mir::Operand;
use pyaot_types::Type;
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile `BoxValue { dest, src, src_type }` — unified box op.
///
/// Dispatches on `src_type`:
/// - `Int`  → inline `(src << 3) | 1`
/// - `Bool` → inline zext + `(b << 3) | 3`
/// - `Float` → `rt_box_float(src)` runtime call; if src already tagged
///   (Cranelift type I64), emit pass-through copy.
/// - `None` → `rt_box_none()` runtime call.
/// - Other (heap shapes, Any) → pass-through copy.
pub(crate) fn compile_box_value(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    src_type: &Type,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);
    let src_cl_ty = builder.func.dfg.value_type(src_val);

    match src_type {
        Type::Int => {
            let shifted = builder.ins().ishl_imm(src_val, INT_SHIFT as i64);
            let tagged = builder.ins().bor_imm(shifted, INT_TAG as i64);
            ctx.store_result(builder, dest, tagged);
            Ok(())
        }
        Type::Bool => {
            let extended = builder.ins().uextend(cltypes::I64, src_val);
            let shifted = builder.ins().ishl_imm(extended, BOOL_SHIFT as i64);
            let tagged = builder.ins().bor_imm(shifted, BOOL_TAG as i64);
            ctx.store_result(builder, dest, tagged);
            Ok(())
        }
        Type::Float => {
            // Pass-through guard: if src is already a tagged Value (i64
            // bits from rt_obj_* / list-element load / etc.), don't bitcast
            // it as f64. Mirror existing emit_value_slot Float-passthrough.
            if src_cl_ty == cltypes::I64 {
                ctx.store_result(builder, dest, src_val);
                return Ok(());
            }
            let boxed = super::calls::box_primitive(
                builder,
                ctx.module,
                "rt_box_float",
                cltypes::F64,
                src_val,
            )?;
            ctx.store_result(builder, dest, boxed);
            Ok(())
        }
        Type::None => {
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.returns.push(AbiParam::new(cltypes::I64));
            let func_id = declare_runtime_function(ctx.module, "rt_box_none", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let call_inst = builder.ins().call(func_ref, &[]);
            let boxed = get_call_result(builder, call_inst);
            ctx.store_result(builder, dest, boxed);
            Ok(())
        }
        _ => {
            // Heap shapes / Any / Union: pass-through. Already a valid
            // tagged Value or 8-byte-aligned heap pointer.
            ctx.store_result(builder, dest, src_val);
            Ok(())
        }
    }
}

/// Compile `UnboxValue { dest, src, dest_type }` — unified unbox op.
///
/// Dispatches on `dest_type`:
/// - `Int`  → arithmetic right shift `>> 3`
/// - `Bool` → `((v >> 3) & 1) as i8`
/// - `Float` → `rt_unbox_float(src)` runtime call (tag-dispatching).
/// - Other → pass-through.
pub(crate) fn compile_unbox_value(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    src: &Operand,
    dest_type: &Type,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let src_val = load_operand(builder, src, ctx.symbols.var_map);

    match dest_type {
        Type::Int => {
            let result = builder.ins().sshr_imm(src_val, INT_SHIFT as i64);
            ctx.store_result(builder, dest, result);
            Ok(())
        }
        Type::Bool => {
            let src_ty = builder.func.dfg.value_type(src_val);
            let widened = if src_ty == cltypes::I8 {
                builder.ins().uextend(cltypes::I64, src_val)
            } else {
                src_val
            };
            let shifted = builder.ins().ushr_imm(widened, BOOL_SHIFT as i64);
            let masked = builder.ins().band_imm(shifted, 1i64);
            let result = builder.ins().ireduce(cltypes::I8, masked);
            ctx.store_result(builder, dest, result);
            Ok(())
        }
        Type::Float => {
            // Pass-through guard: if src is already f64 (optimizer narrowed
            // the local from Any/HeapAny to Float), no unboxing is needed.
            let src_cl_ty = builder.func.dfg.value_type(src_val);
            if src_cl_ty == cltypes::F64 {
                ctx.store_result(builder, dest, src_val);
                return Ok(());
            }
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.returns.push(AbiParam::new(cltypes::F64));
            let func_id = declare_runtime_function(ctx.module, "rt_unbox_float", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let call_inst = builder.ins().call(func_ref, &[src_val]);
            let unboxed = get_call_result(builder, call_inst);
            ctx.store_result(builder, dest, unboxed);
            Ok(())
        }
        _ => {
            ctx.store_result(builder, dest, src_val);
            Ok(())
        }
    }
}
