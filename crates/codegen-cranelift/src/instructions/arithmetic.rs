//! Arithmetic and comparison operations (BinOp, UnOp)
//!
//! Handles BinOp compilation including float operations, integer runtime calls,
//! comparison operations, and boolean/bitwise operations.
//! Also handles UnOp (Neg, Not, Invert).

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};

use crate::context::CodegenContext;
use crate::utils::{
    create_traceback_string_data, declare_runtime_function, get_call_result, is_float_operand,
    load_operand, load_operand_as, promote_to_float,
};

/// Compile a unary operation (Neg, Not, Invert)
pub(crate) fn compile_unop(
    builder: &mut FunctionBuilder,
    dest: &pyaot_utils::LocalId,
    op: &mir::UnOp,
    operand: &Operand,
    ctx: &mut CodegenContext,
) {
    let operand_val = load_operand(builder, operand, ctx.symbols.var_map);
    let is_float = is_float_operand(operand, ctx.symbols.locals);
    let result = match op {
        mir::UnOp::Neg => {
            if is_float {
                builder.ins().fneg(operand_val)
            } else {
                builder.ins().ineg(operand_val)
            }
        }
        mir::UnOp::Not => {
            let val_type = builder.func.dfg.value_type(operand_val);
            if val_type == cltypes::I8 {
                let one = builder.ins().iconst(cltypes::I8, 1);
                builder.ins().isub(one, operand_val)
            } else {
                let zero = builder.ins().iconst(cltypes::I64, 0);
                builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    operand_val,
                    zero,
                )
            }
        }
        mir::UnOp::Invert => builder.ins().bnot(operand_val),
    };
    ctx.store_result(builder, dest, result);
}

/// Promote integer operands to matching types for comparison.
/// When comparing i8 (bool) with i64 (int/any/ptr), promote both to i64.
fn promote_int_operands(
    builder: &mut FunctionBuilder,
    left_val: cranelift_codegen::ir::Value,
    right_val: cranelift_codegen::ir::Value,
) -> (cranelift_codegen::ir::Value, cranelift_codegen::ir::Value) {
    // Use actual Cranelift IR value types instead of MIR types, because the values
    // may have already been promoted by load_operand_as before reaching this point.
    let left_ty = builder.func.dfg.value_type(left_val);
    let right_ty = builder.func.dfg.value_type(right_val);

    if left_ty == right_ty {
        // Same type, no promotion needed
        (left_val, right_val)
    } else if left_ty == cltypes::I8 && right_ty == cltypes::I64 {
        // Promote left (i8) to i64
        (builder.ins().uextend(cltypes::I64, left_val), right_val)
    } else if left_ty == cltypes::I64 && right_ty == cltypes::I8 {
        // Promote right (i8) to i64
        (left_val, builder.ins().uextend(cltypes::I64, right_val))
    } else {
        // Other cases - return as-is
        (left_val, right_val)
    }
}

