//! Copy instruction with type coercion between MIR types

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_types::Type;

use crate::context::CodegenContext;
use crate::utils::{declare_runtime_function, load_operand, type_to_cranelift};

/// Compile a Copy instruction with type coercion between MIR types.
pub(crate) fn compile_copy(
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
