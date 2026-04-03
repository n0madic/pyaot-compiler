//! Print operations code generation
//!
//! Only handles special cases that embed string constants in the binary:
//! - AssertFail: embeds null-terminated message string
//! - PrintValue(Str): embeds null-terminated C string for raw printing
//! - PrintValue(None): prints literal "None" (no argument)
//!
//! All other print operations are migrated to RuntimeFunc::Call(&RuntimeFuncDef).

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand, PrintKind};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::{create_string_data, declare_runtime_function, load_operand};

/// Compile a print-related runtime call (special cases only)
pub fn compile_print_call(
    builder: &mut FunctionBuilder,
    _dest: LocalId,
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
        mir::RuntimeFunc::PrintValue(kind) => {
            compile_print_value(builder, *kind, args, ctx)?;
        }
        _ => unreachable!("Non-print function passed to compile_print_call"),
    }

    Ok(())
}

/// Compile a PrintValue call for Str and None kinds only.
/// Other PrintKind variants are handled via RuntimeFunc::Call(&RuntimeFuncDef).
fn compile_print_value(
    builder: &mut FunctionBuilder,
    kind: PrintKind,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    match kind {
        PrintKind::Str => {
            // Raw string pointer (null-terminated C string embedded in binary)
            sig.params.push(AbiParam::new(cltypes::I64));
        }
        PrintKind::None => {
            // No parameter
        }
        _ => unreachable!(
            "PrintValue({kind:?}) should use RuntimeFunc::Call descriptor, not special codegen"
        ),
    }

    let func_id = declare_runtime_function(ctx.module, kind.runtime_func_name(), &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    if kind == PrintKind::Str {
        let val = if !args.is_empty() {
            if let Operand::Constant(mir::Constant::Str(s)) = &args[0] {
                let data_id = create_string_data(ctx.module, *s, ctx.interner);
                let gv = ctx.module.declare_data_in_func(data_id, builder.func);
                builder.ins().global_value(cltypes::I64, gv)
            } else {
                load_operand(builder, &args[0], ctx.var_map)
            }
        } else {
            builder.ins().iconst(cltypes::I64, 0)
        };
        builder.ins().call(func_ref, &[val]);
    } else {
        // PrintKind::None - no arguments
        builder.ins().call(func_ref, &[]);
    }

    Ok(())
}
