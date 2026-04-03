//! String operations code generation
//!
//! Only handles MakeStr and MakeBytes which require embedding data in the binary.
//! All other string operations are migrated to RuntimeFunc::Call(&RuntimeFuncDef).

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
use crate::utils::{
    create_raw_string_data, declare_runtime_function, get_call_result, load_operand,
};

/// Compile a string-related runtime call (MakeStr / MakeBytes only)
pub fn compile_string_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        // MakeStr has special handling for string constants
        // Compile-time constants use rt_make_str_interned for deduplication
        // Runtime strings use rt_make_str
        mir::RuntimeFunc::MakeStr => {
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // data pointer
            sig.params.push(AbiParam::new(cltypes::I64)); // length
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let (data_ptr, len, is_constant) = if !args.is_empty() {
                if let Operand::Constant(mir::Constant::Str(s)) = &args[0] {
                    let str_content = ctx.interner.resolve(*s);
                    let str_len = str_content.len();
                    let data_id = create_raw_string_data(ctx.module, *s, ctx.interner);
                    let gv = ctx.module.declare_data_in_func(data_id, builder.func);
                    let data_ptr = builder.ins().global_value(cltypes::I64, gv);
                    let len_val = builder.ins().iconst(cltypes::I64, str_len as i64);
                    (data_ptr, len_val, true)
                } else if args.len() >= 2 {
                    let data_ptr = load_operand(builder, &args[0], ctx.var_map);
                    let len = load_operand(builder, &args[1], ctx.var_map);
                    (data_ptr, len, false)
                } else {
                    let zero = builder.ins().iconst(cltypes::I64, 0);
                    (zero, zero, false)
                }
            } else {
                let zero = builder.ins().iconst(cltypes::I64, 0);
                (zero, zero, false)
            };

            // Use interned version for compile-time constants, regular for runtime
            let func_name = if is_constant {
                "rt_make_str_interned"
            } else {
                "rt_make_str"
            };
            let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let call_inst = builder.ins().call(func_ref, &[data_ptr, len]);
            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::MakeBytes => {
            compile_make_bytes(builder, dest, args, ctx)?;
        }
        _ => unreachable!("Non-string function passed to compile_string_call"),
    }
    Ok(())
}

/// Compile MakeBytes: embed bytes constant in binary and call rt_make_bytes(ptr, len)
fn compile_make_bytes(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    if let Operand::Constant(mir::Constant::Bytes(data)) = &args[0] {
        let mut sig = ctx.module.make_signature();
        sig.call_conv = CallConv::SystemV;
        sig.params.push(AbiParam::new(cltypes::I64)); // data pointer
        sig.params.push(AbiParam::new(cltypes::I64)); // length
        sig.returns.push(AbiParam::new(cltypes::I64)); // result pointer

        let func_id = declare_runtime_function(ctx.module, "rt_make_bytes", &sig)?;
        let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

        let data_id = crate::utils::create_raw_bytes_data(ctx.module, data);
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
    Ok(())
}
