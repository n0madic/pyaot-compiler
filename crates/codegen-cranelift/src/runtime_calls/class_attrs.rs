//! Class attribute operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand, ValueKind};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Get the Cranelift type for a ValueKind
fn value_kind_to_cltype(kind: ValueKind) -> cltypes::Type {
    match kind {
        ValueKind::Int => cltypes::I64,
        ValueKind::Float => cltypes::F64,
        ValueKind::Bool => cltypes::I8,
        ValueKind::Ptr => cltypes::I64,
    }
}

/// Get the runtime function name for ClassAttrSet
fn class_attr_set_func_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Int => "rt_class_attr_set_int",
        ValueKind::Float => "rt_class_attr_set_float",
        ValueKind::Bool => "rt_class_attr_set_bool",
        ValueKind::Ptr => "rt_class_attr_set_ptr",
    }
}

/// Get the runtime function name for ClassAttrGet
fn class_attr_get_func_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Int => "rt_class_attr_get_int",
        ValueKind::Float => "rt_class_attr_get_float",
        ValueKind::Bool => "rt_class_attr_get_bool",
        ValueKind::Ptr => "rt_class_attr_get_ptr",
    }
}

/// Compile a class attribute-related runtime call
pub fn compile_class_attr_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::ClassAttrSet(kind) => {
            // rt_class_attr_set_{int,float,bool,ptr}(class_id: u8, attr_idx: u32, value)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.params.push(AbiParam::new(cltypes::I32)); // attr_idx
            sig.params.push(AbiParam::new(value_kind_to_cltype(*kind))); // value

            let func_id =
                declare_runtime_function(ctx.module, class_attr_set_func_name(*kind), &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let class_id_val = load_operand(builder, &args[0], ctx.var_map);
            let class_id_i8 = builder.ins().ireduce(cltypes::I8, class_id_val);
            let attr_idx_val = load_operand(builder, &args[1], ctx.var_map);
            let attr_idx_i32 = builder.ins().ireduce(cltypes::I32, attr_idx_val);
            let value_val = load_operand(builder, &args[2], ctx.var_map);
            builder
                .ins()
                .call(func_ref, &[class_id_i8, attr_idx_i32, value_val]);
        }

        mir::RuntimeFunc::ClassAttrGet(kind) => {
            // rt_class_attr_get_{int,float,bool,ptr}(class_id: u8, attr_idx: u32) -> value
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.params.push(AbiParam::new(cltypes::I32)); // attr_idx
            sig.returns.push(AbiParam::new(value_kind_to_cltype(*kind))); // value

            let func_id =
                declare_runtime_function(ctx.module, class_attr_get_func_name(*kind), &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let class_id_val = load_operand(builder, &args[0], ctx.var_map);
            let class_id_i8 = builder.ins().ireduce(cltypes::I8, class_id_val);
            let attr_idx_val = load_operand(builder, &args[1], ctx.var_map);
            let attr_idx_i32 = builder.ins().ireduce(cltypes::I32, attr_idx_val);
            let call_inst = builder.ins().call(func_ref, &[class_id_i8, attr_idx_i32]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);

            // Only pointer results need GC tracking
            if matches!(kind, ValueKind::Ptr) {
                update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
            }
        }

        _ => unreachable!("Non-class-attr function passed to compile_class_attr_call"),
    }

    Ok(())
}
