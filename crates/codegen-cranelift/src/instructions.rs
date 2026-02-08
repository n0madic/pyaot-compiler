//! Core instruction compilation
//!
//! This module handles code generation for basic MIR instructions including
//! Const, Copy, BinOp, UnOp, and CallDirect.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_types::Type;

use crate::context::CodegenContext;
use crate::exceptions::{
    compile_exc_check_class, compile_exc_check_type, compile_exc_clear, compile_exc_end_handling,
    compile_exc_get_current, compile_exc_get_type, compile_exc_has_exception,
    compile_exc_pop_frame, compile_exc_push_frame, compile_exc_start_handling,
};
use crate::gc::update_gc_root_if_needed;
use crate::runtime_calls::compile_runtime_call;
use crate::utils::{
    declare_runtime_function, get_call_result, is_float_operand, load_operand, load_operand_as,
    promote_to_float,
};

/// Compile a single MIR instruction to Cranelift IR
pub fn compile_instruction(
    builder: &mut FunctionBuilder,
    inst: &mir::Instruction,
    ctx: &mut CodegenContext,
) -> Result<()> {
    match &inst.kind {
        mir::InstructionKind::Const { dest, value } => {
            let val = match value {
                mir::Constant::Int(i) => builder.ins().iconst(cltypes::I64, *i),
                mir::Constant::Float(f) => builder.ins().f64const(*f),
                mir::Constant::Bool(b) => builder.ins().iconst(cltypes::I8, *b as i64),
                mir::Constant::None => builder.ins().iconst(cltypes::I8, 0),
                _ => return Ok(()), // Skip for now
            };
            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, val);
            // Update GC root if needed
            update_gc_root_if_needed(builder, dest, val, ctx.gc_frame_data);
        }

        mir::InstructionKind::BinOp {
            dest,
            op,
            left,
            right,
        } => {
            compile_binop(builder, dest, op, left, right, ctx)?;
        }

        mir::InstructionKind::Copy { dest, src } => {
            let src_val = load_operand(builder, src, ctx.var_map);

            // Get source and destination types for potential conversion
            let src_ty = match src {
                Operand::Local(local_id) => ctx.locals.get(local_id).map(|l| &l.ty),
                Operand::Constant(c) => Some(match c {
                    mir::Constant::Int(_) => &Type::Int,
                    mir::Constant::Float(_) => &Type::Float,
                    mir::Constant::Bool(_) => &Type::Bool,
                    mir::Constant::None => &Type::None,
                    _ => &Type::Int, // Default for other constants
                }),
            };
            let dest_ty = ctx.locals.get(dest).map(|l| &l.ty);

            // Convert between types if needed
            use crate::utils::type_to_cranelift;
            let src_cl_ty = src_ty.map(type_to_cranelift).unwrap_or(cltypes::I64);
            let dest_cl_ty = dest_ty.map(type_to_cranelift).unwrap_or(cltypes::I64);

            let result_val = match (src_cl_ty, dest_cl_ty) {
                // Identity: same Cranelift types, no conversion needed
                (t1, t2) if t1 == t2 => src_val,

                // Bool/None (i8) to Int/Any/Ptr (i64) - unsigned extend
                (cltypes::I8, cltypes::I64) => builder.ins().uextend(cltypes::I64, src_val),

                // Int/Any/Ptr (i64) to Bool/None (i8) - reduce
                (cltypes::I64, cltypes::I8) => builder.ins().ireduce(cltypes::I8, src_val),

                // Float to Int - convert float to signed int (truncates towards zero)
                (cltypes::F64, cltypes::I64) => builder.ins().fcvt_to_sint(cltypes::I64, src_val),

                // Int to Float - convert signed int to float
                (cltypes::I64, cltypes::F64) => builder.ins().fcvt_from_sint(cltypes::F64, src_val),

                // Float to Bool/None (i8) - convert to int first, then reduce
                (cltypes::F64, cltypes::I8) => {
                    let as_int = builder.ins().fcvt_to_sint(cltypes::I64, src_val);
                    builder.ins().ireduce(cltypes::I8, as_int)
                }

                // Bool/None (i8) to Float - extend to int first, then convert to float
                (cltypes::I8, cltypes::F64) => {
                    let as_int = builder.ins().uextend(cltypes::I64, src_val);
                    builder.ins().fcvt_from_sint(cltypes::F64, as_int)
                }

                // All other cases: no conversion needed (e.g., pointer types, union types)
                // Union types, heap object pointers, etc. all use i64 and don't need conversion
                _ => {
                    #[cfg(debug_assertions)]
                    {
                        // In debug builds, warn about unhandled conversions
                        if src_cl_ty != dest_cl_ty {
                            eprintln!(
                                "Warning: Unhandled type conversion {:?} -> {:?} (src: {:?}, dest: {:?})",
                                src_cl_ty, dest_cl_ty, src_ty, dest_ty
                            );
                        }
                    }
                    src_val
                }
            };

            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result_val);
            // Update GC root if needed
            update_gc_root_if_needed(builder, dest, result_val, ctx.gc_frame_data);
        }

        mir::InstructionKind::CallDirect { dest, func, args } => {
            // Get the Cranelift function ID
            let cl_func_id = match ctx.func_ids.get(func) {
                Some(id) => *id,
                None => {
                    return Err(pyaot_diagnostics::CompilerError::codegen_error(format!(
                        "Function ID {:?} not found in module",
                        func
                    )))
                }
            };

            // Get a function reference
            let func_ref = ctx.module.declare_func_in_func(cl_func_id, builder.func);

            // Get expected parameter types for type coercion
            let param_types = ctx.func_param_types.get(func);

            // Prepare arguments, applying type coercion where needed (e.g., Bool -> Int, primitives -> Any)
            let mut arg_vals = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                let arg_val = load_operand(builder, arg, ctx.var_map);

                // Get argument type and expected parameter type
                let arg_type = match arg {
                    Operand::Local(local_id) => ctx.locals.get(local_id).map(|l| &l.ty),
                    Operand::Constant(c) => Some(match c {
                        mir::Constant::Int(_) => &Type::Int,
                        mir::Constant::Float(_) => &Type::Float,
                        mir::Constant::Bool(_) => &Type::Bool,
                        mir::Constant::None => &Type::None,
                        _ => &Type::Int,
                    }),
                };
                let param_type = param_types.and_then(|pts| pts.get(i));

                // Coerce types: Bool -> Int, and non-i64 primitives -> Any
                // Note: Int is already i64, so no coercion needed for Int -> Any
                // This also preserves closure captures which pass values directly
                let arg_cl_type = builder.func.dfg.value_type(arg_val);
                let coerced_val = match (arg_type, param_type) {
                    (Some(Type::Bool), Some(Type::Int)) if arg_cl_type == cltypes::I8 => {
                        // Extend i8 to i64
                        builder.ins().uextend(cltypes::I64, arg_val)
                    }
                    // For Any parameters, only convert types that have different Cranelift representations
                    // Int is already i64, same as Any, so no conversion needed
                    (Some(Type::Float), Some(Type::Any)) if arg_cl_type == cltypes::F64 => {
                        // f64 -> i64: need to box the float
                        box_primitive(builder, ctx.module, "rt_box_float", cltypes::F64, arg_val)?
                    }
                    (Some(Type::Bool), Some(Type::Any)) if arg_cl_type == cltypes::I8 => {
                        // i8 -> i64: extend bool to i64 for Any parameter
                        builder.ins().uextend(cltypes::I64, arg_val)
                    }
                    (Some(Type::None), Some(Type::Any)) if arg_cl_type == cltypes::I8 => {
                        // i8 -> i64: extend None to i64 for Any parameter
                        builder.ins().uextend(cltypes::I64, arg_val)
                    }
                    // None passed for pointer-typed parameters (list, dict, str, tuple, etc.)
                    // None is i8 (0) but pointer params expect i64 (null pointer)
                    (Some(Type::None), Some(param_ty)) if arg_cl_type == cltypes::I8 => {
                        let expected = crate::utils::type_to_cranelift(param_ty);
                        if expected == cltypes::I64 {
                            builder.ins().uextend(cltypes::I64, arg_val)
                        } else {
                            arg_val
                        }
                    }
                    // Fallback: check Cranelift function signature for type mismatch.
                    // This handles cases where param_types is unavailable (e.g., None
                    // constant passed as default for a pointer-typed parameter).
                    _ => {
                        let sig = &builder.func.dfg.signatures
                            [builder.func.dfg.ext_funcs[func_ref].signature];
                        if let Some(expected_param) = sig.params.get(i) {
                            let expected_ty = expected_param.value_type;
                            if arg_cl_type == cltypes::I8 && expected_ty == cltypes::I64 {
                                builder.ins().uextend(cltypes::I64, arg_val)
                            } else {
                                arg_val
                            }
                        } else {
                            arg_val
                        }
                    }
                };

                arg_vals.push(coerced_val);
            }

            // Make the call
            let call_inst = builder.ins().call(func_ref, &arg_vals);

            // Get the return value
            let results = builder.inst_results(call_inst);
            if !results.is_empty() {
                let result_val = results[0];
                let dest_var = *ctx
                    .var_map
                    .get(dest)
                    .expect("internal error: local not in var_map - codegen bug");
                builder.def_var(dest_var, result_val);
                // Update GC root if needed
                update_gc_root_if_needed(builder, dest, result_val, ctx.gc_frame_data);
            }
        }

        mir::InstructionKind::CallNamed { dest, name, args } => {
            // Look up Cranelift function ID by name (for cross-module calls)
            let cl_func_id = match ctx.func_name_ids.get(name) {
                Some(id) => *id,
                None => {
                    return Err(pyaot_diagnostics::CompilerError::codegen_error(format!(
                        "Function '{}' not found in module",
                        name
                    )))
                }
            };

            // Get a function reference
            let func_ref = ctx.module.declare_func_in_func(cl_func_id, builder.func);

            // Prepare arguments
            let mut arg_vals = Vec::new();
            for arg in args {
                let arg_val = load_operand(builder, arg, ctx.var_map);
                arg_vals.push(arg_val);
            }

            // Make the call
            let call_inst = builder.ins().call(func_ref, &arg_vals);

            // Get the return value
            let results = builder.inst_results(call_inst);
            if !results.is_empty() {
                let result_val = results[0];
                let dest_var = *ctx
                    .var_map
                    .get(dest)
                    .expect("internal error: local not in var_map - codegen bug");
                builder.def_var(dest_var, result_val);
                // Update GC root if needed
                update_gc_root_if_needed(builder, dest, result_val, ctx.gc_frame_data);
            }
        }

        mir::InstructionKind::Call { dest, func, args } => {
            // Indirect call through a function pointer
            // Load the function pointer
            let func_ptr = load_operand(builder, func, ctx.var_map);

            // Prepare arguments
            let mut arg_vals = Vec::new();
            for arg in args {
                let arg_val = load_operand(builder, arg, ctx.var_map);
                arg_vals.push(arg_val);
            }

            // Get the destination type to determine return type
            let dest_local = ctx.locals.get(dest);
            let return_type = dest_local
                .map(|l| crate::utils::type_to_cranelift(&l.ty))
                .unwrap_or(cltypes::I64);

            // Build the signature for the indirect call
            // All parameters are I64 (pointers or integers)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            for _ in 0..arg_vals.len() {
                sig.params.push(AbiParam::new(cltypes::I64));
            }
            sig.returns.push(AbiParam::new(return_type));

            let sig_ref = builder.import_signature(sig);

            // Make indirect call
            let call_inst = builder.ins().call_indirect(sig_ref, func_ptr, &arg_vals);

            // Get the return value
            let results = builder.inst_results(call_inst);
            if !results.is_empty() {
                let result_val = results[0];
                let dest_var = *ctx
                    .var_map
                    .get(dest)
                    .expect("internal error: local not in var_map - codegen bug");
                builder.def_var(dest_var, result_val);
                // Update GC root if needed
                update_gc_root_if_needed(builder, dest, result_val, ctx.gc_frame_data);
            }
        }

        mir::InstructionKind::CallVirtual {
            dest,
            obj,
            slot,
            args,
        } => {
            // Load the object pointer (self)
            let obj_val = load_operand(builder, obj, ctx.var_map);

            // InstanceObj layout:
            // - header: ObjHeader (type_tag: u8 + marked: bool + size: usize = 10 bytes, aligned to 16)
            // - vtable: *const u8 (8 bytes) - at offset 16
            // - class_id: u8
            // - field_count: usize
            // - fields: [*mut Obj; 0]
            //
            // ObjHeader is 10 bytes but aligned, so vtable is at offset 16 (after padding)
            let vtable_offset = 16i32;

            // Load vtable pointer from instance
            let vtable_ptr = builder.ins().load(
                cltypes::I64,
                cranelift_codegen::ir::MemFlags::new(),
                obj_val,
                vtable_offset,
            );

            // Vtable layout: [num_slots: u64, method_ptrs: [*const (); num_slots]]
            // Method pointer at offset 8 + slot * 8
            let method_offset = (8 + slot * 8) as i32;
            let method_ptr = builder.ins().load(
                cltypes::I64,
                cranelift_codegen::ir::MemFlags::new(),
                vtable_ptr,
                method_offset,
            );

            // Build arguments: self first, then additional args
            let mut arg_vals = vec![obj_val];
            for arg in args {
                let arg_val = load_operand(builder, arg, ctx.var_map);
                arg_vals.push(arg_val);
            }

            // Get the destination type to determine return type
            let dest_local = match ctx.locals.get(dest) {
                Some(local) => local,
                None => {
                    return Err(pyaot_diagnostics::CompilerError::codegen_error(format!(
                        "Destination local {:?} not found for virtual call",
                        dest
                    )))
                }
            };
            let return_type = crate::utils::type_to_cranelift(&dest_local.ty);

            // Build the signature for the indirect call
            // All method parameters are I64 (pointers or integers, including self)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            for _ in 0..arg_vals.len() {
                sig.params.push(AbiParam::new(cltypes::I64));
            }
            // Return type matches the destination variable's type
            sig.returns.push(AbiParam::new(return_type));

            let sig_ref = builder.import_signature(sig);

            // Make indirect call
            let call_inst = builder.ins().call_indirect(sig_ref, method_ptr, &arg_vals);

            // Get the return value
            let results = builder.inst_results(call_inst);
            if !results.is_empty() {
                let result_val = results[0];
                let dest_var = *ctx
                    .var_map
                    .get(dest)
                    .expect("internal error: local not in var_map - codegen bug");
                builder.def_var(dest_var, result_val);
                // Update GC root if needed
                update_gc_root_if_needed(builder, dest, result_val, ctx.gc_frame_data);
            }
        }

        mir::InstructionKind::FuncAddr { dest, func } => {
            // Get the Cranelift function ID
            let cl_func_id = match ctx.func_ids.get(func) {
                Some(id) => *id,
                None => {
                    return Err(pyaot_diagnostics::CompilerError::codegen_error(format!(
                        "Function ID {:?} not found for address lookup",
                        func
                    )))
                }
            };

            // Get a function reference in this function
            let func_ref = ctx.module.declare_func_in_func(cl_func_id, builder.func);

            // Get the function's address as a pointer
            let func_addr = builder.ins().func_addr(cltypes::I64, func_ref);

            let dest_var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, func_addr);
        }

        mir::InstructionKind::BuiltinAddr { dest, builtin } => {
            // Get function pointer for a builtin from the runtime table
            // Call rt_get_builtin_func_ptr(builtin_id) -> func_ptr
            let builtin_id = builtin.id() as i64;

            // Build signature: (i64) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.returns.push(AbiParam::new(cltypes::I64));

            let func_id = declare_runtime_function(ctx.module, "rt_get_builtin_func_ptr", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let id_val = builder.ins().iconst(cltypes::I64, builtin_id);
            let call_inst = builder.ins().call(func_ref, &[id_val]);
            let result = builder.inst_results(call_inst)[0];

            let dest_var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result);
        }

        mir::InstructionKind::UnOp { dest, op, operand } => {
            let operand_val = load_operand(builder, operand, ctx.var_map);
            let is_float = is_float_operand(operand, ctx.locals);
            let result = match op {
                mir::UnOp::Neg => {
                    if is_float {
                        builder.ins().fneg(operand_val)
                    } else {
                        builder.ins().ineg(operand_val)
                    }
                }
                mir::UnOp::Not => {
                    // For boolean not: result = 1 - operand (for i8 bool)
                    let one = builder.ins().iconst(cltypes::I8, 1);
                    builder.ins().isub(one, operand_val)
                }
                mir::UnOp::Invert => {
                    // Bitwise NOT: result = ~operand (flip all bits)
                    builder.ins().bnot(operand_val)
                }
            };
            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result);
            // Update GC root if needed
            update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
        }

        mir::InstructionKind::RuntimeCall { dest, func, args } => {
            compile_runtime_call(builder, *dest, func, args, ctx)?;
        }

        // Exception handling instructions
        mir::InstructionKind::ExcPushFrame { frame_local } => {
            compile_exc_push_frame(builder, frame_local, ctx)?;
        }

        mir::InstructionKind::ExcPopFrame => {
            compile_exc_pop_frame(builder, ctx)?;
        }

        mir::InstructionKind::ExcGetType { dest } => {
            compile_exc_get_type(builder, dest, ctx)?;
        }

        mir::InstructionKind::ExcClear => {
            compile_exc_clear(builder, ctx)?;
        }

        mir::InstructionKind::ExcHasException { dest } => {
            compile_exc_has_exception(builder, dest, ctx)?;
        }

        mir::InstructionKind::ExcGetCurrent { dest } => {
            compile_exc_get_current(builder, dest, ctx)?;
        }

        mir::InstructionKind::ExcCheckType { dest, type_tag } => {
            compile_exc_check_type(builder, dest, *type_tag, ctx)?;
        }

        mir::InstructionKind::ExcCheckClass { dest, class_id } => {
            compile_exc_check_class(builder, dest, *class_id, ctx)?;
        }

        mir::InstructionKind::ExcStartHandling => {
            compile_exc_start_handling(builder, ctx)?;
        }

        mir::InstructionKind::ExcEndHandling => {
            compile_exc_end_handling(builder, ctx)?;
        }

        // Type conversion instructions
        mir::InstructionKind::FloatToInt { dest, src } => {
            let src_val = load_operand(builder, src, ctx.var_map);
            // fcvt_to_sint: convert float to signed integer, truncating towards zero
            let result = builder.ins().fcvt_to_sint(cltypes::I64, src_val);
            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
        }

        mir::InstructionKind::BoolToInt { dest, src } => {
            let src_val = load_operand(builder, src, ctx.var_map);
            // uextend: zero-extend i8 bool to i64 int (0 -> 0, 1 -> 1)
            let result = builder.ins().uextend(cltypes::I64, src_val);
            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
        }

        mir::InstructionKind::IntToFloat { dest, src } => {
            let src_val = load_operand(builder, src, ctx.var_map);
            // fcvt_from_sint: convert signed integer to float
            let result = builder.ins().fcvt_from_sint(cltypes::F64, src_val);
            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
        }

        mir::InstructionKind::FloatAbs { dest, src } => {
            let src_val = load_operand(builder, src, ctx.var_map);
            // fabs: compute absolute value of float
            let result = builder.ins().fabs(src_val);
            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
        }

        mir::InstructionKind::FloatBits { dest, src } => {
            let src_val = load_operand(builder, src, ctx.var_map);
            // bitcast: reinterpret f64 bits as i64
            let result = builder.ins().bitcast(
                cltypes::I64,
                cranelift_codegen::ir::MemFlags::new(),
                src_val,
            );
            let var = *ctx
                .var_map
                .get(dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, result);
            update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
        }

        _ => {
            // Skip unsupported instructions for now
        }
    }
    Ok(())
}

