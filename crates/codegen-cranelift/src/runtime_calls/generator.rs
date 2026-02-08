//! Generator operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile a generator-related runtime call
pub fn compile_generator_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::MakeGenerator => {
            // rt_make_generator(func_id: u32, num_locals: u32) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I32)); // func_id
            sig.params.push(AbiParam::new(cltypes::I32)); // num_locals
            sig.returns.push(AbiParam::new(cltypes::I64)); // generator pointer

            let func_id = declare_runtime_function(ctx.module, "rt_make_generator", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let func_id_val = load_operand(builder, &args[0], ctx.var_map);
            let num_locals_val = load_operand(builder, &args[1], ctx.var_map);

            // Convert to i32 for the call
            let func_id_i32 = builder.ins().ireduce(cltypes::I32, func_id_val);
            let num_locals_i32 = builder.ins().ireduce(cltypes::I32, num_locals_val);

            let call_inst = builder.ins().call(func_ref, &[func_id_i32, num_locals_i32]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::GeneratorGetState => {
            // rt_generator_get_state(gen: *mut Obj) -> u32
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.returns.push(AbiParam::new(cltypes::I32)); // state

            let func_id = declare_runtime_function(ctx.module, "rt_generator_get_state", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[gen_val]);

            let result_val = get_call_result(builder, call_inst);
            // Extend to i64 for storage
            let result_i64 = builder.ins().uextend(cltypes::I64, result_val);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_i64);
        }

        mir::RuntimeFunc::GeneratorSetState => {
            // rt_generator_set_state(gen: *mut Obj, state: u32)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.params.push(AbiParam::new(cltypes::I32)); // state

            let func_id = declare_runtime_function(ctx.module, "rt_generator_set_state", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let state_val = load_operand(builder, &args[1], ctx.var_map);
            let state_i32 = builder.ins().ireduce(cltypes::I32, state_val);
            builder.ins().call(func_ref, &[gen_val, state_i32]);
        }

        mir::RuntimeFunc::GeneratorGetLocal => {
            // rt_generator_get_local(gen: *mut Obj, index: u32) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.params.push(AbiParam::new(cltypes::I32)); // index
            sig.returns.push(AbiParam::new(cltypes::I64)); // value

            let func_id = declare_runtime_function(ctx.module, "rt_generator_get_local", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let index_val = load_operand(builder, &args[1], ctx.var_map);
            let index_i32 = builder.ins().ireduce(cltypes::I32, index_val);
            let call_inst = builder.ins().call(func_ref, &[gen_val, index_i32]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }

        mir::RuntimeFunc::GeneratorSetLocal => {
            // rt_generator_set_local(gen: *mut Obj, index: u32, value: i64)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.params.push(AbiParam::new(cltypes::I32)); // index
            sig.params.push(AbiParam::new(cltypes::I64)); // value

            let func_id = declare_runtime_function(ctx.module, "rt_generator_set_local", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let index_val = load_operand(builder, &args[1], ctx.var_map);
            let value_val = load_operand(builder, &args[2], ctx.var_map);
            let index_i32 = builder.ins().ireduce(cltypes::I32, index_val);
            builder
                .ins()
                .call(func_ref, &[gen_val, index_i32, value_val]);
        }

        mir::RuntimeFunc::GeneratorGetLocalPtr => {
            // rt_generator_get_local_ptr(gen: *mut Obj, index: u32) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.params.push(AbiParam::new(cltypes::I32)); // index
            sig.returns.push(AbiParam::new(cltypes::I64)); // value pointer

            let func_id = declare_runtime_function(ctx.module, "rt_generator_get_local_ptr", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let index_val = load_operand(builder, &args[1], ctx.var_map);
            let index_i32 = builder.ins().ireduce(cltypes::I32, index_val);
            let call_inst = builder.ins().call(func_ref, &[gen_val, index_i32]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::GeneratorSetLocalPtr => {
            // rt_generator_set_local_ptr(gen: *mut Obj, index: u32, value: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.params.push(AbiParam::new(cltypes::I32)); // index
            sig.params.push(AbiParam::new(cltypes::I64)); // value pointer

            let func_id = declare_runtime_function(ctx.module, "rt_generator_set_local_ptr", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let index_val = load_operand(builder, &args[1], ctx.var_map);
            let value_val = load_operand(builder, &args[2], ctx.var_map);
            let index_i32 = builder.ins().ireduce(cltypes::I32, index_val);
            builder
                .ins()
                .call(func_ref, &[gen_val, index_i32, value_val]);
        }

        mir::RuntimeFunc::GeneratorSetLocalType => {
            // rt_generator_set_local_type(gen: *mut Obj, index: u32, type_tag: u8)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.params.push(AbiParam::new(cltypes::I32)); // index
            sig.params.push(AbiParam::new(cltypes::I8)); // type tag

            let func_id =
                declare_runtime_function(ctx.module, "rt_generator_set_local_type", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let index_val = load_operand(builder, &args[1], ctx.var_map);
            let type_tag_val = load_operand(builder, &args[2], ctx.var_map);
            let index_i32 = builder.ins().ireduce(cltypes::I32, index_val);
            let type_tag_i8 = builder.ins().ireduce(cltypes::I8, type_tag_val);
            builder
                .ins()
                .call(func_ref, &[gen_val, index_i32, type_tag_i8]);
        }

        mir::RuntimeFunc::GeneratorSetExhausted => {
            // rt_generator_set_exhausted(gen: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer

            let func_id = declare_runtime_function(ctx.module, "rt_generator_set_exhausted", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            builder.ins().call(func_ref, &[gen_val]);
        }

        mir::RuntimeFunc::GeneratorIsExhausted => {
            // rt_generator_is_exhausted(gen: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.returns.push(AbiParam::new(cltypes::I8)); // bool result

            let func_id = declare_runtime_function(ctx.module, "rt_generator_is_exhausted", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[gen_val]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }

        mir::RuntimeFunc::GeneratorSend => {
            // rt_generator_send(gen: *mut Obj, value: i64) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.params.push(AbiParam::new(cltypes::I64)); // value to send
            sig.returns.push(AbiParam::new(cltypes::I64)); // yielded value

            let func_id = declare_runtime_function(ctx.module, "rt_generator_send", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let value_val = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[gen_val, value_val]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::GeneratorGetSentValue => {
            // rt_generator_get_sent_value(gen: *mut Obj) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.returns.push(AbiParam::new(cltypes::I64)); // sent value

            let func_id =
                declare_runtime_function(ctx.module, "rt_generator_get_sent_value", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[gen_val]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }

        mir::RuntimeFunc::GeneratorClose => {
            // rt_generator_close(gen: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer

            let func_id = declare_runtime_function(ctx.module, "rt_generator_close", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            builder.ins().call(func_ref, &[gen_val]);
        }

        mir::RuntimeFunc::GeneratorIsClosing => {
            // rt_generator_is_closing(gen: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // generator pointer
            sig.returns.push(AbiParam::new(cltypes::I8)); // bool result

            let func_id = declare_runtime_function(ctx.module, "rt_generator_is_closing", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let gen_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[gen_val]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }

        _ => unreachable!("Non-generator function passed to compile_generator_call"),
    }

    Ok(())
}
