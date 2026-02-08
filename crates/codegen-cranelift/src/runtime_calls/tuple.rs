//! Tuple operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_mir::{self as mir, Operand};
use pyaot_types::Type;
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::runtime_helpers::{
    compile_binary_runtime_call, compile_container_len, compile_make_container_with_tag,
    compile_slice3, compile_slice4,
};
use crate::utils::{declare_runtime_function, get_call_result, is_float_operand, load_operand};

/// Compile a tuple-related runtime call
pub fn compile_tuple_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> pyaot_diagnostics::Result<()> {
    match func {
        mir::RuntimeFunc::MakeTuple => {
            compile_make_container_with_tag(
                builder,
                "rt_make_tuple",
                &args[0],
                &args[1],
                dest,
                ctx,
            )?;
        }
        mir::RuntimeFunc::TupleSet => {
            // rt_tuple_set(tuple: *mut Obj, index: i64, value: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // tuple
            sig.params.push(AbiParam::new(cltypes::I64)); // index
            sig.params.push(AbiParam::new(cltypes::I64)); // value

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_set", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let tuple = load_operand(builder, &args[0], ctx.var_map);
            let index = load_operand(builder, &args[1], ctx.var_map);
            let value_raw = load_operand(builder, &args[2], ctx.var_map);
            // If value is f64 (float), bitcast to i64 for storage
            let value = if is_float_operand(&args[2], ctx.locals) {
                builder
                    .ins()
                    .bitcast(cltypes::I64, MemFlags::new(), value_raw)
            } else {
                value_raw
            };
            builder.ins().call(func_ref, &[tuple, index, value]);
        }
        mir::RuntimeFunc::TupleGet => {
            // rt_tuple_get(tuple: *mut Obj, index: i64) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // tuple
            sig.params.push(AbiParam::new(cltypes::I64)); // index
            sig.returns.push(AbiParam::new(cltypes::I64)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_get", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let tuple = load_operand(builder, &args[0], ctx.var_map);
            let index = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[tuple, index]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            // Check if destination is Float type - need to bitcast i64 to f64
            let final_val = if let Some(local) = ctx.locals.get(&dest) {
                if matches!(local.ty, Type::Float) {
                    builder
                        .ins()
                        .bitcast(cltypes::F64, MemFlags::new(), result_val)
                } else {
                    result_val
                }
            } else {
                result_val
            };
            builder.def_var(dest_var, final_val);
            update_gc_root_if_needed(builder, &dest, final_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleLen => {
            compile_container_len(builder, "rt_tuple_len", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::TupleSlice => {
            compile_slice3(builder, "rt_tuple_slice", args, dest, ctx)?;
        }
        mir::RuntimeFunc::TupleSliceStep => {
            compile_slice4(builder, "rt_tuple_slice_step", args, dest, ctx)?;
        }
        mir::RuntimeFunc::TupleSliceToList => {
            compile_slice3(builder, "rt_tuple_slice_to_list", args, dest, ctx)?;
        }
        mir::RuntimeFunc::TupleGetInt => {
            // rt_tuple_get_int(tuple: *mut Obj, index: i64) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // tuple
            sig.params.push(AbiParam::new(cltypes::I64)); // index
            sig.returns.push(AbiParam::new(cltypes::I64)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_get_int", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let tuple = load_operand(builder, &args[0], ctx.var_map);
            let index = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[tuple, index]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleGetFloat => {
            // rt_tuple_get_float(tuple: *mut Obj, index: i64) -> f64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // tuple
            sig.params.push(AbiParam::new(cltypes::I64)); // index
            sig.returns.push(AbiParam::new(cltypes::F64)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_get_float", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let tuple = load_operand(builder, &args[0], ctx.var_map);
            let index = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[tuple, index]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::TupleGetBool => {
            // rt_tuple_get_bool(tuple: *mut Obj, index: i64) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // tuple
            sig.params.push(AbiParam::new(cltypes::I64)); // index
            sig.returns.push(AbiParam::new(cltypes::I8)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_get_bool", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let tuple = load_operand(builder, &args[0], ctx.var_map);
            let index = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[tuple, index]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::TupleFromList => {
            // rt_tuple_from_list(list: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // list
            sig.returns.push(AbiParam::new(cltypes::I64)); // tuple

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_from_list", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let list = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[list]);
            let result = get_call_result(builder, call_inst);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleFromStr => {
            // rt_tuple_from_str(str: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // str
            sig.returns.push(AbiParam::new(cltypes::I64)); // tuple

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_from_str", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let str_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[str_val]);
            let result = get_call_result(builder, call_inst);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleFromRange => {
            // rt_tuple_from_range(start: i64, stop: i64, step: i64) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // start
            sig.params.push(AbiParam::new(cltypes::I64)); // stop
            sig.params.push(AbiParam::new(cltypes::I64)); // step
            sig.returns.push(AbiParam::new(cltypes::I64)); // tuple

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_from_range", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let start = load_operand(builder, &args[0], ctx.var_map);
            let stop = load_operand(builder, &args[1], ctx.var_map);
            let step = load_operand(builder, &args[2], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[start, stop, step]);
            let result = get_call_result(builder, call_inst);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleFromIter => {
            // rt_tuple_from_iter(iter: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // iter
            sig.returns.push(AbiParam::new(cltypes::I64)); // tuple

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_from_iter", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let iter = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[iter]);
            let result = get_call_result(builder, call_inst);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleFromSet => {
            // rt_tuple_from_set(set: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.returns.push(AbiParam::new(cltypes::I64)); // tuple

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_from_set", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[set]);
            let result = get_call_result(builder, call_inst);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleFromDict => {
            // rt_tuple_from_dict(dict: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // dict
            sig.returns.push(AbiParam::new(cltypes::I64)); // tuple

            let func_id = declare_runtime_function(ctx.module, "rt_tuple_from_dict", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let dict = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[dict]);
            let result = get_call_result(builder, call_inst);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::TupleConcat => {
            // rt_tuple_concat(tuple1: *mut Obj, tuple2: *mut Obj) -> *mut Obj
            compile_binary_runtime_call(
                builder,
                "rt_tuple_concat",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                dest,
                ctx,
                true, // update_gc since result is a heap object
            )?;
        }
        mir::RuntimeFunc::TupleIndex => {
            compile_binary_runtime_call(
                builder,
                "rt_tuple_index",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::TupleCount => {
            compile_binary_runtime_call(
                builder,
                "rt_tuple_count",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }
        _ => unreachable!("Non-tuple function passed to compile_tuple_call"),
    }

    Ok(())
}
