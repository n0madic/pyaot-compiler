//! Terminator compilation
//!
//! This module handles code generation for MIR terminators including
//! Return, Goto, Branch, Unreachable, and exception-related terminators.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_core_defs::tag::{BOOL_SHIFT, BOOL_TAG, INT_SHIFT, INT_TAG};
use pyaot_diagnostics::Result;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::context::CodegenContext;
use crate::exceptions::{
    compile_raise, compile_raise_custom, compile_raise_instance, compile_reraise,
    compile_try_setjmp,
};
use crate::instructions::calls::box_primitive;
use crate::utils::{declare_runtime_function, get_call_result, load_operand, type_to_cranelift};

/// Inline `Value::from_int(x)` — `(x << 3) | INT_TAG`. Same shape as
/// `instructions::tag::compile_value_from_int` but reusable from
/// terminator codegen without going through a MIR instruction.
fn box_int_inline(
    builder: &mut FunctionBuilder,
    value: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let shifted = builder.ins().ishl_imm(value, INT_SHIFT as i64);
    builder.ins().bor_imm(shifted, INT_TAG as i64)
}

/// Inline `Value::from_bool(b)` — zero-extend i8 to i64, `(b << 3) | BOOL_TAG`.
fn box_bool_inline(
    builder: &mut FunctionBuilder,
    value: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let extended = builder.ins().uextend(cltypes::I64, value);
    let shifted = builder.ins().ishl_imm(extended, BOOL_SHIFT as i64);
    builder.ins().bor_imm(shifted, BOOL_TAG as i64)
}

