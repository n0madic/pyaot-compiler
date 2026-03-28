//! Dictionary operations code generation

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
use crate::runtime_helpers::{
    compile_binary_runtime_call, compile_container_copy, compile_container_get,
    compile_container_len, compile_container_void_method, compile_make_container,
};
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile a dictionary-related runtime call
pub fn compile_dict_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::MakeDict => {
            compile_make_container(builder, "rt_make_dict", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::DictSet => {
            // rt_dict_set(dict: *mut Obj, key: *mut Obj, value: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // dict
            sig.params.push(AbiParam::new(cltypes::I64)); // key
            sig.params.push(AbiParam::new(cltypes::I64)); // value

            let func_id = declare_runtime_function(ctx.module, "rt_dict_set", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let dict = load_operand(builder, &args[0], ctx.var_map);
            let key = load_operand(builder, &args[1], ctx.var_map);
            let value = load_operand(builder, &args[2], ctx.var_map);
            builder.ins().call(func_ref, &[dict, key, value]);
        }
        mir::RuntimeFunc::DictGet => {
            compile_container_get(builder, "rt_dict_get", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::DictLen => {
            compile_container_len(builder, "rt_dict_len", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::DictContains => {
            // rt_dict_contains(dict: *mut Obj, key: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // dict
            sig.params.push(AbiParam::new(cltypes::I64)); // key
            sig.returns.push(AbiParam::new(cltypes::I8)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_dict_contains", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let dict = load_operand(builder, &args[0], ctx.var_map);
            let key = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[dict, key]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::DictGetDefault => {
            // rt_dict_get_default(dict: *mut Obj, key: *mut Obj, default: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // dict
            sig.params.push(AbiParam::new(cltypes::I64)); // key
            sig.params.push(AbiParam::new(cltypes::I64)); // default
            sig.returns.push(AbiParam::new(cltypes::I64)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_dict_get_default", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let dict = load_operand(builder, &args[0], ctx.var_map);
            let key = load_operand(builder, &args[1], ctx.var_map);
            let default = load_operand(builder, &args[2], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[dict, key, default]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::DictPop => {
            compile_container_get(builder, "rt_dict_pop", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::DictClear => {
            compile_container_void_method(builder, "rt_dict_clear", &args[0], ctx)?;
        }
        mir::RuntimeFunc::DictCopy => {
            compile_container_copy(builder, "rt_dict_copy", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::DictKeys => {
            // rt_dict_keys(dict: *mut Obj, elem_tag: u8) -> *mut Obj
            compile_dict_keys_values(builder, "rt_dict_keys", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::DictValues => {
            // rt_dict_values(dict: *mut Obj, elem_tag: u8) -> *mut Obj
            compile_dict_keys_values(builder, "rt_dict_values", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::DictItems => {
            compile_container_copy(builder, "rt_dict_items", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::DictUpdate => {
            // rt_dict_update(dict: *mut Obj, other: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // dict
            sig.params.push(AbiParam::new(cltypes::I64)); // other

            let func_id = declare_runtime_function(ctx.module, "rt_dict_update", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let dict = load_operand(builder, &args[0], ctx.var_map);
            let other = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[dict, other]);
        }
        mir::RuntimeFunc::DictFromPairs => {
            // rt_dict_from_pairs(pairs: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // pairs
            sig.returns.push(AbiParam::new(cltypes::I64)); // dict

            let func_id = declare_runtime_function(ctx.module, "rt_dict_from_pairs", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let pairs = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[pairs]);
            let result = get_call_result(builder, call_inst);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::DictSetDefault => {
            // rt_dict_setdefault(dict: *mut Obj, key: *mut Obj, default: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // dict
            sig.params.push(AbiParam::new(cltypes::I64)); // key
            sig.params.push(AbiParam::new(cltypes::I64)); // default
            sig.returns.push(AbiParam::new(cltypes::I64)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_dict_setdefault", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let dict = load_operand(builder, &args[0], ctx.var_map);
            let key = load_operand(builder, &args[1], ctx.var_map);
            let default = load_operand(builder, &args[2], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[dict, key, default]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::DictPopItem => {
            // rt_dict_popitem(dict: *mut Obj) -> *mut Obj (tuple)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // dict
            sig.returns.push(AbiParam::new(cltypes::I64)); // result tuple

            let func_id = declare_runtime_function(ctx.module, "rt_dict_popitem", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let dict = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[dict]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::DictFromKeys => {
            compile_binary_runtime_call(
                builder,
                "rt_dict_fromkeys",
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
        mir::RuntimeFunc::DictMerge => {
            compile_binary_runtime_call(
                builder,
                "rt_dict_merge",
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
        // DefaultDict operations
        mir::RuntimeFunc::MakeDefaultDict => {
            // rt_make_defaultdict(capacity: i64, factory_tag: i64) -> *mut Obj
            compile_binary_runtime_call(
                builder,
                "rt_make_defaultdict",
                cltypes::I64, // capacity
                cltypes::I64, // factory_tag
                cltypes::I64, // return ptr
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::DefaultDictGet => {
            // rt_defaultdict_get(dd: *mut Obj, key: *mut Obj) -> *mut Obj
            compile_container_get(builder, "rt_defaultdict_get", &args[0], &args[1], dest, ctx)?;
        }

        // Counter operations
        mir::RuntimeFunc::MakeCounterFromIter => {
            compile_make_container(builder, "rt_make_counter_from_iter", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::MakeCounterEmpty => {
            // rt_make_counter_empty() -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.returns.push(AbiParam::new(cltypes::I64));
            let func_id = declare_runtime_function(ctx.module, "rt_make_counter_empty", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let call_inst = builder.ins().call(func_ref, &[]);
            let result_val = get_call_result(builder, call_inst);
            let dest_var = ctx.var_map[&dest];
            builder.def_var(dest_var, result_val);
        }

        // Deque operations
        mir::RuntimeFunc::MakeDeque => {
            compile_make_container(builder, "rt_make_deque", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::MakeDequeFromIter => {
            // rt_deque_from_iter(iter: *mut Obj, maxlen: i64) -> *mut Obj
            compile_binary_runtime_call(
                builder,
                "rt_deque_from_iter",
                cltypes::I64, // iter
                cltypes::I64, // maxlen
                cltypes::I64, // return ptr
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }

        _ => unreachable!("Non-dict function passed to compile_dict_call"),
    }

    Ok(())
}

/// Compile rt_dict_keys/rt_dict_values calls with elem_tag parameter.
/// Signature: fn(dict: *mut Obj, elem_tag: u8) -> *mut Obj
fn compile_dict_keys_values(
    builder: &mut FunctionBuilder,
    func_name: &str,
    dict_arg: &Operand,
    elem_tag_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // dict: *mut Obj
    sig.params.push(AbiParam::new(cltypes::I8)); // elem_tag: u8
    sig.returns.push(AbiParam::new(cltypes::I64)); // -> *mut Obj

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let dict_val = load_operand(builder, dict_arg, ctx.var_map);
    let elem_tag_i64 = load_operand(builder, elem_tag_arg, ctx.var_map);
    let elem_tag = builder.ins().ireduce(cltypes::I8, elem_tag_i64);
    let call_inst = builder.ins().call(func_ref, &[dict_val, elem_tag]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);

    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}
