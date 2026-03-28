//! Type conversion operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::InstBuilder;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::runtime_helpers::{
    compile_binary_runtime_call, compile_nullary_runtime_call, compile_ternary_runtime_call,
    compile_to_str, compile_unary_runtime_call,
};
use crate::utils::{get_call_result, load_operand};

/// Compile a type conversion-related runtime call
pub fn compile_conversion_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::Convert { from, to } => {
            compile_convert(builder, *from, *to, args, dest, ctx)?;
        }
        mir::RuntimeFunc::StrContains => {
            compile_binary_runtime_call(
                builder,
                "rt_str_contains",
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
        // Number formatting (all i64 -> *Obj)
        mir::RuntimeFunc::IntToBin => {
            compile_unary_runtime_call(
                builder,
                "rt_int_to_bin",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IntToHex => {
            compile_unary_runtime_call(
                builder,
                "rt_int_to_hex",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IntToOct => {
            compile_unary_runtime_call(
                builder,
                "rt_int_to_oct",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        // Format-specific number conversions (no prefix)
        mir::RuntimeFunc::IntFmtBin => {
            compile_unary_runtime_call(
                builder,
                "rt_int_fmt_bin",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IntFmtHex => {
            compile_unary_runtime_call(
                builder,
                "rt_int_fmt_hex",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IntFmtHexUpper => {
            compile_unary_runtime_call(
                builder,
                "rt_int_fmt_hex_upper",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IntFmtOct => {
            compile_unary_runtime_call(
                builder,
                "rt_int_fmt_oct",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IntFmtGrouped => {
            compile_binary_runtime_call(
                builder,
                "rt_int_fmt_grouped",
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
        mir::RuntimeFunc::FloatFmtGrouped => {
            compile_ternary_runtime_call(
                builder,
                "rt_float_fmt_grouped",
                cltypes::F64,
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
        // repr/ascii functions (unified via ToStringRepr)
        mir::RuntimeFunc::ToStringRepr(target_kind, format) => {
            compile_to_string_repr(builder, *target_kind, *format, args, dest, ctx)?;
        }
        mir::RuntimeFunc::TypeName => {
            compile_unary_runtime_call(
                builder,
                "rt_type_name",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::TypeNameExtract => {
            compile_unary_runtime_call(
                builder,
                "rt_type_name_extract",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::ExcClassName => {
            compile_unary_runtime_call(
                builder,
                "rt_exc_class_name",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::FormatValue => {
            // rt_format_value(value: *mut Obj, spec: *mut Obj) -> *mut Obj
            compile_binary_runtime_call(
                builder,
                "rt_format_value",
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
        mir::RuntimeFunc::StrToIntWithBase => {
            // rt_str_to_int_with_base(str: *mut Obj, base: i64) -> i64
            compile_binary_runtime_call(
                builder,
                "rt_str_to_int_with_base",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                dest,
                ctx,
                false, // returns raw i64, not a heap object
            )?;
        }
        _ => unreachable!("Non-conversion function passed to compile_conversion_call"),
    }

    Ok(())
}

/// Compile unified Convert { from, to } operation
fn compile_convert(
    builder: &mut FunctionBuilder,
    from: mir::ConversionTypeKind,
    to: mir::ConversionTypeKind,
    args: &[Operand],
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let func_name = mir::ConversionTypeKind::runtime_func_name(from, to);

    match (from, to) {
        // To string conversions
        (mir::ConversionTypeKind::Int, mir::ConversionTypeKind::Str) => {
            compile_to_str(builder, &func_name, cltypes::I64, &args[0], dest, ctx)?;
        }
        (mir::ConversionTypeKind::Float, mir::ConversionTypeKind::Str) => {
            compile_to_str(builder, &func_name, cltypes::F64, &args[0], dest, ctx)?;
        }
        (mir::ConversionTypeKind::Bool, mir::ConversionTypeKind::Str) => {
            compile_bool_to_str_call(builder, &func_name, &args[0], dest, ctx)?;
        }
        (mir::ConversionTypeKind::None, mir::ConversionTypeKind::Str) => {
            compile_nullary_runtime_call(builder, &func_name, cltypes::I64, dest, ctx, true)?;
        }
        // From string conversions
        (mir::ConversionTypeKind::Str, mir::ConversionTypeKind::Int) => {
            compile_unary_runtime_call(
                builder,
                &func_name,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false, // not a heap allocation (returns raw i64)
            )?;
        }
        (mir::ConversionTypeKind::Str, mir::ConversionTypeKind::Float) => {
            compile_unary_runtime_call(
                builder,
                &func_name,
                cltypes::I64,
                cltypes::F64,
                &args[0],
                dest,
                ctx,
                false, // not a heap allocation (returns raw f64)
            )?;
        }
        _ => unreachable!(
            "Unsupported conversion: {:?} -> {:?}",
            from.name(),
            to.name()
        ),
    }

    Ok(())
}

/// Helper for bool-to-string conversions that need i8 type handling
fn compile_bool_to_str_call(
    builder: &mut FunctionBuilder,
    func_name: &str,
    arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    use crate::utils::declare_runtime_function;
    use cranelift_codegen::ir::AbiParam;
    use cranelift_codegen::isa::CallConv;
    use cranelift_module::Module;

    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I8)); // bool value
    sig.returns.push(AbiParam::new(cltypes::I64)); // string pointer

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let value = load_operand(builder, arg, ctx.var_map);
    let value_type = builder.func.dfg.value_type(value);
    let value_i8 = if value_type == cltypes::I8 {
        value
    } else {
        builder.ins().ireduce(cltypes::I8, value)
    };
    let call_inst = builder.ins().call(func_ref, &[value_i8]);

    let result_val = get_call_result(builder, call_inst);
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);
    crate::gc::update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);

    Ok(())
}

/// Compile unified ToStringRepr (repr/ascii) operation
fn compile_to_string_repr(
    builder: &mut FunctionBuilder,
    target_kind: mir::ReprTargetKind,
    format: mir::StringFormat,
    args: &[Operand],
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Build function name: "rt_repr_int", "rt_ascii_str", etc.
    let func_name = format!("{}{}", format.prefix(), target_kind.suffix());

    match target_kind {
        // Nullary: None takes no arguments
        mir::ReprTargetKind::None => {
            compile_nullary_runtime_call(builder, &func_name, cltypes::I64, dest, ctx, true)?;
        }
        // Bool needs special i8 handling
        mir::ReprTargetKind::Bool => {
            compile_bool_to_str_call(builder, &func_name, &args[0], dest, ctx)?;
        }
        // Float takes f64 input
        mir::ReprTargetKind::Float => {
            compile_unary_runtime_call(
                builder,
                &func_name,
                cltypes::F64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        // Int takes i64 input
        mir::ReprTargetKind::Int => {
            compile_unary_runtime_call(
                builder,
                &func_name,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        // All heap types take i64 (pointer) input
        mir::ReprTargetKind::Str
        | mir::ReprTargetKind::List
        | mir::ReprTargetKind::Tuple
        | mir::ReprTargetKind::Dict
        | mir::ReprTargetKind::Set
        | mir::ReprTargetKind::Bytes
        | mir::ReprTargetKind::Obj => {
            compile_unary_runtime_call(
                builder,
                &func_name,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
    }

    Ok(())
}
