//! Exception handling code generation
//!
//! This module handles code generation for exception-related instructions
//! and terminators including try/except, raise, and reraise.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, StackSlotData, StackSlotKind};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::{
    create_raw_string_data, declare_runtime_function, get_call_result, load_operand,
};

/// Compile ExcPushFrame instruction
/// Allocates exception frame on stack and pushes it to the handler stack
pub fn compile_exc_push_frame(
    builder: &mut FunctionBuilder,
    frame_local: &LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    // ExceptionFrame: prev (8) + jmp_buf (200) + gc_stack_top (8) = 216 bytes

    // Create stack slot for ExceptionFrame
    let frame_slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        216, // Size of ExceptionFrame
        3,   // 8-byte alignment
    ));

    // Get address of the frame on stack
    let frame_addr = builder.ins().stack_addr(cltypes::I64, frame_slot, 0);

    // Store the frame address in the local variable
    let var = *ctx
        .var_map
        .get(frame_local)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(var, frame_addr);

    // Call rt_exc_push_frame(frame_addr)
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64));

    let func_id = declare_runtime_function(ctx.module, "rt_exc_push_frame", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    builder.ins().call(func_ref, &[frame_addr]);
    Ok(())
}

/// Compile ExcPopFrame instruction
/// Pops the current exception frame from the handler stack
pub fn compile_exc_pop_frame(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    let func_id = declare_runtime_function(ctx.module, "rt_exc_pop_frame", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    builder.ins().call(func_ref, &[]);
    Ok(())
}

/// Compile ExcGetType instruction
/// Gets the type of the current exception
pub fn compile_exc_get_type(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.returns.push(AbiParam::new(cltypes::I32));

    let func_id = declare_runtime_function(ctx.module, "rt_exc_get_type", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    let call_inst = builder.ins().call(func_ref, &[]);
    let result = get_call_result(builder, call_inst);

    // Extend i32 to i64 for consistency with our local types
    let result_i64 = builder.ins().sextend(cltypes::I64, result);
    let var = *ctx
        .var_map
        .get(dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(var, result_i64);
    Ok(())
}

/// Compile ExcClear instruction
/// Clears the current exception state
pub fn compile_exc_clear(builder: &mut FunctionBuilder, ctx: &mut CodegenContext) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    let func_id = declare_runtime_function(ctx.module, "rt_exc_clear", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    builder.ins().call(func_ref, &[]);
    Ok(())
}

/// Compile ExcHasException instruction
/// Checks if there's a current active exception
pub fn compile_exc_has_exception(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.returns.push(AbiParam::new(cltypes::I8));

    let func_id = declare_runtime_function(ctx.module, "rt_exc_has_exception", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    let call_inst = builder.ins().call(func_ref, &[]);
    let result = get_call_result(builder, call_inst);

    let var = *ctx
        .var_map
        .get(dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(var, result);
    Ok(())
}

/// Compile TrySetjmp terminator
/// Sets up setjmp for exception handling, branches to try_body or handler
///
/// Calls `setjmp` directly from Cranelift-generated code rather than through
/// a Rust wrapper function. This is critical because `setjmp`/`longjmp` requires
/// that the function which called `setjmp` has not returned when `longjmp` fires.
/// A Rust wrapper (`rt_exc_setjmp`) would return immediately, making the later
/// `longjmp` undefined behavior — which manifests as SIGILL in debug builds
/// where the wrapper is not inlined.
pub fn compile_try_setjmp(
    builder: &mut FunctionBuilder,
    frame_local: &LocalId,
    try_body: &pyaot_utils::BlockId,
    handler_entry: &pyaot_utils::BlockId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Declare setjmp directly: extern "C" fn(*mut u8) -> i32
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // jmp_buf pointer
    sig.returns.push(AbiParam::new(cltypes::I32)); // 0 or non-zero

    let func_id = declare_runtime_function(ctx.module, "setjmp", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    // Get frame pointer from local variable
    let frame_ptr = builder.use_var(
        *ctx.var_map
            .get(frame_local)
            .expect("internal error: local not in var_map - codegen bug"),
    );

    // Compute jmp_buf address: frame_ptr + 8 (offset of jmp_buf in ExceptionFrame)
    // ExceptionFrame layout: prev (*mut ExceptionFrame, 8 bytes) | jmp_buf ([u8; 200]) | ...
    let jmp_buf_ptr = builder.ins().iadd_imm(frame_ptr, 8);

    // Call setjmp directly from Cranelift-generated code
    let call_inst = builder.ins().call(func_ref, &[jmp_buf_ptr]);
    let result = get_call_result(builder, call_inst);

    // Branch based on result: 0 = try_body, non-zero = handler_entry
    let zero = builder.ins().iconst(cltypes::I32, 0);
    let is_normal =
        builder
            .ins()
            .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, result, zero);

    let try_body_cl = *ctx
        .block_map
        .get(try_body)
        .expect("internal error: block not in block_map - codegen bug");
    let handler_cl = *ctx
        .block_map
        .get(handler_entry)
        .expect("internal error: block not in block_map - codegen bug");
    builder
        .ins()
        .brif(is_normal, try_body_cl, &[], handler_cl, &[]);
    Ok(())
}

/// Extract a message pointer and length from an operand (string constant or heap string).
/// Returns (ptr, len) as Cranelift values.
fn extract_message_operand(
    builder: &mut FunctionBuilder,
    operand: &Option<Operand>,
    ctx: &mut CodegenContext,
) -> Result<(cranelift_codegen::ir::Value, cranelift_codegen::ir::Value)> {
    if let Some(op) = operand {
        if let Operand::Constant(mir::Constant::Str(s)) = op {
            let str_content = ctx.interner.resolve(*s);
            let str_len = str_content.len();
            let data_id = create_raw_string_data(ctx.module, *s, ctx.interner);
            let gv = ctx.module.declare_data_in_func(data_id, builder.func);
            let ptr = builder.ins().global_value(cltypes::I64, gv);
            let len = builder.ins().iconst(cltypes::I64, str_len as i64);
            Ok((ptr, len))
        } else {
            let str_obj = load_operand(builder, op, ctx.var_map);

            let mut data_sig = ctx.module.make_signature();
            data_sig.call_conv = CallConv::SystemV;
            data_sig.params.push(AbiParam::new(cltypes::I64));
            data_sig.returns.push(AbiParam::new(cltypes::I64));

            let data_id = declare_runtime_function(ctx.module, "rt_str_data", &data_sig)?;
            let data_ref = ctx.module.declare_func_in_func(data_id, builder.func);
            let data_call = builder.ins().call(data_ref, &[str_obj]);
            let ptr = get_call_result(builder, data_call);

            let len_id = declare_runtime_function(ctx.module, "rt_str_len", &data_sig)?;
            let len_ref = ctx.module.declare_func_in_func(len_id, builder.func);
            let len_call = builder.ins().call(len_ref, &[str_obj]);
            let len = get_call_result(builder, len_call);

            Ok((ptr, len))
        }
    } else {
        let null = builder.ins().iconst(cltypes::I64, 0);
        let zero = builder.ins().iconst(cltypes::I64, 0);
        Ok((null, zero))
    }
}

/// Compile Raise terminator
/// Raises an exception with the given type and optional message, optionally with a cause
/// suppress_context: if true and no cause, call rt_exc_raise_from_none to suppress context display
pub fn compile_raise(
    builder: &mut FunctionBuilder,
    exc_type: u8,
    message: &Option<Operand>,
    cause: &Option<mir::RaiseCause>,
    suppress_context: bool,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let exc_type_val = builder.ins().iconst(cltypes::I8, exc_type as i64);
    let (msg_ptr, msg_len) = extract_message_operand(builder, message, ctx)?;

    if let Some(cause_info) = cause {
        // raise X from Y: call rt_exc_raise_from with both main and cause info
        let mut sig = ctx.module.make_signature();
        sig.call_conv = CallConv::SystemV;
        sig.params.push(AbiParam::new(cltypes::I8)); // exc_type
        sig.params.push(AbiParam::new(cltypes::I64)); // message ptr
        sig.params.push(AbiParam::new(cltypes::I64)); // message len
        sig.params.push(AbiParam::new(cltypes::I8)); // cause_type
        sig.params.push(AbiParam::new(cltypes::I64)); // cause_message ptr
        sig.params.push(AbiParam::new(cltypes::I64)); // cause_message len

        let func_id = declare_runtime_function(ctx.module, "rt_exc_raise_from", &sig)?;
        let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

        let cause_type_val = builder
            .ins()
            .iconst(cltypes::I8, cause_info.exc_type as i64);
        let (cause_msg_ptr, cause_msg_len) =
            extract_message_operand(builder, &cause_info.message, ctx)?;

        builder.ins().call(
            func_ref,
            &[
                exc_type_val,
                msg_ptr,
                msg_len,
                cause_type_val,
                cause_msg_ptr,
                cause_msg_len,
            ],
        );
    } else if suppress_context {
        // raise X from None: call rt_exc_raise_from_none to suppress context
        let mut sig = ctx.module.make_signature();
        sig.call_conv = CallConv::SystemV;
        sig.params.push(AbiParam::new(cltypes::I8)); // exc_type
        sig.params.push(AbiParam::new(cltypes::I64)); // message ptr
        sig.params.push(AbiParam::new(cltypes::I64)); // message len

        let func_id = declare_runtime_function(ctx.module, "rt_exc_raise_from_none", &sig)?;
        let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

        builder
            .ins()
            .call(func_ref, &[exc_type_val, msg_ptr, msg_len]);
    } else {
        // Plain raise X: call rt_exc_raise
        let mut sig = ctx.module.make_signature();
        sig.call_conv = CallConv::SystemV;
        sig.params.push(AbiParam::new(cltypes::I8)); // exc_type
        sig.params.push(AbiParam::new(cltypes::I64)); // message ptr
        sig.params.push(AbiParam::new(cltypes::I64)); // message len

        let func_id = declare_runtime_function(ctx.module, "rt_exc_raise", &sig)?;
        let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

        builder
            .ins()
            .call(func_ref, &[exc_type_val, msg_ptr, msg_len]);
    }

    // rt_exc_raise / rt_exc_raise_from / rt_exc_raise_from_none never return, so add an unreachable trap
    builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::unwrap_user(2));
    Ok(())
}

/// Compile Reraise terminator
/// Re-raises the current exception
pub fn compile_reraise(builder: &mut FunctionBuilder, ctx: &mut CodegenContext) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    let func_id = declare_runtime_function(ctx.module, "rt_exc_reraise", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    builder.ins().call(func_ref, &[]);
    // rt_exc_reraise never returns, so add an unreachable trap
    builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::unwrap_user(3));
    Ok(())
}

/// Compile ExcGetCurrent instruction
/// Gets the current exception as a string object (for `except E as e:`)
pub fn compile_exc_get_current(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

    let func_id = declare_runtime_function(ctx.module, "rt_exc_get_current", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    let call_inst = builder.ins().call(func_ref, &[]);
    let result = get_call_result(builder, call_inst);

    let var = *ctx
        .var_map
        .get(dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(var, result);

    // Update GC root if needed
    crate::gc::update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
    Ok(())
}

/// Compile ExcCheckType instruction
/// Checks if current exception matches the given type tag
pub fn compile_exc_check_type(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    type_tag: u8,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I8)); // type_tag
    sig.returns.push(AbiParam::new(cltypes::I8)); // bool result

    let func_id = declare_runtime_function(ctx.module, "rt_exc_isinstance", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let type_tag_val = builder.ins().iconst(cltypes::I8, type_tag as i64);
    let call_inst = builder.ins().call(func_ref, &[type_tag_val]);
    let result = get_call_result(builder, call_inst);

    let var = *ctx
        .var_map
        .get(dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(var, result);
    Ok(())
}

/// Compile ExcCheckClass instruction
/// Checks if current exception is an instance of the given class (with inheritance support)
/// Uses rt_exc_isinstance_class which walks the inheritance chain
pub fn compile_exc_check_class(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    class_id: u8,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I8)); // class_id
    sig.returns.push(AbiParam::new(cltypes::I8)); // bool result

    let func_id = declare_runtime_function(ctx.module, "rt_exc_isinstance_class", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let class_id_val = builder.ins().iconst(cltypes::I8, class_id as i64);
    let call_inst = builder.ins().call(func_ref, &[class_id_val]);
    let result = get_call_result(builder, call_inst);

    let var = *ctx
        .var_map
        .get(dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(var, result);
    Ok(())
}

/// Compile RaiseCustom terminator
/// Raises a custom exception with the given class_id and optional message
pub fn compile_raise_custom(
    builder: &mut FunctionBuilder,
    class_id: u8,
    message: &Option<Operand>,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let class_id_val = builder.ins().iconst(cltypes::I8, class_id as i64);
    let (msg_ptr, msg_len) = extract_message_operand(builder, message, ctx)?;

    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I8)); // class_id
    sig.params.push(AbiParam::new(cltypes::I64)); // message ptr
    sig.params.push(AbiParam::new(cltypes::I64)); // message len

    let func_id = declare_runtime_function(ctx.module, "rt_exc_raise_custom", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    builder
        .ins()
        .call(func_ref, &[class_id_val, msg_ptr, msg_len]);

    // rt_exc_raise_custom never returns, so add an unreachable trap
    builder
        .ins()
        .trap(cranelift_codegen::ir::TrapCode::unwrap_user(4));
    Ok(())
}

/// Compile ExcStartHandling instruction
/// Marks the start of exception handling, preserving the exception for __context__
pub fn compile_exc_start_handling(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    let func_id = declare_runtime_function(ctx.module, "rt_exc_start_handling", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    builder.ins().call(func_ref, &[]);
    Ok(())
}

/// Compile ExcEndHandling instruction
/// Marks the end of exception handling, clearing the saved exception
pub fn compile_exc_end_handling(
    builder: &mut FunctionBuilder,
    ctx: &mut CodegenContext,
) -> Result<()> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    let func_id = declare_runtime_function(ctx.module, "rt_exc_end_handling", &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
    builder.ins().call(func_ref, &[]);
    Ok(())
}
