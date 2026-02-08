//! File I/O runtime call code generation

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

/// Compile a file-related runtime call
pub fn compile_file_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::FileOpen => {
            // rt_file_open(filename: *mut Obj, mode: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // filename
            sig.params.push(AbiParam::new(cltypes::I64)); // mode
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, "rt_file_open", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let filename = load_operand(builder, &args[0], ctx.var_map);
            let mode = load_operand(builder, &args[1], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[filename, mode]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::FileRead => {
            // rt_file_read(file: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, "rt_file_read", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::FileReadN => {
            // rt_file_read_n(file: *mut Obj, n: i64) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.params.push(AbiParam::new(cltypes::I64)); // n
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, "rt_file_read_n", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);
            let n = load_operand(builder, &args[1], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file, n]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::FileReadline => {
            // rt_file_readline(file: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, "rt_file_readline", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::FileReadlines => {
            // rt_file_readlines(file: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, "rt_file_readlines", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::FileWrite => {
            // rt_file_write(file: *mut Obj, data: *mut Obj) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.params.push(AbiParam::new(cltypes::I64)); // data
            sig.returns.push(AbiParam::new(cltypes::I64)); // bytes written

            let func_id = declare_runtime_function(ctx.module, "rt_file_write", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);
            let data = load_operand(builder, &args[1], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file, data]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
        }

        mir::RuntimeFunc::FileClose => {
            // rt_file_close(file: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file

            let func_id = declare_runtime_function(ctx.module, "rt_file_close", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            builder.ins().call(func_ref, &[file]);
            // Set dest to 0 (None) - use I8 for None type
            let zero = builder.ins().iconst(cltypes::I8, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }

        mir::RuntimeFunc::FileFlush => {
            // rt_file_flush(file: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file

            let func_id = declare_runtime_function(ctx.module, "rt_file_flush", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            builder.ins().call(func_ref, &[file]);
            // Set dest to 0 (None) - use I8 for None type
            let zero = builder.ins().iconst(cltypes::I8, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }

        mir::RuntimeFunc::FileEnter => {
            // rt_file_enter(file: *mut Obj) -> *mut Obj (returns self)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.returns.push(AbiParam::new(cltypes::I64)); // self

            let func_id = declare_runtime_function(ctx.module, "rt_file_enter", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }

        mir::RuntimeFunc::FileExit => {
            // rt_file_exit(file: *mut Obj) -> i8 (bool)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.returns.push(AbiParam::new(cltypes::I8)); // bool

            let func_id = declare_runtime_function(ctx.module, "rt_file_exit", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
        }

        mir::RuntimeFunc::FileIsClosed => {
            // rt_file_is_closed(file: *mut Obj) -> i8 (bool)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.returns.push(AbiParam::new(cltypes::I8)); // bool

            let func_id = declare_runtime_function(ctx.module, "rt_file_is_closed", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
        }

        mir::RuntimeFunc::FileName => {
            // rt_file_name(file: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // file
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, "rt_file_name", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let file = load_operand(builder, &args[0], ctx.var_map);

            let inst = builder.ins().call(func_ref, &[file]);
            let result = get_call_result(builder, inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
            update_gc_root_if_needed(builder, &dest, result, ctx.gc_frame_data);
        }

        _ => {
            // Unknown file function
        }
    }

    Ok(())
}