/// Promote integer operands to matching types for comparison
/// When comparing i8 (bool) with i64 (int/any/ptr), promote both to i64
fn promote_int_operands(
    builder: &mut FunctionBuilder,
    left_val: cranelift_codegen::ir::Value,
    right_val: cranelift_codegen::ir::Value,
    _left: &Operand,
    _right: &Operand,
    _locals: &indexmap::IndexMap<pyaot_utils::LocalId, mir::Local>,
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
fn compile_binop(
    builder: &mut FunctionBuilder,
    dest: &pyaot_utils::LocalId,
    op: &mir::BinOp,
    left: &Operand,
    right: &Operand,
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Check if this is a float operation (either operand is float)
    let left_is_float = is_float_operand(left, ctx.locals);
    let right_is_float = is_float_operand(right, ctx.locals);
    let is_float = left_is_float || right_is_float;

    // Determine if this is a boolean operation that should keep i8 operands
    let is_bool_op = matches!(op, mir::BinOp::And | mir::BinOp::Or);

    // Load operands with appropriate type coercion:
    // - Float operations: load as-is (will be promoted to float later)
    // - Boolean operations (And, Or): keep as i8
    // - Integer operations: coerce Bool (i8) to Int (i64) for Python semantics
    let (left_val, right_val) = if is_float || is_bool_op {
        (
            load_operand(builder, left, ctx.var_map),
            load_operand(builder, right, ctx.var_map),
        )
    } else {
        // For integer operations, ensure both operands are i64
        // This coerces Bool (i8) to Int (i64) as needed
        (
            load_operand_as(builder, left, ctx.var_map, cltypes::I64),
            load_operand_as(builder, right, ctx.var_map, cltypes::I64),
        )
    };

    let result = if is_float {
        // Promote int operands to float for mixed-type operations
        let left_float = promote_to_float(builder, left_val, left, ctx.locals);
        let right_float = promote_to_float(builder, right_val, right, ctx.locals);

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
                ));
            }
        }
    } else {
        // Integer operations - use runtime functions with overflow/division-by-zero checks
        match op {
            mir::BinOp::Add => {
                // Call rt_add_int(a: i64, b: i64) -> i64 (raises OverflowError)
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.returns.push(AbiParam::new(cltypes::I64));

                let func_id = declare_runtime_function(ctx.module, "rt_add_int", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_val, right_val]);
                get_call_result(builder, call_inst)
            }
            mir::BinOp::Sub => {
                // Call rt_sub_int(a: i64, b: i64) -> i64 (raises OverflowError)
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.returns.push(AbiParam::new(cltypes::I64));

                let func_id = declare_runtime_function(ctx.module, "rt_sub_int", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_val, right_val]);
                get_call_result(builder, call_inst)
            }
            mir::BinOp::Mul => {
                // Call rt_mul_int(a: i64, b: i64) -> i64 (raises OverflowError)
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.returns.push(AbiParam::new(cltypes::I64));

                let func_id = declare_runtime_function(ctx.module, "rt_mul_int", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_val, right_val]);
                get_call_result(builder, call_inst)
            }
            mir::BinOp::Div => {
                // Python 3: true division always returns float
                // Call rt_true_div_int(a: i64, b: i64) -> f64 (raises ZeroDivisionError)
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.returns.push(AbiParam::new(cltypes::F64));

                let func_id = declare_runtime_function(ctx.module, "rt_true_div_int", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_val, right_val]);
                get_call_result(builder, call_inst)
            }
            mir::BinOp::FloorDiv => {
                // Floor division returns int for int operands
                // Call rt_div_int(a: i64, b: i64) -> i64 (raises ZeroDivisionError)
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.returns.push(AbiParam::new(cltypes::I64));

                let func_id = declare_runtime_function(ctx.module, "rt_div_int", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_val, right_val]);
                get_call_result(builder, call_inst)
            }
            mir::BinOp::Mod => {
                // Call rt_mod_int(a: i64, b: i64) -> i64 (raises ZeroDivisionError)
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.returns.push(AbiParam::new(cltypes::I64));

                let func_id = declare_runtime_function(ctx.module, "rt_mod_int", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_val, right_val]);
                get_call_result(builder, call_inst)
            }
            mir::BinOp::Pow => {
                // Call rt_pow_int(base: i64, exp: i64) -> i64
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.params.push(AbiParam::new(cltypes::I64));
                sig.returns.push(AbiParam::new(cltypes::I64));

                let func_id = declare_runtime_function(ctx.module, "rt_pow_int", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                let call_inst = builder.ins().call(func_ref, &[left_val, right_val]);
                get_call_result(builder, call_inst)
            }
            // Integer comparison operations - icmp returns i1, extend to dest type
            // First, promote operands to matching types if needed (i8 vs i64)
            mir::BinOp::Eq => {
                let (l, r) =
                    promote_int_operands(builder, left_val, right_val, left, right, ctx.locals);
                let cmp = builder
                    .ins()
                    .icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, l, r);
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::NotEq => {
                let (l, r) =
                    promote_int_operands(builder, left_val, right_val, left, right, ctx.locals);
                let cmp =
                    builder
                        .ins()
                        .icmp(cranelift_codegen::ir::condcodes::IntCC::NotEqual, l, r);
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::Lt => {
                let (l, r) =
                    promote_int_operands(builder, left_val, right_val, left, right, ctx.locals);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                    l,
                    r,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::LtE => {
                let (l, r) =
                    promote_int_operands(builder, left_val, right_val, left, right, ctx.locals);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThanOrEqual,
                    l,
                    r,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::Gt => {
                let (l, r) =
                    promote_int_operands(builder, left_val, right_val, left, right, ctx.locals);
                let cmp = builder.ins().icmp(
                    cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThan,
                    l,
                    r,
                );
                extend_comparison_result(builder, cmp, dest, ctx)
            }
            mir::BinOp::GtE => {
                let (l, r) =
                    promote_int_operands(builder, left_val, right_val, left, right, ctx.locals);
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

    let var = *ctx
        .var_map
        .get(dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(var, result);
    // Update GC root if needed (e.g., future string concatenation)
    update_gc_root_if_needed(builder, dest, result, ctx.gc_frame_data);
    Ok(())
}

/// Extend an i1 comparison result to the target type based on destination variable type.
/// icmp/fcmp return i1 (native bool), but Python bools are i8 and other types (int, Any, etc.) are i64.
fn extend_comparison_result(
    _builder: &mut FunctionBuilder,
    cmp_result: Value,
    _dest: &pyaot_utils::LocalId,
    _ctx: &CodegenContext,
) -> Value {
    // Just return the comparison result directly (i1) - Cranelift will handle conversion
    cmp_result
}

/// Box a primitive value (int, float, bool) for passing to Any-typed parameters.
/// Returns a boxed object pointer (i64).
fn box_primitive(
    builder: &mut FunctionBuilder,
    module: &mut cranelift_object::ObjectModule,
    func_name: &str,
    param_type: cltypes::Type,
    value: Value,
) -> pyaot_diagnostics::Result<Value> {
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(param_type));
    sig.returns.push(AbiParam::new(cltypes::I64));

    let func_id = declare_runtime_function(module, func_name, &sig)?;
    let func_ref = module.declare_func_in_func(func_id, builder.func);
    let call_inst = builder.ins().call(func_ref, &[value]);
    Ok(get_call_result(builder, call_inst))
}
