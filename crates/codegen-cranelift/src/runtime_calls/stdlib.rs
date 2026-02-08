//! Standard library runtime call code generation
//!
//! Handles code generation for stdlib calls using definitions from stdlib-defs
//! (Single Source of Truth) and Match object methods.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_mir::{self as mir, Operand};
use pyaot_stdlib_defs::{
    ObjectFieldDef, StdlibAttrDef, StdlibFunctionDef, StdlibMethodDef, TypeSpec,
};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile a stdlib RuntimeCall instruction
pub fn compile_stdlib_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<(), pyaot_diagnostics::CompilerError> {
    match func {
        // Generic stdlib call handler (Single Source of Truth)
        mir::RuntimeFunc::StdlibCall(func_def) => {
            compile_generic_stdlib_call(builder, dest, func_def, args, ctx)?
        }
        // Generic stdlib attr getter (Single Source of Truth)
        mir::RuntimeFunc::StdlibAttrGet(attr_def) => {
            compile_generic_stdlib_attr(builder, dest, attr_def, ctx)?
        }
        // Generic object field getter (Single Source of Truth)
        mir::RuntimeFunc::ObjectFieldGet(field_def) => {
            compile_generic_object_field_get(builder, dest, field_def, args, ctx)?
        }
        // Generic object method call (Single Source of Truth)
        mir::RuntimeFunc::ObjectMethodCall(method_def) => {
            compile_generic_object_method_call(builder, dest, method_def, args, ctx)?
        }
        _ => {}
    }
    Ok(())
}

/// Compile a generic stdlib function call using the definition
fn compile_generic_stdlib_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func_def: &'static StdlibFunctionDef,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<(), pyaot_diagnostics::CompilerError> {
    // Build signature from definition
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    // Build params from function definition
    for (i, _) in args.iter().enumerate() {
        let param_type = if i < func_def.params.len() {
            typespec_to_cranelift_type(&func_def.params[i].ty)
        } else {
            // Variadic or extra args default to I64 (pointer)
            cltypes::I64
        };
        sig.params
            .push(cranelift_codegen::ir::AbiParam::new(param_type));
    }

    // Return type from definition
    let ret_type = typespec_to_cranelift_type(&func_def.return_type);
    if !matches!(func_def.return_type, TypeSpec::None) {
        sig.returns
            .push(cranelift_codegen::ir::AbiParam::new(ret_type));
    }

    let func_id = declare_runtime_function(ctx.module, func_def.runtime_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    // Load all arguments with type coercion
    let arg_vals: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(i, arg)| {
            let target_type = if i < func_def.params.len() {
                typespec_to_cranelift_type(&func_def.params[i].ty)
            } else {
                cltypes::I64 // Variadic args default to pointer type
            };
            crate::utils::load_operand_as(builder, arg, ctx.var_map, target_type)
        })
        .collect();

    let call = builder.ins().call(func_ref, &arg_vals);

    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: local not in var_map - codegen bug");

    if matches!(func_def.return_type, TypeSpec::None) {
        // No return value - set to zero
        let zero = builder.ins().iconst(cltypes::I8, 0);
        builder.def_var(dest_var, zero);
    } else {
        let result = get_call_result(builder, call);
        builder.def_var(dest_var, result);
    }

    Ok(())
}

/// Compile a generic stdlib attribute getter using the definition
fn compile_generic_stdlib_attr(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    attr_def: &'static StdlibAttrDef,
    ctx: &mut CodegenContext,
) -> Result<(), pyaot_diagnostics::CompilerError> {
    // Build signature (no params, returns pointer)
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.returns
        .push(cranelift_codegen::ir::AbiParam::new(cltypes::I64));

    let func_id = declare_runtime_function(ctx.module, attr_def.runtime_getter, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let call = builder.ins().call(func_ref, &[]);
    let result = get_call_result(builder, call);

    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(dest_var, result);

    Ok(())
}

/// Convert TypeSpec to Cranelift type for return values
fn typespec_to_cranelift_type(spec: &TypeSpec) -> cranelift_codegen::ir::Type {
    match spec {
        TypeSpec::Int => cltypes::I64,
        TypeSpec::Float => cltypes::F64,
        TypeSpec::Bool => cltypes::I8,
        TypeSpec::None => cltypes::I8, // Placeholder
        _ => cltypes::I64,             // All heap types are pointers
    }
}

// ============================================================================
// Generic object field/method handlers (Single Source of Truth)
// ============================================================================

/// Compile a generic object field getter using the field definition
fn compile_generic_object_field_get(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    field_def: &'static ObjectFieldDef,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<(), pyaot_diagnostics::CompilerError> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    // Single object pointer parameter
    sig.params
        .push(cranelift_codegen::ir::AbiParam::new(cltypes::I64));

    // Return type is always I64 (either raw int or pointer)
    sig.returns
        .push(cranelift_codegen::ir::AbiParam::new(cltypes::I64));

    // Use runtime_getter from field definition
    let func_id = declare_runtime_function(ctx.module, field_def.runtime_getter, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let obj_val = load_operand(builder, &args[0], ctx.var_map);

    let call = builder.ins().call(func_ref, &[obj_val]);
    let result = get_call_result(builder, call);

    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(dest_var, result);
    Ok(())
}

/// Compile a generic object method call using the method definition
fn compile_generic_object_method_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    method_def: &'static StdlibMethodDef,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<(), pyaot_diagnostics::CompilerError> {
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    // First parameter is always self (object pointer)
    sig.params
        .push(cranelift_codegen::ir::AbiParam::new(cltypes::I64));

    // Add remaining parameters from method definition
    for param in method_def.params.iter() {
        let param_type = typespec_to_cranelift_type(&param.ty);
        sig.params
            .push(cranelift_codegen::ir::AbiParam::new(param_type));
    }

    // Return type from definition
    let ret_type = typespec_to_cranelift_type(&method_def.return_type);
    if !matches!(method_def.return_type, TypeSpec::None) {
        sig.returns
            .push(cranelift_codegen::ir::AbiParam::new(ret_type));
    }

    let func_id = declare_runtime_function(ctx.module, method_def.runtime_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    // Load all arguments with type coercion
    // args[0] is self (always I64 pointer), args[1+] are params from definition
    let arg_vals: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(i, arg)| {
            let target_type = if i == 0 {
                // Self is always a pointer
                cltypes::I64
            } else if i - 1 < method_def.params.len() {
                // Parameter types from definition
                typespec_to_cranelift_type(&method_def.params[i - 1].ty)
            } else {
                // Fallback for extra args
                cltypes::I64
            };
            crate::utils::load_operand_as(builder, arg, ctx.var_map, target_type)
        })
        .collect();

    let call = builder.ins().call(func_ref, &arg_vals);

    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: local not in var_map - codegen bug");

    if matches!(method_def.return_type, TypeSpec::None) {
        // No return value - set to zero
        let zero = builder.ins().iconst(cltypes::I8, 0);
        builder.def_var(dest_var, zero);
    } else {
        let result = get_call_result(builder, call);
        builder.def_var(dest_var, result);
    }

    Ok(())
}