/// Compile a binary operation
pub(crate) fn compile_binop(
    builder: &mut FunctionBuilder,
    dest: &pyaot_utils::LocalId,
    op: &mir::BinOp,
    left: &Operand,
    right: &Operand,
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Check if this is a float operation (either operand is float)
    let left_is_float = is_float_operand(left, ctx.symbols.locals);
    let right_is_float = is_float_operand(right, ctx.symbols.locals);
    let is_float = left_is_float || right_is_float;

    // Determine if this is a boolean operation that should keep i8 operands
    let is_bool_op = matches!(op, mir::BinOp::And | mir::BinOp::Or);

    // Load operands with appropriate type coercion:
    // - Float operations: load as-is (will be promoted to float later)
    // - Boolean operations (And, Or): keep as i8
    // - Integer operations: coerce Bool (i8) to Int (i64) for Python semantics
    let (left_val, right_val) = if is_float || is_bool_op {
        (
            load_operand(builder, left, ctx.symbols.var_map),
            load_operand(builder, right, ctx.symbols.var_map),
        )
    } else {
        // For integer operations, ensure both operands are i64
        // This coerces Bool (i8) to Int (i64) as needed
        (
            load_operand_as(builder, left, ctx.symbols.var_map, cltypes::I64),
            load_operand_as(builder, right, ctx.symbols.var_map, cltypes::I64),
        )
    };

    let result = if is_float {
        // Promote int operands to float for mixed-type operations
        let left_float = promote_to_float(builder, left_val, left, ctx.symbols.locals);
        let right_float = promote_to_float(builder, right_val, right, ctx.symbols.locals);

        // Float operations
        match op {
            mir::BinOp::Add => builder.ins().fadd(left_float, right_float),
            mir::BinOp::Sub => builder.ins().fsub(left_float, right_float),
            mir::BinOp::Mul => builder.ins().fmul(left_float, right_float),
            mir::BinOp::Div => builder.ins().fdiv(left_float, right_float),
            mir::BinOp::FloorDiv => {
                // Floor division for floats: floor(a / b)
                let div_result = builder.ins().fdiv(left_float, right_float);
                builder.ins().floor(div_result)
            }
            mir::BinOp::Mod => {
                // Float modulo: a - floor(a/b) * b
                let div = builder.ins().fdiv(left_float, right_float);
                let floored = builder.ins().floor(div);
                let prod = builder.ins().fmul(floored, right_float);
                builder.ins().fsub(left_float, prod)
            }
            mir::BinOp::Pow => {
                // Call rt_pow_float(base: f64, exp: f64) -> f64
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::F64));
                sig.params.push(AbiParam::new(cltypes::F64));
                sig.returns.push(AbiParam::new(cltypes::F64));

                let func_id = declare_runtime_function(ctx.module, "rt_pow_float", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_float, right_float]);
                get_call_result(builder, call_inst)
            }
            // Float comparison operations - fcmp returns i1, extend to dest type
            mir::BinOp::Eq => {
                let cmp = builder.ins().fcmp(
                    cranelift_codegen::ir::condcodes::FloatCC::Equal,
                    left_float,
                    right_float,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::NotEq => {
                let cmp = builder.ins().fcmp(
                    cranelift_codegen::ir::condcodes::FloatCC::NotEqual,
                    left_float,
                    right_float,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::Lt => {
                let cmp = builder.ins().fcmp(
                    cranelift_codegen::ir::condcodes::FloatCC::LessThan,
                    left_float,
                    right_float,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::LtE => {
                let cmp = builder.ins().fcmp(
                    cranelift_codegen::ir::condcodes::FloatCC::LessThanOrEqual,
                    left_float,
                    right_float,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::Gt => {
                let cmp = builder.ins().fcmp(
                    cranelift_codegen::ir::condcodes::FloatCC::GreaterThan,
                    left_float,
                    right_float,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::GtE => {
                let cmp = builder.ins().fcmp(
                    cranelift_codegen::ir::condcodes::FloatCC::GreaterThanOrEqual,
                    left_float,
                    right_float,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            // Boolean operations don't apply to floats - use integer branch
            mir::BinOp::And | mir::BinOp::Or => {
                // This should not happen - And/Or are boolean operations
                // Fall through to integer operations which handle bool (i8)
                return Err(pyaot_diagnostics::CompilerError::codegen_error(
                    "Boolean operations (and/or) cannot be applied to float operands".to_string(),
                    None,
                ));
            }
            // Bitwise operations are only valid for integers
            mir::BinOp::BitAnd
            | mir::BinOp::BitOr
            | mir::BinOp::BitXor
            | mir::BinOp::LShift
            | mir::BinOp::RShift => {
                // Bitwise operations on floats are not supported
                // This should be caught by type checking
                return Err(pyaot_diagnostics::CompilerError::codegen_error(
                    "Bitwise operations cannot be applied to float operands".to_string(),
                    None,
                ));
            }
        }
    } else {
        // Integer operations - inlined with overflow/division-by-zero checks
        match op {
            mir::BinOp::Add => inline_int_add(builder, ctx, left_val, right_val)?,
            mir::BinOp::Sub => inline_int_sub(builder, ctx, left_val, right_val)?,
            mir::BinOp::Mul => inline_int_mul(builder, ctx, left_val, right_val)?,
            mir::BinOp::Div => inline_int_truediv(builder, ctx, left_val, right_val)?,
            mir::BinOp::FloorDiv => inline_int_floordiv(builder, ctx, left_val, right_val)?,
            mir::BinOp::Mod => inline_int_mod(builder, ctx, left_val, right_val)?,
            mir::BinOp::Pow => call_int_binop_rt(
                builder,
                ctx,
                "rt_pow_int",
                cltypes::I64,
                left_val,
                right_val,
            )?,
            // Integer comparison operations - icmp returns i1, extend to dest type
            // First, promote operands to matching types if needed (i8 vs i64)
            mir::BinOp::Eq => {
                let (l, r) = promote_int_operands(builder, left_val, right_val);
                let cmp = builder
                    .ins()
                    .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, l, r);
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::NotEq => {
                let (l, r) = promote_int_operands(builder, left_val, right_val);
                let cmp =
                    builder
                        .ins()
                        .icmp(cranelift_codegen::ir::condcodes::IntCC::NotEqual, l, r);
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::Lt => {
                let (l, r) = promote_int_operands(builder, left_val, right_val);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                    l,
                    r,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::LtE => {
                let (l, r) = promote_int_operands(builder, left_val, right_val);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThanOrEqual,
                    l,
                    r,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::Gt => {
                let (l, r) = promote_int_operands(builder, left_val, right_val);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThan,
                    l,
                    r,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::GtE => {
                let (l, r) = promote_int_operands(builder, left_val, right_val);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                    l,
                    r,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            // Boolean operations (operands are i8 bools: 0 or 1)
            mir::BinOp::And => builder.ins().band(left_val, right_val),
            mir::BinOp::Or => builder.ins().bor(left_val, right_val),
            // Bitwise operations (integer only)
            mir::BinOp::BitAnd => builder.ins().band(left_val, right_val),
            mir::BinOp::BitOr => builder.ins().bor(left_val, right_val),
            mir::BinOp::BitXor => builder.ins().bxor(left_val, right_val),
            mir::BinOp::LShift => builder.ins().ishl(left_val, right_val),
            mir::BinOp::RShift => builder.ins().sshr(left_val, right_val),
        }
    };

    ctx.store_result(builder, dest, result);
    Ok(())
}

/// Extend a comparison result to the target type based on destination variable type.
/// icmp/fcmp return i8 (0 or 1) in Cranelift. If the destination expects i64
/// (e.g., for Int or Any-typed variables), extend to match.
pub(crate) fn extend_comparison_result(
    builder: &mut FunctionBuilder,
    cmp_result: Value,
    dest: &pyaot_utils::LocalId,
    ctx: &CodegenContext,
) -> Value {
    let dest_cl_ty = ctx
        .symbols
        .locals
        .get(dest)
        .map(|l| crate::utils::type_to_cranelift(&l.ty))
        .unwrap_or(cltypes::I8);
    let result_ty = builder.func.dfg.value_type(cmp_result);
    if result_ty == dest_cl_ty {
        cmp_result
    } else if dest_cl_ty == cltypes::I64 {
        builder.ins().uextend(cltypes::I64, cmp_result)
    } else {
        cmp_result
    }
}

/// Emit `rt_exc_raise(exc_tag, msg)` + `trap` in the current (error) block.
fn emit_raise_static(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    exc_tag: i64,
    msg: &str,
) -> Result<()> {
    let data_id = create_traceback_string_data(ctx.module, msg);
    let gv = ctx.module.declare_data_in_func(data_id, builder.func);
    let msg_ptr = builder.ins().global_value(cltypes::I64, gv);
    let msg_len = builder.ins().iconst(cltypes::I64, msg.len() as i64);
    let exc_type_val = builder.ins().iconst(cltypes::I8, exc_tag);

    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I8));
    sig.params.push(AbiParam::new(cltypes::I64));
    sig.params.push(AbiParam::new(cltypes::I64));

    let func_id = declare_runtime_function(ctx.module, "rt_exc_raise", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    builder
        .ins()
        .call(func_ref, &[exc_type_val, msg_ptr, msg_len]);
    builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::unwrap_user(2));
    Ok(())
}

fn inline_int_add(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    a: Value,
    b: Value,
) -> Result<Value> {
    let (result, of) = builder.ins().sadd_overflow(a, b);
    let overflow_block = builder.create_block();
    let merge_block = builder.create_block();
    builder
        .ins()
        .brif(of, overflow_block, &[], merge_block, &[]);

    builder.switch_to_block(overflow_block);
    builder.set_cold_block(overflow_block);
    emit_raise_static(builder, ctx, 12, "integer overflow")?;

    builder.switch_to_block(merge_block);
    Ok(result)
}

fn inline_int_sub(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    a: Value,
    b: Value,
) -> Result<Value> {
    let (result, of) = builder.ins().ssub_overflow(a, b);
    let overflow_block = builder.create_block();
    let merge_block = builder.create_block();
    builder
        .ins()
        .brif(of, overflow_block, &[], merge_block, &[]);

    builder.switch_to_block(overflow_block);
    builder.set_cold_block(overflow_block);
    emit_raise_static(builder, ctx, 12, "integer overflow")?;

    builder.switch_to_block(merge_block);
    Ok(result)
}

fn inline_int_mul(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    a: Value,
    b: Value,
) -> Result<Value> {
    let (result, of) = builder.ins().smul_overflow(a, b);
    let overflow_block = builder.create_block();
    let merge_block = builder.create_block();
    builder
        .ins()
        .brif(of, overflow_block, &[], merge_block, &[]);

    builder.switch_to_block(overflow_block);
    builder.set_cold_block(overflow_block);
    emit_raise_static(builder, ctx, 12, "integer overflow")?;

    builder.switch_to_block(merge_block);
    Ok(result)
}

/// Inline Python true division (a / b → f64) with zero-division check.
fn inline_int_truediv(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    a: Value,
    b: Value,
) -> Result<Value> {
    let zero = builder.ins().iconst(cltypes::I64, 0);
    let b_is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
    let zero_block = builder.create_block();
    let merge_block = builder.create_block();
    builder
        .ins()
        .brif(b_is_zero, zero_block, &[], merge_block, &[]);

    builder.switch_to_block(zero_block);
    builder.set_cold_block(zero_block);
    emit_raise_static(builder, ctx, 11, "division by zero")?;

    builder.switch_to_block(merge_block);
    let a_f = builder.ins().fcvt_from_sint(cltypes::F64, a);
    let b_f = builder.ins().fcvt_from_sint(cltypes::F64, b);
    Ok(builder.ins().fdiv(a_f, b_f))
}

/// Inline Python floor division (a // b) with zero-division and overflow checks.
/// Python semantics: rounds toward negative infinity.
fn inline_int_floordiv(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    a: Value,
    b: Value,
) -> Result<Value> {
    let zero = builder.ins().iconst(cltypes::I64, 0);
    let min_int = builder.ins().iconst(cltypes::I64, i64::MIN);
    let neg_one = builder.ins().iconst(cltypes::I64, -1);

    let b_is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
    let zero_block = builder.create_block();
    let after_zero = builder.create_block();
    builder
        .ins()
        .brif(b_is_zero, zero_block, &[], after_zero, &[]);

    builder.switch_to_block(zero_block);
    builder.set_cold_block(zero_block);
    emit_raise_static(builder, ctx, 11, "integer division or modulo by zero")?;

    builder.switch_to_block(after_zero);

    let a_is_min = builder.ins().icmp(IntCC::Equal, a, min_int);
    let b_is_neg_one = builder.ins().icmp(IntCC::Equal, b, neg_one);
    let overflow_cond = builder.ins().band(a_is_min, b_is_neg_one);
    let overflow_block = builder.create_block();
    let compute_block = builder.create_block();
    builder
        .ins()
        .brif(overflow_cond, overflow_block, &[], compute_block, &[]);

    builder.switch_to_block(overflow_block);
    builder.set_cold_block(overflow_block);
    emit_raise_static(builder, ctx, 12, "integer overflow")?;

    builder.switch_to_block(compute_block);

    let q = builder.ins().sdiv(a, b);
    let r = builder.ins().srem(a, b);
    let r_ne_zero = builder.ins().icmp(IntCC::NotEqual, r, zero);
    let rxb = builder.ins().bxor(r, b);
    let rxb_neg = builder.ins().icmp(IntCC::SignedLessThan, rxb, zero);
    let adjust = builder.ins().band(r_ne_zero, rxb_neg);
    let q_minus_1 = builder.ins().iadd_imm(q, -1);
    Ok(builder.ins().select(adjust, q_minus_1, q))
}

/// Inline Python modulo (a % b) with zero-division and overflow checks.
/// Python semantics: result has same sign as divisor.
fn inline_int_mod(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    a: Value,
    b: Value,
) -> Result<Value> {
    let zero = builder.ins().iconst(cltypes::I64, 0);
    let min_int = builder.ins().iconst(cltypes::I64, i64::MIN);
    let neg_one = builder.ins().iconst(cltypes::I64, -1);

    let b_is_zero = builder.ins().icmp(IntCC::Equal, b, zero);
    let zero_block = builder.create_block();
    let after_zero = builder.create_block();
    builder
        .ins()
        .brif(b_is_zero, zero_block, &[], after_zero, &[]);

    builder.switch_to_block(zero_block);
    builder.set_cold_block(zero_block);
    emit_raise_static(builder, ctx, 11, "integer division or modulo by zero")?;

    builder.switch_to_block(after_zero);

    let a_is_min = builder.ins().icmp(IntCC::Equal, a, min_int);
    let b_is_neg_one = builder.ins().icmp(IntCC::Equal, b, neg_one);
    let overflow_cond = builder.ins().band(a_is_min, b_is_neg_one);
    let overflow_block = builder.create_block();
    let compute_block = builder.create_block();
    builder
        .ins()
        .brif(overflow_cond, overflow_block, &[], compute_block, &[]);

    builder.switch_to_block(overflow_block);
    builder.set_cold_block(overflow_block);
    emit_raise_static(builder, ctx, 12, "integer overflow")?;

    builder.switch_to_block(compute_block);

    let r = builder.ins().srem(a, b);
    let r_ne_zero = builder.ins().icmp(IntCC::NotEqual, r, zero);
    let rxb = builder.ins().bxor(r, b);
    let rxb_neg = builder.ins().icmp(IntCC::SignedLessThan, rxb, zero);
    let adjust = builder.ins().band(r_ne_zero, rxb_neg);
    let r_plus_b = builder.ins().iadd(r, b);
    Ok(builder.ins().select(adjust, r_plus_b, r))
}

/// Call a binary integer runtime function: rt_name(a: i64, b: i64) -> ret_type.
/// Used for Pow which is too complex to inline.
fn call_int_binop_rt(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
    func_name: &str,
    ret_type: cltypes::Type,
    left: Value,
    right: Value,
) -> pyaot_diagnostics::Result<Value> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64));
    sig.params.push(AbiParam::new(cltypes::I64));
    sig.returns.push(AbiParam::new(ret_type));

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    let call_inst = builder.ins().call(func_ref, &[left, right]);
    Ok(get_call_result(builder, call_inst))
}
