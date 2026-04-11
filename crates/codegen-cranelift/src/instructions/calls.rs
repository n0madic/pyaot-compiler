//! Call-related instruction compilation
//!
//! This module handles code generation for call instructions: CallDirect, CallNamed,
//! Call (indirect), CallVirtual, CallVirtualNamed, FuncAddr, and BuiltinAddr.

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, FuncRef, InstBuilder, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_core_defs::layout;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, BuiltinFunctionKind, Operand};
use pyaot_types::Type;
use pyaot_utils::{FuncId, LocalId};

use crate::context::CodegenContext;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile a direct function call: `dest = func(args)`.
///
/// Resolves the function by `FuncId`, coerces argument types to match the callee
/// signature (Bool -> Int, primitives -> Any, etc.), and stores the return value.
pub(crate) fn compile_call_direct(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    func: &FuncId,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Get the Cranelift function ID
    let cl_func_id = match ctx.symbols.func_ids.get(func) {
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
    let param_types = ctx.symbols.func_param_types.get(func);

    // Prepare arguments, applying type coercion where needed (e.g., Bool -> Int, primitives -> Any)
    let mut arg_vals = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let arg_val = load_operand(builder, arg, ctx.symbols.var_map);

        // Get argument type and expected parameter type
        let arg_type = match arg {
            Operand::Local(local_id) => ctx.symbols.locals.get(local_id).map(|l| &l.ty),
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
            // For Any/Union parameters, only convert types that have different Cranelift representations
            // Int is already i64, same as Any/Union, so no conversion needed
            (Some(Type::Float), Some(Type::Any | Type::Union(_)))
                if arg_cl_type == cltypes::F64 =>
            {
                // f64 -> i64: need to box the float
                box_primitive(builder, ctx.module, "rt_box_float", cltypes::F64, arg_val)?
            }
            (Some(Type::Bool), Some(Type::Any | Type::Union(_))) if arg_cl_type == cltypes::I8 => {
                // i8 -> i64: extend bool to i64 for Any/Union parameter
                builder.ins().uextend(cltypes::I64, arg_val)
            }
            (Some(Type::None), Some(Type::Any | Type::Union(_))) if arg_cl_type == cltypes::I8 => {
                // i8 -> i64: extend None to i64 for Any/Union parameter
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
            _ => coerce_arg_by_signature(builder, ctx.module, arg_val, func_ref, i)?,
        };

        arg_vals.push(coerced_val);
    }

    // Make the call
    let call_inst = builder.ins().call(func_ref, &arg_vals);

    // Get the return value
    let results = builder.inst_results(call_inst);
    if !results.is_empty() {
        let result_val = results[0];
        ctx.store_result(builder, dest, result_val);
    }
    Ok(())
}

/// Compile a named function call: `dest = name(args)`.
///
/// Looks up the function by name string (for cross-module calls), coerces arguments
/// via Cranelift signature inspection, and stores the return value.
pub(crate) fn compile_call_named(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    name: &str,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Look up Cranelift function ID by name (for cross-module calls)
    let cl_func_id = match ctx.symbols.func_name_ids.get(name) {
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

    // Prepare arguments with type coercion via Cranelift signature inspection
    let mut arg_vals = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let arg_val = load_operand(builder, arg, ctx.symbols.var_map);
        let coerced_val = coerce_arg_by_signature(builder, ctx.module, arg_val, func_ref, i)?;
        arg_vals.push(coerced_val);
    }

    // Make the call
    let call_inst = builder.ins().call(func_ref, &arg_vals);

    // Get the return value
    let results = builder.inst_results(call_inst);
    if !results.is_empty() {
        let result_val = results[0];
        ctx.store_result(builder, dest, result_val);
    }
    Ok(())
}

/// Compile an indirect function call: `dest = func_ptr(args)`.
///
/// Loads the function pointer from an operand, builds a call signature from the
/// actual argument types and the destination's return type, then makes an indirect call.
pub(crate) fn compile_call_indirect(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    func: &Operand,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Indirect call through a function pointer
    // Load the function pointer
    let func_ptr = load_operand(builder, func, ctx.symbols.var_map);

    // Prepare arguments
    let mut arg_vals = Vec::new();
    for arg in args {
        let arg_val = load_operand(builder, arg, ctx.symbols.var_map);
        arg_vals.push(arg_val);
    }

    // Get the destination type to determine return type
    let dest_local = ctx.symbols.locals.get(dest);
    let return_type = dest_local
        .map(|l| crate::utils::type_to_cranelift(&l.ty))
        .unwrap_or(cltypes::I64);

    // Build the signature for the indirect call using actual value types
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    for arg_val in &arg_vals {
        let arg_ty = builder.func.dfg.value_type(*arg_val);
        sig.params.push(AbiParam::new(arg_ty));
    }
    sig.returns.push(AbiParam::new(return_type));

    let sig_ref = builder.import_signature(sig);

    // Make indirect call
    let call_inst = builder.ins().call_indirect(sig_ref, func_ptr, &arg_vals);

    // Get the return value
    let results = builder.inst_results(call_inst);
    if !results.is_empty() {
        let result_val = results[0];
        ctx.store_result(builder, dest, result_val);
    }
    Ok(())
}

/// Compile a virtual method call via vtable: `dest = obj.vtable[slot](obj, args...)`.
///
/// Loads the vtable pointer from the instance object, extracts the method pointer
/// at the given slot, prepends `self` to the arguments, and makes an indirect call.
pub(crate) fn compile_call_virtual(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    obj: &Operand,
    slot: usize,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Load the object pointer (self)
    let obj_val = load_operand(builder, obj, ctx.symbols.var_map);

    // InstanceObj: ObjHeader followed by vtable pointer
    let vtable_offset = layout::INSTANCE_VTABLE_OFFSET;

    // Load vtable pointer from instance
    let vtable_ptr = builder.ins().load(
        cltypes::I64,
        cranelift_codegen::ir::MemFlags::new(),
        obj_val,
        vtable_offset,
    );

    // Vtable layout: [num_slots: u64, method_ptrs: [*const (); num_slots]]
    let method_offset = layout::vtable_slot_offset(slot);
    let method_ptr = builder.ins().load(
        cltypes::I64,
        cranelift_codegen::ir::MemFlags::new(),
        vtable_ptr,
        method_offset,
    );

    // Build arguments: self first, then additional args
    let mut arg_vals = vec![obj_val];
    for arg in args {
        let arg_val = load_operand(builder, arg, ctx.symbols.var_map);
        arg_vals.push(arg_val);
    }

    // Get the destination type to determine return type
    let dest_local = match ctx.symbols.locals.get(dest) {
        Some(local) => local,
        None => {
            return Err(pyaot_diagnostics::CompilerError::codegen_error(format!(
                "Destination local {:?} not found for virtual call",
                dest
            )))
        }
    };
    let return_type = crate::utils::type_to_cranelift(&dest_local.ty);

    // Build the signature for the indirect call using actual value types
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    for arg_val in &arg_vals {
        let arg_ty = builder.func.dfg.value_type(*arg_val);
        sig.params.push(AbiParam::new(arg_ty));
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
        ctx.store_result(builder, dest, result_val);
    }
    Ok(())
}

/// Compile a name-based virtual method call: `dest = rt_vtable_lookup_by_name(obj, hash)(obj, args...)`.
///
/// Used for Protocol dispatch where the vtable slot is not known at compile time.
/// Calls the runtime to resolve the method pointer by name hash, then makes an
/// indirect call with `self` prepended.
pub(crate) fn compile_call_virtual_named(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    obj: &Operand,
    name_hash: u64,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Name-based virtual dispatch for Protocol types.
    // Calls rt_vtable_lookup_by_name(obj, name_hash) to get the method pointer,
    // then makes an indirect call.
    let obj_val = load_operand(builder, obj, ctx.symbols.var_map);

    // Declare rt_vtable_lookup_by_name(obj: *mut u8, name_hash: i64) -> *const u8
    let mut lookup_sig = ctx.module.make_signature();
    lookup_sig.call_conv = CallConv::SystemV;
    lookup_sig.params.push(AbiParam::new(cltypes::I64)); // obj
    lookup_sig.params.push(AbiParam::new(cltypes::I64)); // name_hash
    lookup_sig.returns.push(AbiParam::new(cltypes::I64)); // fn ptr

    let lookup_func_id = crate::utils::declare_runtime_function(
        ctx.module,
        "rt_vtable_lookup_by_name",
        &lookup_sig,
    )?;
    let lookup_ref = ctx
        .module
        .declare_func_in_func(lookup_func_id, builder.func);

    let hash_val = builder.ins().iconst(cltypes::I64, name_hash as i64);
    let lookup_call = builder.ins().call(lookup_ref, &[obj_val, hash_val]);
    let method_ptr = crate::utils::get_call_result(builder, lookup_call);

    // Build arguments: self first, then additional args
    let mut arg_vals = vec![obj_val];
    for arg in args {
        let arg_val = load_operand(builder, arg, ctx.symbols.var_map);
        arg_vals.push(arg_val);
    }

    // Get return type from destination local
    let dest_local = ctx
        .symbols
        .locals
        .get(dest)
        .expect("internal error: dest local not found for CallVirtualNamed");
    let return_type = crate::utils::type_to_cranelift(&dest_local.ty);

    // Build signature for indirect call
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    for arg_val in &arg_vals {
        let arg_ty = builder.func.dfg.value_type(*arg_val);
        sig.params.push(AbiParam::new(arg_ty));
    }
    sig.returns.push(AbiParam::new(return_type));
    let sig_ref = builder.import_signature(sig);

    // Indirect call through the resolved method pointer
    let call_inst = builder.ins().call_indirect(sig_ref, method_ptr, &arg_vals);

    let results = builder.inst_results(call_inst);
    if !results.is_empty() {
        let result_val = results[0];
        ctx.store_result(builder, dest, result_val);
    }
    Ok(())
}

/// Compile a function address lookup: `dest = &func`.
///
/// Gets the address of a compiled function as an i64 pointer value.
pub(crate) fn compile_func_addr(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    func: &FuncId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Get the Cranelift function ID
    let cl_func_id = match ctx.symbols.func_ids.get(func) {
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

    ctx.store_result(builder, dest, func_addr);
    Ok(())
}

/// Compile a builtin function address lookup: `dest = rt_get_builtin_func_ptr(builtin_id)`.
///
/// Retrieves a function pointer for a builtin function (e.g., `len`, `str`, `int`)
/// from the runtime's builtin function table.
pub(crate) fn compile_builtin_addr(
    builder: &mut FunctionBuilder,
    dest: &LocalId,
    builtin: &BuiltinFunctionKind,
    ctx: &mut CodegenContext,
) -> Result<()> {
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

    ctx.store_result(builder, dest, result);
    Ok(())
}

/// Coerce a call argument based on the Cranelift function signature.
///
/// Handles mismatches between the actual argument type and the expected parameter type:
/// - I8 -> I64: zero-extend (bool/None to int-sized)
/// - F64 -> I64: box float via `rt_box_float` (for Any/Union parameters)
/// - Same type or unrecognized mismatch: pass through unchanged
pub(crate) fn coerce_arg_by_signature(
    builder: &mut FunctionBuilder,
    module: &mut cranelift_object::ObjectModule,
    arg_val: Value,
    func_ref: FuncRef,
    arg_index: usize,
) -> pyaot_diagnostics::Result<Value> {
    let arg_type = builder.func.dfg.value_type(arg_val);
    let sig = &builder.func.dfg.signatures[builder.func.dfg.ext_funcs[func_ref].signature];
    let Some(expected_param) = sig.params.get(arg_index) else {
        return Ok(arg_val);
    };
    let expected_ty = expected_param.value_type;

    if arg_type == expected_ty {
        return Ok(arg_val);
    }

    // I8 -> I64: extend bool/None to int-sized
    if arg_type == cltypes::I8 && expected_ty == cltypes::I64 {
        return Ok(builder.ins().uextend(cltypes::I64, arg_val));
    }

    // F64 -> I64: box float for Any/Union parameters
    if arg_type == cltypes::F64 && expected_ty == cltypes::I64 {
        return box_primitive(builder, module, "rt_box_float", cltypes::F64, arg_val);
    }

    // Fallback: pass through unchanged
    Ok(arg_val)
}

/// Box a primitive value (int, float, bool) for passing to Any-typed parameters.
/// Returns a boxed object pointer (i64).
pub(crate) fn box_primitive(
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
