//! Print operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand, PrintKind};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::utils::{create_string_data, declare_runtime_function, get_call_result, load_operand};

/// Compile a print-related runtime call
pub fn compile_print_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::AssertFail => {
            // Declare rt_assert_fail: extern "C" fn(*const i8) -> !
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // pointer to message

            let func_id = declare_runtime_function(ctx.module, "rt_assert_fail", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            // Load message argument
            let msg_val = if !args.is_empty() {
                // Check if it's a string constant
                if let Operand::Constant(mir::Constant::Str(s)) = &args[0] {
                    // Create a data section for the string
                    let data_id = create_string_data(ctx.module, *s, ctx.interner);
                    let gv = ctx.module.declare_data_in_func(data_id, builder.func);
                    builder.ins().global_value(cltypes::I64, gv)
                } else {
                    load_operand(builder, &args[0], ctx.var_map)
                }
            } else {
                builder.ins().iconst(cltypes::I64, 0)
            };

            builder.ins().call(func_ref, &[msg_val]);
        }
        mir::RuntimeFunc::AssertFailObj => {
            // Declare rt_assert_fail_obj: extern "C" fn(*const Obj) -> !
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // pointer to string object

            let func_id = declare_runtime_function(ctx.module, "rt_assert_fail_obj", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            // Load message argument (string object pointer)
            let msg_val = if !args.is_empty() {
                load_operand(builder, &args[0], ctx.var_map)
            } else {
                builder.ins().iconst(cltypes::I64, 0)
            };

            builder.ins().call(func_ref, &[msg_val]);
        }
        mir::RuntimeFunc::PrintValue(kind) => {
            compile_print_value(builder, *kind, args, ctx)?;
        }
        mir::RuntimeFunc::PrintNewline => {
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;

            let func_id = declare_runtime_function(ctx.module, "rt_print_newline", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            builder.ins().call(func_ref, &[]);
        }
        mir::RuntimeFunc::PrintSep => {
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;

            let func_id = declare_runtime_function(ctx.module, "rt_print_sep", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            builder.ins().call(func_ref, &[]);
        }
        mir::RuntimeFunc::Input => {
            // rt_input(prompt: *mut Obj) -> *mut Obj (str)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // prompt
            sig.returns.push(AbiParam::new(cltypes::I64)); // result str

            let func_id = declare_runtime_function(ctx.module, "rt_input", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let prompt = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[prompt]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: dest local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::PrintSetStderr => {
            // rt_print_set_stderr() - void, no args
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;

            let func_id = declare_runtime_function(ctx.module, "rt_print_set_stderr", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            builder.ins().call(func_ref, &[]);
        }
        mir::RuntimeFunc::PrintSetStdout => {
            // rt_print_set_stdout() - void, no args
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;

            let func_id = declare_runtime_function(ctx.module, "rt_print_set_stdout", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            builder.ins().call(func_ref, &[]);
        }
        mir::RuntimeFunc::PrintFlush => {
            // rt_print_flush() - void, no args
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;

            let func_id = declare_runtime_function(ctx.module, "rt_print_flush", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            builder.ins().call(func_ref, &[]);
        }
        _ => unreachable!("Non-print function passed to compile_print_call"),
    }

    Ok(())
}

/// Compile a PrintValue call based on the PrintKind
fn compile_print_value(
    builder: &mut FunctionBuilder,
    kind: PrintKind,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    // Add parameter based on kind
    match kind {
        PrintKind::Int => sig.params.push(AbiParam::new(cltypes::I64)),
        PrintKind::Float => sig.params.push(AbiParam::new(cltypes::F64)),
        PrintKind::Bool => sig.params.push(AbiParam::new(cltypes::I8)),
        PrintKind::None => {} // No parameter
        PrintKind::Str => sig.params.push(AbiParam::new(cltypes::I64)), // raw string pointer
        PrintKind::StrObj | PrintKind::BytesObj | PrintKind::Obj => {
            sig.params.push(AbiParam::new(cltypes::I64)) // heap object pointer
        }
    }

    let func_id = declare_runtime_function(ctx.module, kind.runtime_func_name(), &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    // Build call arguments
    if kind.has_argument() {
        let val = if kind == PrintKind::Str {
            // For raw strings, handle string constants specially
            if !args.is_empty() {
                if let Operand::Constant(mir::Constant::Str(s)) = &args[0] {
                    let data_id = create_string_data(ctx.module, *s, ctx.interner);
                    let gv = ctx.module.declare_data_in_func(data_id, builder.func);
                    builder.ins().global_value(cltypes::I64, gv)
                } else {
                    load_operand(builder, &args[0], ctx.var_map)
                }
            } else {
                builder.ins().iconst(cltypes::I64, 0)
            }
        } else {
            load_operand(builder, &args[0], ctx.var_map)
        };
        builder.ins().call(func_ref, &[val]);
    } else {
        builder.ins().call(func_ref, &[]);
    }

    Ok(())
}
