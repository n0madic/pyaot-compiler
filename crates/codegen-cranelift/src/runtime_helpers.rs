//! Helper functions for runtime call code generation
//!
//! This module contains helper functions that reduce code duplication
//! in runtime call compilation by providing common patterns for:
//! - Unary operations (str.upper, str.lower, etc.)
//! - Container operations (list.copy, dict.copy, etc.)
//! - Slice operations (list[1:4], tuple[::2], etc.)
//! - Type conversions (int_to_str, box_int, etc.)

// These helper functions have many parameters by design to reduce
// boilerplate in callers - each parameter eliminates repeated code
#![allow(clippy::too_many_arguments)]

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::Operand;
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::utils::{declare_runtime_function, load_operand, load_operand_as};

/// Compile a unary string operation (e.g., str.upper(), str.lower(), str.strip())
pub fn compile_str_unary_op(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // str_obj
    sig.returns.push(AbiParam::new(cltypes::I64)); // result

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let str_obj = load_operand(builder, arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[str_obj]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a container creation operation (e.g., rt_make_dict, rt_make_set)
/// For containers that only take a capacity argument
pub fn compile_make_container(
    builder: &mut FunctionBuilder,
    func_name: &str,
    capacity_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // capacity
    sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let capacity = load_operand(builder, capacity_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[capacity]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a list/tuple creation operation with elem_tag
/// rt_make_list(capacity, elem_tag) and rt_make_tuple(size, elem_tag)
pub fn compile_make_container_with_tag(
    builder: &mut FunctionBuilder,
    func_name: &str,
    capacity_arg: &Operand,
    elem_tag_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // capacity/size
    sig.params.push(AbiParam::new(cltypes::I8)); // elem_tag
    sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let capacity = load_operand(builder, capacity_arg, ctx.var_map);
    let elem_tag_i64 = load_operand(builder, elem_tag_arg, ctx.var_map);
    // Truncate i64 to i8 for the elem_tag parameter
    let elem_tag = builder.ins().ireduce(cltypes::I8, elem_tag_i64);
    let call_inst = builder.ins().call(func_ref, &[capacity, elem_tag]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a container get operation (e.g., list[i], dict[key])
pub fn compile_container_get(
    builder: &mut FunctionBuilder,
    func_name: &str,
    container_arg: &Operand,
    index_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // container
    sig.params.push(AbiParam::new(cltypes::I64)); // index
    sig.returns.push(AbiParam::new(cltypes::I64)); // result

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let container = load_operand(builder, container_arg, ctx.var_map);
    let index = load_operand(builder, index_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[container, index]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a container length operation (e.g., len(list), len(dict), len(tuple))
pub fn compile_container_len(
    builder: &mut FunctionBuilder,
    func_name: &str,
    container_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // container
    sig.returns.push(AbiParam::new(cltypes::I64)); // length

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let container = load_operand(builder, container_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[container]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a 3-argument slice operation (e.g., list[start:end], tuple[1:4])
pub fn compile_slice3(
    builder: &mut FunctionBuilder,
    func_name: &str,
    args: &[Operand],
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // container
    sig.params.push(AbiParam::new(cltypes::I64)); // start
    sig.params.push(AbiParam::new(cltypes::I64)); // end
    sig.returns.push(AbiParam::new(cltypes::I64)); // result

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let container = load_operand(builder, &args[0], ctx.var_map);
    let start = load_operand(builder, &args[1], ctx.var_map);
    let end = load_operand(builder, &args[2], ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[container, start, end]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a 4-argument slice operation with step (e.g., list[start:end:step], tuple[::2])
pub fn compile_slice4(
    builder: &mut FunctionBuilder,
    func_name: &str,
    args: &[Operand],
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // container
    sig.params.push(AbiParam::new(cltypes::I64)); // start
    sig.params.push(AbiParam::new(cltypes::I64)); // end
    sig.params.push(AbiParam::new(cltypes::I64)); // step
    sig.returns.push(AbiParam::new(cltypes::I64)); // result

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let container = load_operand(builder, &args[0], ctx.var_map);
    let start = load_operand(builder, &args[1], ctx.var_map);
    let end = load_operand(builder, &args[2], ctx.var_map);
    let step = load_operand(builder, &args[3], ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[container, start, end, step]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a void container method (e.g., list.clear(), dict.clear(), list.reverse())
pub fn compile_container_void_method(
    builder: &mut FunctionBuilder,
    func_name: &str,
    container_arg: &Operand,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // container

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let container = load_operand(builder, container_arg, ctx.var_map);
    builder.ins().call(func_ref, &[container]);
    Ok(())
}

/// Compile a list binary operation returning i64 (e.g., list.index(), list.count())
pub fn compile_list_binary_to_i64(
    builder: &mut FunctionBuilder,
    func_name: &str,
    list_arg: &Operand,
    value_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // list
    sig.params.push(AbiParam::new(cltypes::I64)); // value
    sig.returns.push(AbiParam::new(cltypes::I64)); // result

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let list = load_operand(builder, list_arg, ctx.var_map);
    let value = load_operand(builder, value_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[list, value]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a list binary operation returning i8 (e.g., list_get_bool)
pub fn compile_list_binary_to_i8(
    builder: &mut FunctionBuilder,
    func_name: &str,
    list_arg: &Operand,
    value_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // list
    sig.params.push(AbiParam::new(cltypes::I64)); // index
    sig.returns.push(AbiParam::new(cltypes::I8)); // result (bool as i8)

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let list = load_operand(builder, list_arg, ctx.var_map);
    let value = load_operand(builder, value_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[list, value]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a container copy operation (e.g., list.copy(), dict.copy())
pub fn compile_container_copy(
    builder: &mut FunctionBuilder,
    func_name: &str,
    container_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // container
    sig.returns.push(AbiParam::new(cltypes::I64)); // new container

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let container = load_operand(builder, container_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[container]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a primitive boxing operation (e.g., rt_box_int, rt_box_bool)
pub fn compile_box_primitive(
    builder: &mut FunctionBuilder,
    func_name: &str,
    param_type: cltypes::Type,
    value_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(param_type)); // value
    sig.returns.push(AbiParam::new(cltypes::I64)); // boxed value

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let value = load_operand(builder, value_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[value]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Compile a to-string conversion operation (e.g., rt_int_to_str, rt_float_to_str)
pub fn compile_to_str(
    builder: &mut FunctionBuilder,
    func_name: &str,
    param_type: cltypes::Type,
    value_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(param_type)); // value
    sig.returns.push(AbiParam::new(cltypes::I64)); // string pointer

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let value = load_operand(builder, value_arg, ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[value]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}

/// Generic helper: Compile a simple unary runtime call (one argument, one return value)
/// For operations like rt_hash_int(i64) -> i64, rt_unbox_float(*Obj) -> f64
/// Automatically coerces argument type to match the function signature
pub fn compile_unary_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg_type: cltypes::Type,
    ret_type: cltypes::Type,
    arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(arg_type));
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let arg_val = load_operand_as(builder, arg, ctx.var_map, arg_type);
    let call_inst = builder.ins().call(func_ref, &[arg_val]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Generic helper: Compile a binary runtime call (two arguments, one return value)
/// For operations like rt_dict_get(dict, key) -> value
/// Automatically coerces argument types to match the function signature
pub fn compile_binary_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg1_type: cltypes::Type,
    arg2_type: cltypes::Type,
    ret_type: cltypes::Type,
    arg1: &Operand,
    arg2: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(arg1_type));
    sig.params.push(AbiParam::new(arg2_type));
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let arg1_val = load_operand_as(builder, arg1, ctx.var_map, arg1_type);
    let arg2_val = load_operand_as(builder, arg2, ctx.var_map, arg2_type);
    let call_inst = builder.ins().call(func_ref, &[arg1_val, arg2_val]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Generic helper: Compile a ternary runtime call (three arguments, one return value)
/// For operations like rt_list_insert(list, index, value)
/// Automatically coerces argument types to match the function signature
pub fn compile_ternary_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg1_type: cltypes::Type,
    arg2_type: cltypes::Type,
    arg3_type: cltypes::Type,
    ret_type: cltypes::Type,
    arg1: &Operand,
    arg2: &Operand,
    arg3: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(arg1_type));
    sig.params.push(AbiParam::new(arg2_type));
    sig.params.push(AbiParam::new(arg3_type));
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let arg1_val = load_operand_as(builder, arg1, ctx.var_map, arg1_type);
    let arg2_val = load_operand_as(builder, arg2, ctx.var_map, arg2_type);
    let arg3_val = load_operand_as(builder, arg3, ctx.var_map, arg3_type);
    let call_inst = builder
        .ins()
        .call(func_ref, &[arg1_val, arg2_val, arg3_val]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Generic helper: Compile a void runtime call (no return value)
/// For operations like rt_list_clear(list), rt_dict_clear(dict)
/// Automatically coerces argument types to match the function signature
pub fn compile_void_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg_types: &[cltypes::Type],
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    for &arg_type in arg_types {
        sig.params.push(AbiParam::new(arg_type));
    }

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    // Load operands with type coercion to match function signature
    let arg_vals: Vec<_> = args
        .iter()
        .zip(arg_types.iter())
        .map(|(arg, &expected_type)| load_operand_as(builder, arg, ctx.var_map, expected_type))
        .collect();
    builder.ins().call(func_ref, &arg_vals);
    Ok(())
}

/// Generic helper: Compile a nullary runtime call (no arguments, one return value)
/// For operations like rt_box_none() -> *Obj
pub fn compile_nullary_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    ret_type: cltypes::Type,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let call_inst = builder.ins().call(func_ref, &[]);
    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Helper for global variable get operations (var_id needs i32 truncation)
/// For rt_global_get_*, rt_global_get(var_id: u32) -> value
pub fn compile_global_get(
    builder: &mut FunctionBuilder,
    func_name: &str,
    ret_type: cltypes::Type,
    var_id_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I32)); // var_id
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let var_id_val = load_operand(builder, var_id_arg, ctx.var_map);
    let var_id_i32 = builder.ins().ireduce(cltypes::I32, var_id_val);
    let call_inst = builder.ins().call(func_ref, &[var_id_i32]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Generic helper: Compile a quaternary runtime call (four arguments, one return value)
/// For operations like rt_sorted_range(start, stop, step, reverse)
/// Automatically coerces argument types to match the function signature
pub fn compile_quaternary_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg1_type: cltypes::Type,
    arg2_type: cltypes::Type,
    arg3_type: cltypes::Type,
    arg4_type: cltypes::Type,
    ret_type: cltypes::Type,
    arg1: &Operand,
    arg2: &Operand,
    arg3: &Operand,
    arg4: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(arg1_type));
    sig.params.push(AbiParam::new(arg2_type));
    sig.params.push(AbiParam::new(arg3_type));
    sig.params.push(AbiParam::new(arg4_type));
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let arg1_val = load_operand_as(builder, arg1, ctx.var_map, arg1_type);
    let arg2_val = load_operand_as(builder, arg2, ctx.var_map, arg2_type);
    let arg3_val = load_operand_as(builder, arg3, ctx.var_map, arg3_type);
    let arg4_val = load_operand_as(builder, arg4, ctx.var_map, arg4_type);
    let call_inst = builder
        .ins()
        .call(func_ref, &[arg1_val, arg2_val, arg3_val, arg4_val]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Generic helper: Compile a quinary runtime call (five arguments, one return value)
/// For operations like rt_filter_new(func_ptr, iter, elem_tag, captures, capture_count)
/// Automatically coerces argument types to match the function signature
pub fn compile_quinary_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg1_type: cltypes::Type,
    arg2_type: cltypes::Type,
    arg3_type: cltypes::Type,
    arg4_type: cltypes::Type,
    arg5_type: cltypes::Type,
    ret_type: cltypes::Type,
    arg1: &Operand,
    arg2: &Operand,
    arg3: &Operand,
    arg4: &Operand,
    arg5: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(arg1_type));
    sig.params.push(AbiParam::new(arg2_type));
    sig.params.push(AbiParam::new(arg3_type));
    sig.params.push(AbiParam::new(arg4_type));
    sig.params.push(AbiParam::new(arg5_type));
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let arg1_val = load_operand_as(builder, arg1, ctx.var_map, arg1_type);
    let arg2_val = load_operand_as(builder, arg2, ctx.var_map, arg2_type);
    let arg3_val = load_operand_as(builder, arg3, ctx.var_map, arg3_type);
    let arg4_val = load_operand_as(builder, arg4, ctx.var_map, arg4_type);
    let arg5_val = load_operand_as(builder, arg5, ctx.var_map, arg5_type);
    let call_inst = builder.ins().call(
        func_ref,
        &[arg1_val, arg2_val, arg3_val, arg4_val, arg5_val],
    );

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Generic helper: Compile a senary runtime call (six arguments, one return value)
/// For operations like rt_reduce(func_ptr, iter, initial, has_initial, captures, capture_count)
pub fn compile_senary_runtime_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg1_type: cltypes::Type,
    arg2_type: cltypes::Type,
    arg3_type: cltypes::Type,
    arg4_type: cltypes::Type,
    arg5_type: cltypes::Type,
    arg6_type: cltypes::Type,
    ret_type: cltypes::Type,
    arg1: &Operand,
    arg2: &Operand,
    arg3: &Operand,
    arg4: &Operand,
    arg5: &Operand,
    arg6: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
    update_gc: bool,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(arg1_type));
    sig.params.push(AbiParam::new(arg2_type));
    sig.params.push(AbiParam::new(arg3_type));
    sig.params.push(AbiParam::new(arg4_type));
    sig.params.push(AbiParam::new(arg5_type));
    sig.params.push(AbiParam::new(arg6_type));
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let arg1_val = load_operand_as(builder, arg1, ctx.var_map, arg1_type);
    let arg2_val = load_operand_as(builder, arg2, ctx.var_map, arg2_type);
    let arg3_val = load_operand_as(builder, arg3, ctx.var_map, arg3_type);
    let arg4_val = load_operand_as(builder, arg4, ctx.var_map, arg4_type);
    let arg5_val = load_operand_as(builder, arg5, ctx.var_map, arg5_type);
    let arg6_val = load_operand_as(builder, arg6, ctx.var_map, arg6_type);
    let call_inst = builder.ins().call(
        func_ref,
        &[arg1_val, arg2_val, arg3_val, arg4_val, arg5_val, arg6_val],
    );

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    if update_gc {
        update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    }
    Ok(())
}

/// Helper for global variable set operations (var_id needs i32 truncation)
/// For rt_global_set_*, rt_global_set(var_id: u32, value)
pub fn compile_global_set(
    builder: &mut FunctionBuilder,
    func_name: &str,
    value_type: cltypes::Type,
    var_id_arg: &Operand,
    value_arg: &Operand,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I32)); // var_id
    sig.params.push(AbiParam::new(value_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let var_id_val = load_operand(builder, var_id_arg, ctx.var_map);
    let var_id_i32 = builder.ins().ireduce(cltypes::I32, var_id_val);
    let value_val = load_operand(builder, value_arg, ctx.var_map);
    builder.ins().call(func_ref, &[var_id_i32, value_val]);
    Ok(())
}