/// Compile a MIR terminator to Cranelift IR
pub fn compile_terminator(
    builder: &mut FunctionBuilder,
    term: &mir::Terminator,
    ctx: &mut CodegenContext,
) -> Result<()> {
    match term {
        mir::Terminator::Return(val) => {
            // Pop traceback frame before returning (always — every function pushed one)
            if let Some(stack_pop_id) = ctx.gc.stack_pop_id {
                let stack_pop_ref = ctx.module.declare_func_in_func(stack_pop_id, builder.func);
                builder.ins().call(stack_pop_ref, &[]);
            }

            // Call gc_pop before returning if we have GC roots
            if ctx.gc.frame_data.is_some() {
                if let Some(gc_pop_id) = ctx.gc.gc_pop_id {
                    let gc_pop_ref = ctx.module.declare_func_in_func(gc_pop_id, builder.func);
                    builder.ins().call(gc_pop_ref, &[]);
                }
            }

            // Only skip returning a value if the function's return type is exactly None.
            // For Union types containing None (e.g., Point | None), we still need to return a value.
            if matches!(ctx.debug.return_type, Type::None) {
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
                    let ret_val = load_operand(builder, operand, ctx.symbols.var_map);
                    // When the function's declared return type is a tagged-Value
                    // slot (`Union[…]` / `Any` / `HeapAny`), box primitive operands
                    // so callers see well-formed `Value` bits — immediate Int/Bool/
                    // None tags or 8-byte-aligned heap pointers — instead of raw
                    // scalars that downstream consumers (`rt_print_obj`, Phi merges
                    // typed Union, etc.) would mis-decode as invalid pointers and
                    // SEGV when dereferenced. Mirrors the merge-block boxing in
                    // `coerce_phi_source_for_dest` below.
                    //
                    // After §F.7c BigBang: generator `*$resume` functions are
                    // treated like any other function returning a tagged-Value
                    // slot. The for-loop / next() consumer applies the same
                    // UnwrapValue unwrapping as for List/Dict/Tuple/Set
                    // iterators, so yields must arrive as tagged Value bits.
                    let is_generator_resume = ctx.debug.function_name.ends_with("$resume");
                    let coerced_val = if is_generator_resume
                        || matches!(
                            ctx.debug.return_type,
                            Type::Union(_) | Type::Any | Type::HeapAny
                        ) {
                        let source_ty = operand_semantic_type(operand, ctx);
                        match source_ty {
                            // §F.7d.2: Int/Bool boxing happens inline as
                            // tagged `Value` bits — no runtime call.
                            Type::Int => box_int_inline(builder, ret_val),
                            Type::Bool => box_bool_inline(builder, ret_val),
                            Type::Float => box_primitive(
                                builder,
                                ctx.module,
                                "rt_box_float",
                                cltypes::F64,
                                ret_val,
                            )?,
                            Type::None => box_none(builder, ctx)?,
                            // Already heap-typed (Str/List/Dict/Tuple/…), Any, or
                            // HeapAny — bits are already a valid tagged Value or an
                            // 8-byte-aligned heap pointer; pass through.
                            _ => ret_val,
                        }
                    } else {
                        // Same-shape return: keep the existing Cranelift-level
                        // coercion (i8→i64 uextend for Bool stored as i8, f64→i64
                        // bitcast for resume-function returns whose dfg type is
                        // F64 but whose ABI is I64). Also promote Int/Bool → Float
                        // for numeric-tower-promoted return types: when a function
                        // returns either `1.5` or `0`, type inference unifies the
                        // return type to `Float` (`int ⊂ float`), but the Int
                        // branch's Return operand is still raw I64 — without the
                        // (I64|I8, F64) arms below, Cranelift's verifier rejects
                        // the function ("result has type i64, must match function
                        // signature of f64").
                        let expected_type = type_to_cranelift(ctx.debug.return_type);
                        let val_type = builder.func.dfg.value_type(ret_val);
                        if val_type != expected_type {
                            match (val_type, expected_type) {
                                (cltypes::I8, cltypes::I64) => {
                                    builder.ins().uextend(cltypes::I64, ret_val)
                                }
                                (cltypes::F64, cltypes::I64) => builder.ins().bitcast(
                                    cltypes::I64,
                                    cranelift_codegen::ir::MemFlags::new(),
                                    ret_val,
                                ),
                                // Int → Float: signed-int-to-float conversion.
                                (cltypes::I64, cltypes::F64) => {
                                    builder.ins().fcvt_from_sint(cltypes::F64, ret_val)
                                }
                                // Bool (i8) → Float: extend to i64 first, then
                                // signed-int-to-float (False→0.0, True→1.0).
                                (cltypes::I8, cltypes::F64) => {
                                    let extended = builder.ins().uextend(cltypes::I64, ret_val);
                                    builder.ins().fcvt_from_sint(cltypes::F64, extended)
                                }
                                _ => ret_val,
                            }
                        } else {
                            ret_val
                        }
                    };
                    builder.ins().return_(&[coerced_val]);
                }
            } else {
                builder.ins().return_(&[]);
            }
        }

        mir::Terminator::Goto(target) => {
            let cl_block = *ctx
                .symbols
                .block_map
                .get(target)
                .expect("internal error: block not in block_map - codegen bug");
            let args = phi_branch_args(builder, ctx, target)?;
            builder.ins().jump(cl_block, &args);
        }

        mir::Terminator::Branch {
            cond,
            then_block,
            else_block,
        } => {
            let cond_val = load_operand(builder, cond, ctx.symbols.var_map);
            // Convert i8 bool to i1 for brif instruction
            let zero = builder.ins().iconst(cltypes::I8, 0);
            let cond_i1 = builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                cond_val,
                zero,
            );
            let then_cl = *ctx
                .symbols
                .block_map
                .get(then_block)
                .expect("internal error: block not in block_map - codegen bug");
            let else_cl = *ctx
                .symbols
                .block_map
                .get(else_block)
                .expect("internal error: block not in block_map - codegen bug");
            let then_args = phi_branch_args(builder, ctx, then_block)?;
            let else_args = phi_branch_args(builder, ctx, else_block)?;
            builder
                .ins()
                .brif(cond_i1, then_cl, &then_args, else_cl, &else_args);
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

/// Collect the SSA φ-source values a branch from `ctx.symbols.current_block`
/// to `target` must pass as block-call args. For each leading Phi in
/// `target`, find the source operand whose predecessor BlockId equals the
/// current block and load its value. Ordering matches the block-param
/// declaration order set up in `function.rs`.
///
/// Returns an empty `Vec` when `target` has no leading Phi instructions —
/// blocks with no phi joins still dispatch through here.
fn phi_branch_args(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    target: &pyaot_utils::BlockId,
) -> Result<Vec<cranelift_codegen::ir::BlockArg>> {
    let Some(target_block) = ctx.symbols.mir_blocks.get(target) else {
        return Ok(Vec::new());
    };
    let pred = ctx.symbols.current_block;
    let mut args = Vec::new();
    for inst in &target_block.instructions {
        let mir::InstructionKind::Phi { dest, sources } = &inst.kind else {
            break;
        };
        let source_op = sources
            .iter()
            .find(|(bb, _)| *bb == pred)
            .map(|(_, op)| op)
            .expect("phi has no source for predecessor block — arity violation");
        let value = coerce_phi_source_for_dest(builder, ctx, source_op, dest)?;
        args.push(cranelift_codegen::ir::BlockArg::Value(value));
    }
    Ok(args)
}

fn coerce_phi_source_for_dest(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    source_op: &mir::Operand,
    dest: &pyaot_utils::LocalId,
) -> Result<cranelift_codegen::ir::Value> {
    let value = load_operand(builder, source_op, ctx.symbols.var_map);
    let Some(dest_ty) = ctx.symbols.locals.get(dest).map(|local| &local.ty) else {
        return Ok(value);
    };

    if !matches!(dest_ty, Type::Union(_) | Type::Any | Type::HeapAny) {
        return Ok(value);
    }

    let source_ty = operand_semantic_type(source_op, ctx);
    match source_ty {
        // §F.7d.2: Int/Bool inlined as tagged Value bits.
        Type::Int => Ok(box_int_inline(builder, value)),
        Type::Bool => Ok(box_bool_inline(builder, value)),
        Type::Float => box_primitive(builder, ctx.module, "rt_box_float", cltypes::F64, value),
        Type::None => box_none(builder, ctx),
        _ => Ok(value),
    }
}

fn operand_semantic_type(op: &mir::Operand, ctx: &CodegenContext) -> Type {
    match op {
        mir::Operand::Local(id) => ctx
            .symbols
            .locals
            .get(id)
            .map(|local| local.ty.clone())
            .unwrap_or(Type::Any),
        mir::Operand::Constant(c) => match c {
            mir::Constant::Int(_) => Type::Int,
            mir::Constant::Float(_) => Type::Float,
            mir::Constant::Bool(_) => Type::Bool,
            mir::Constant::Str(_) => Type::Str,
            mir::Constant::Bytes(_) => Type::Bytes,
            mir::Constant::None => Type::None,
        },
    }
}

fn box_none(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
) -> Result<cranelift_codegen::ir::Value> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.returns
        .push(cranelift_codegen::ir::AbiParam::new(cltypes::I64));
    let func_id = declare_runtime_function(ctx.module, "rt_box_none", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    let call_inst = builder.ins().call(func_ref, &[]);
    Ok(get_call_result(builder, call_inst))
}
