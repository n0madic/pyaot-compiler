//! Bytes operations code generation

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
    compile_binary_runtime_call, compile_container_len, compile_slice3, compile_slice4,
    compile_ternary_runtime_call, compile_unary_runtime_call,
};
use crate::utils::{
    create_raw_bytes_data, declare_runtime_function, get_call_result, load_operand,
};

/// Compile a bytes-related runtime call
pub fn compile_bytes_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::MakeBytes => {
            // rt_make_bytes(data: *const u8, len: usize) -> *mut Obj
            // Args: [Constant::Bytes(data)]
            if let Operand::Constant(mir::Constant::Bytes(data)) = &args[0] {
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64)); // data pointer
                sig.params.push(AbiParam::new(cltypes::I64)); // length
                sig.returns.push(AbiParam::new(cltypes::I64)); // result pointer

                let func_id = declare_runtime_function(ctx.module, "rt_make_bytes", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

                // Create data section for bytes
                let data_id = create_raw_bytes_data(ctx.module, data);
                let gv = ctx.module.declare_data_in_func(data_id, builder.func);
                let data_ptr = builder.ins().global_value(cltypes::I64, gv);
                let len_val = builder.ins().iconst(cltypes::I64, data.len() as i64);

                let call_inst = builder.ins().call(func_ref, &[data_ptr, len_val]);
                let result_val = get_call_result(builder, call_inst);
                let dest_var = *ctx
                    .var_map
                    .get(&dest)
                    .expect("internal error: local not in var_map - codegen bug");
                builder.def_var(dest_var, result_val);
                update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
            }
        }
        mir::RuntimeFunc::MakeBytesZero => {
            // rt_make_bytes_zero(len: i64) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // length
            sig.returns.push(AbiParam::new(cltypes::I64)); // result pointer

            let func_id = declare_runtime_function(ctx.module, "rt_make_bytes_zero", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let len_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[len_val]);
            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::MakeBytesFromList => {
            // rt_make_bytes_from_list(list: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // list pointer
            sig.returns.push(AbiParam::new(cltypes::I64)); // result pointer

            let func_id = declare_runtime_function(ctx.module, "rt_make_bytes_from_list", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let list_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[list_val]);
            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::MakeBytesFromStr => {
            // rt_make_bytes_from_str(str_obj: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // str pointer
            sig.returns.push(AbiParam::new(cltypes::I64)); // result pointer

            let func_id = declare_runtime_function(ctx.module, "rt_make_bytes_from_str", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let str_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[str_val]);
            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::BytesGet => {
            // rt_bytes_get(bytes: *mut Obj, index: i64) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // bytes pointer
            sig.params.push(AbiParam::new(cltypes::I64)); // index
            sig.returns.push(AbiParam::new(cltypes::I64)); // byte value

            let func_id = declare_runtime_function(ctx.module, "rt_bytes_get", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let bytes_val = load_operand(builder, &args[0], ctx.var_map);
            let index_val = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[bytes_val, index_val]);
            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::BytesLen => {
            // rt_bytes_len(bytes: *mut Obj) -> i64
            compile_container_len(builder, "rt_bytes_len", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::BytesSlice => {
            // rt_bytes_slice(bytes: *mut Obj, start: i64, end: i64) -> *mut Obj
            compile_slice3(builder, "rt_bytes_slice", args, dest, ctx)?;
        }
        mir::RuntimeFunc::BytesSliceStep => {
            // rt_bytes_slice_step(bytes: *mut Obj, start: i64, end: i64, step: i64) -> *mut Obj
            compile_slice4(builder, "rt_bytes_slice_step", args, dest, ctx)?;
        }
        mir::RuntimeFunc::BytesDecode => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_decode",
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
        mir::RuntimeFunc::BytesStartsWith => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_startswith",
                cltypes::I64,
                cltypes::I64,
                cltypes::I8,
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesEndsWith => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_endswith",
                cltypes::I64,
                cltypes::I64,
                cltypes::I8,
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesFind => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_find",
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
        mir::RuntimeFunc::BytesRfind => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_rfind",
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
        mir::RuntimeFunc::BytesIndex => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_index",
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
        mir::RuntimeFunc::BytesRindex => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_rindex",
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
        mir::RuntimeFunc::BytesCount => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_count",
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
        mir::RuntimeFunc::BytesReplace => {
            compile_ternary_runtime_call(
                builder,
                "rt_bytes_replace",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesSplit => {
            compile_ternary_runtime_call(
                builder,
                "rt_bytes_split",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesRsplit => {
            compile_ternary_runtime_call(
                builder,
                "rt_bytes_rsplit",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesJoin => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_join",
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
        mir::RuntimeFunc::BytesStrip => {
            compile_unary_runtime_call(
                builder,
                "rt_bytes_strip",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesLstrip => {
            compile_unary_runtime_call(
                builder,
                "rt_bytes_lstrip",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesRstrip => {
            compile_unary_runtime_call(
                builder,
                "rt_bytes_rstrip",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesUpper => {
            compile_unary_runtime_call(
                builder,
                "rt_bytes_upper",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesLower => {
            compile_unary_runtime_call(
                builder,
                "rt_bytes_lower",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesConcat => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_concat",
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
        mir::RuntimeFunc::BytesRepeat => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_repeat",
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
        mir::RuntimeFunc::BytesFromHex => {
            compile_unary_runtime_call(
                builder,
                "rt_bytes_from_hex",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::BytesContains => {
            compile_binary_runtime_call(
                builder,
                "rt_bytes_contains",
                cltypes::I64,
                cltypes::I64,
                cltypes::I8,
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }
        _ => unreachable!("Non-bytes function passed to compile_bytes_call"),
    }

    Ok(())
}
