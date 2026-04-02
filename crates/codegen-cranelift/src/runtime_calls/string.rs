//! String operations code generation

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
    compile_binary_runtime_call, compile_quaternary_runtime_call, compile_str_unary_op,
    compile_ternary_runtime_call, compile_unary_runtime_call,
};
use crate::utils::{
    create_raw_string_data, declare_runtime_function, get_call_result, load_operand,
};

/// Compile a string-related runtime call
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
        mir::RuntimeFunc::StrData => {
            compile_unary_runtime_call(
                builder,
                "rt_str_data",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::StrLen => {
            compile_unary_runtime_call(
                builder,
                "rt_str_len",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::StrLenInt => {
            compile_unary_runtime_call(
                builder,
                "rt_str_len_int",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrConcat => {
            compile_binary_runtime_call(
                builder,
                "rt_str_concat",
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
        mir::RuntimeFunc::StrSlice => {
            compile_ternary_runtime_call(
                builder,
                "rt_str_slice",
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
        mir::RuntimeFunc::StrSliceStep => {
            compile_quaternary_runtime_call(
                builder,
                "rt_str_slice_step",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                &args[3],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrGetChar => {
            compile_binary_runtime_call(
                builder,
                "rt_str_getchar",
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
        mir::RuntimeFunc::StrSubscript => {
            compile_binary_runtime_call(
                builder,
                "rt_str_subscript",
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
        mir::RuntimeFunc::StrMul => {
            compile_binary_runtime_call(
                builder,
                "rt_str_mul",
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
        // Unary string methods returning string
        mir::RuntimeFunc::StrUpper => {
            compile_str_unary_op(builder, "rt_str_upper", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::StrLower => {
            compile_str_unary_op(builder, "rt_str_lower", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::StrStrip => {
            compile_str_unary_op(builder, "rt_str_strip", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::StrTitle => {
            compile_str_unary_op(builder, "rt_str_title", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::StrCapitalize => {
            compile_str_unary_op(builder, "rt_str_capitalize", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::StrSwapcase => {
            compile_str_unary_op(builder, "rt_str_swapcase", &args[0], dest, ctx)?;
        }
        // String predicate methods returning bool
        mir::RuntimeFunc::StrStartsWith => {
            compile_binary_runtime_call(
                builder,
                "rt_str_startswith",
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
        mir::RuntimeFunc::StrEndsWith => {
            compile_binary_runtime_call(
                builder,
                "rt_str_endswith",
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
        mir::RuntimeFunc::StrSearch(op) => {
            compile_str_search(builder, dest, *op, args, ctx)?;
        }
        mir::RuntimeFunc::StrCount => {
            compile_binary_runtime_call(
                builder,
                "rt_str_count",
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
        mir::RuntimeFunc::StrReplace => {
            compile_ternary_runtime_call(
                builder,
                "rt_str_replace",
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
        mir::RuntimeFunc::StrSplit => {
            compile_ternary_runtime_call(
                builder,
                "rt_str_split",
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
        mir::RuntimeFunc::StrJoin => {
            compile_binary_runtime_call(
                builder,
                "rt_str_join",
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
        mir::RuntimeFunc::StrLstrip => {
            compile_binary_runtime_call(
                builder,
                "rt_str_lstrip",
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
        mir::RuntimeFunc::StrRstrip => {
            compile_binary_runtime_call(
                builder,
                "rt_str_rstrip",
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
        mir::RuntimeFunc::StrCenter => {
            compile_ternary_runtime_call(
                builder,
                "rt_str_center",
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
        mir::RuntimeFunc::StrLjust => {
            compile_ternary_runtime_call(
                builder,
                "rt_str_ljust",
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
        mir::RuntimeFunc::StrRjust => {
            compile_ternary_runtime_call(
                builder,
                "rt_str_rjust",
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
        mir::RuntimeFunc::StrZfill => {
            compile_binary_runtime_call(
                builder,
                "rt_str_zfill",
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
        // Character predicates (all: *Obj -> i8)
        mir::RuntimeFunc::StrIsDigit => {
            compile_unary_runtime_call(
                builder,
                "rt_str_isdigit",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrIsAlpha => {
            compile_unary_runtime_call(
                builder,
                "rt_str_isalpha",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrIsAlnum => {
            compile_unary_runtime_call(
                builder,
                "rt_str_isalnum",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrIsSpace => {
            compile_unary_runtime_call(
                builder,
                "rt_str_isspace",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrIsUpper => {
            compile_unary_runtime_call(
                builder,
                "rt_str_isupper",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrIsLower => {
            compile_unary_runtime_call(
                builder,
                "rt_str_islower",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        // New string methods
        mir::RuntimeFunc::StrRemovePrefix => {
            compile_binary_runtime_call(
                builder,
                "rt_str_removeprefix",
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
        mir::RuntimeFunc::StrRemoveSuffix => {
            compile_binary_runtime_call(
                builder,
                "rt_str_removesuffix",
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
        mir::RuntimeFunc::StrSplitLines => {
            compile_unary_runtime_call(
                builder,
                "rt_str_splitlines",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrPartition => {
            compile_binary_runtime_call(
                builder,
                "rt_str_partition",
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
        mir::RuntimeFunc::StrRpartition => {
            compile_binary_runtime_call(
                builder,
                "rt_str_rpartition",
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
        mir::RuntimeFunc::StrExpandTabs => {
            compile_binary_runtime_call(
                builder,
                "rt_str_expandtabs",
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
        // StringBuilder operations
        mir::RuntimeFunc::MakeStringBuilder => {
            compile_unary_runtime_call(
                builder,
                "rt_make_string_builder",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StringBuilderAppend => {
            // void function - no return value
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // builder
            sig.params.push(AbiParam::new(cltypes::I64)); // str

            let func_id = declare_runtime_function(ctx.module, "rt_string_builder_append", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let builder_val = load_operand(builder, &args[0], ctx.var_map);
            let str_val = load_operand(builder, &args[1], ctx.var_map);

            builder.ins().call(func_ref, &[builder_val, str_val]);

            // Set dest to 0 since this is a void function
            let zero = builder.ins().iconst(cltypes::I64, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }
        mir::RuntimeFunc::StringBuilderToStr => {
            compile_unary_runtime_call(
                builder,
                "rt_string_builder_to_str",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrRsplit => {
            compile_ternary_runtime_call(
                builder,
                "rt_str_rsplit",
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
        mir::RuntimeFunc::StrIsAscii => {
            compile_unary_runtime_call(
                builder,
                "rt_str_isascii",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::StrEncode => {
            compile_binary_runtime_call(
                builder,
                "rt_str_encode",
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
        _ => unreachable!("Non-string function passed to compile_string_call"),
    }
    Ok(())
}

/// Compile a unified string search operation (find/rfind/index/rindex)
/// Calls rt_str_search(str, sub, op_tag) -> i64
fn compile_str_search(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    op: mir::SearchOp,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // str_obj
    sig.params.push(AbiParam::new(cltypes::I64)); // sub
    sig.params.push(AbiParam::new(cltypes::I8)); // op_tag
    sig.returns.push(AbiParam::new(cltypes::I64)); // result

    let func_id = declare_runtime_function(ctx.module, "rt_str_search", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let str_val = load_operand(builder, &args[0], ctx.var_map);
    let sub_val = load_operand(builder, &args[1], ctx.var_map);
    let tag = builder.ins().iconst(cltypes::I8, op.to_tag() as i64);
    let call_inst = builder.ins().call(func_ref, &[str_val, sub_val, tag]);

    let result_val = get_call_result(builder, call_inst);
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}
