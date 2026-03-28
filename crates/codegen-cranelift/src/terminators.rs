//! Terminator compilation
//!
//! This module handles code generation for MIR terminators including
//! Return, Goto, Branch, Unreachable, and exception-related terminators.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::CodegenContext;
use crate::exceptions::{
    compile_raise, compile_raise_custom, compile_raise_instance, compile_reraise,
    compile_try_setjmp,
};
use crate::utils::{declare_runtime_function, get_call_result, load_operand, type_to_cranelift};

/// Compile a MIR terminator to Cranelift IR
pub fn compile_terminator(
    builder: &mut FunctionBuilder,
    term: &mir::Terminator,
    ctx: &mut CodegenContext,
) -> Result<()> {
    match term {
        mir::Terminator::Return(val) => {
            // Pop traceback frame before returning (always — every function pushed one)
            if let Some(stack_pop_id) = ctx.stack_pop_id {
                let stack_pop_ref = ctx.module.declare_func_in_func(stack_pop_id, builder.func);
                builder.ins().call(stack_pop_ref, &[]);
            }

            // Call gc_pop before returning if we have GC roots
            if ctx.gc_frame_data.is_some() {
                if let Some(gc_pop_id) = ctx.gc_pop_id {
                    let gc_pop_ref = ctx.module.declare_func_in_func(gc_pop_id, builder.func);
                    builder.ins().call(gc_pop_ref, &[]);
                }
            }

            // Only skip returning a value if the function's return type is exactly None.
            // For Union types containing None (e.g., Point | None), we still need to return a value.
            if matches!(ctx.return_type, Type::None) {
                builder.ins().return_(&[]);
            } else if let Some(operand) = val {
                // Check if we're returning a None constant for a non-None return type (e.g., Union)
                // In this case, we need to call rt_box_none to get the boxed None singleton
                let is_none_constant =
                    matches!(operand, mir::Operand::Constant(mir::Constant::None));
                if is_none_constant {
                    // Box the None value for Union types
                    let mut sig = ctx.module.make_signature();
                    sig.call_conv = CallConv::SystemV;
                    sig.returns
                        .push(cranelift_codegen::ir::AbiParam::new(cltypes::I64));
                    let func_id = declare_runtime_function(ctx.module, "rt_box_none", &sig)?;
                    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                    let call_inst = builder.ins().call(func_ref, &[]);
                    let boxed_none = get_call_result(builder, call_inst);
                    builder.ins().return_(&[boxed_none]);
                } else {
                    let ret_val = load_operand(builder, operand, ctx.var_map);
                    // Coerce the return value to the expected return type if needed
                    let expected_type = type_to_cranelift(ctx.return_type);
                    let val_type = builder.func.dfg.value_type(ret_val);
                    let coerced_val = if val_type != expected_type {
                        match (val_type, expected_type) {
                            // i8 to i64 - unsigned extend (for bool values in Union types)
                            (cltypes::I8, cltypes::I64) => {
                                builder.ins().uextend(cltypes::I64, ret_val)
                            }
                            // f64 to i64 - bitcast (resume functions return i64 but field
                            // loads may produce f64; bits are preserved for boxing/unboxing)
                            (cltypes::F64, cltypes::I64) => builder.ins().bitcast(
                                cltypes::I64,
                                cranelift_codegen::ir::MemFlags::new(),
                                ret_val,
                            ),
                            // Other cases - return as-is
                            _ => ret_val,
                        }
                    } else {
                        ret_val
                    };
                    builder.ins().return_(&[coerced_val]);
                }
            } else {
                builder.ins().return_(&[]);
            }
        }

        mir::Terminator::Goto(target) => {
            let cl_block = *ctx
                .block_map
                .get(target)
                .expect("internal error: block not in block_map - codegen bug");
            builder.ins().jump(cl_block, &[]);
        }

        mir::Terminator::Branch {
            cond,
            then_block,
            else_block,
        } => {
            let cond_val = load_operand(builder, cond, ctx.var_map);
            // Convert i8 bool to i1 for brif instruction
            let zero = builder.ins().iconst(cltypes::I8, 0);
            let cond_i1 = builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                cond_val,
                zero,
            );
            let then_cl = *ctx
                .block_map
                .get(then_block)
                .expect("internal error: block not in block_map - codegen bug");
            let else_cl = *ctx
                .block_map
                .get(else_block)
                .expect("internal error: block not in block_map - codegen bug");
            builder.ins().brif(cond_i1, then_cl, &[], else_cl, &[]);
        }

        mir::Terminator::Unreachable => {
            builder
                .ins()
                .trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
        }

        // Exception handling terminators
        mir::Terminator::TrySetjmp {
            frame_local,
            try_body,
            handler_entry,
        } => {
            compile_try_setjmp(builder, frame_local, try_body, handler_entry, ctx)?;
        }

        mir::Terminator::Raise {
            exc_type,
            message,
            cause,
            suppress_context,
        } => {
            compile_raise(builder, *exc_type, message, cause, *suppress_context, ctx)?;
        }

        mir::Terminator::Reraise => {
            compile_reraise(builder, ctx)?;
        }

        mir::Terminator::RaiseCustom {
            class_id,
            message,
            instance,
        } => {
            compile_raise_custom(builder, *class_id, message, instance, ctx)?;
        }

        mir::Terminator::RaiseInstance { instance } => {
            compile_raise_instance(builder, instance, ctx)?;
        }
    }
    Ok(())
}
