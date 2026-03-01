//! Global variable operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{Linkage, Module};
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand, ValueKind};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::runtime_helpers::{compile_global_get, compile_global_set};
use crate::utils::load_operand;

/// Get the Cranelift type for a ValueKind
fn value_kind_to_cltype(kind: ValueKind) -> cltypes::Type {
    match kind {
        ValueKind::Int => cltypes::I64,
        ValueKind::Float => cltypes::F64,
        ValueKind::Bool => cltypes::I8,
        ValueKind::Ptr => cltypes::I64,
    }
}

/// Get the runtime function name for GlobalSet
fn global_set_func_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Int => "rt_global_set_int",
        ValueKind::Float => "rt_global_set_float",
        ValueKind::Bool => "rt_global_set_bool",
        ValueKind::Ptr => "rt_global_set_ptr",
    }
}

/// Get the runtime function name for GlobalGet
fn global_get_func_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Int => "rt_global_get_int",
        ValueKind::Float => "rt_global_get_float",
        ValueKind::Bool => "rt_global_get_bool",
        ValueKind::Ptr => "rt_global_get_ptr",
    }
}

/// Compile a global variable-related runtime call
pub fn compile_global_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        // GlobalSet operations
        mir::RuntimeFunc::GlobalSet(kind) => {
            // GlobalSetPtr has special handling for type mismatch (bool/None -> i64)
            if matches!(kind, ValueKind::Ptr) {
                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I32)); // var_id
                sig.params.push(AbiParam::new(cltypes::I64)); // value (pointer as i64)

                let func_id = ctx
                    .module
                    .declare_function("rt_global_set_ptr", Linkage::Import, &sig)
                    .expect("Failed to declare runtime function 'rt_global_set_ptr'");
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

                let var_id_val = load_operand(builder, &args[0], ctx.var_map);
                let var_id_i32 = builder.ins().ireduce(cltypes::I32, var_id_val);
                let value_val = load_operand(builder, &args[1], ctx.var_map);
                // Handle type mismatch: coerce non-i64 values to i64 for pointer storage
                let value_ty = builder.func.dfg.value_type(value_val);
                let value_i64 = if value_ty == cltypes::I8 {
                    // Bool/None (i8) → extend to i64
                    builder.ins().uextend(cltypes::I64, value_val)
                } else if value_ty == cltypes::F64 {
                    // Float (f64) → bitcast to i64
                    builder.ins().bitcast(
                        cltypes::I64,
                        cranelift_codegen::ir::MemFlags::new(),
                        value_val,
                    )
                } else {
                    value_val
                };
                builder.ins().call(func_ref, &[var_id_i32, value_i64]);
            } else {
                compile_global_set(
                    builder,
                    global_set_func_name(*kind),
                    value_kind_to_cltype(*kind),
                    &args[0],
                    &args[1],
                    ctx,
                )?;
            }
        }

        // GlobalGet operations
        mir::RuntimeFunc::GlobalGet(kind) => {
            let needs_gc = matches!(kind, ValueKind::Ptr);
            compile_global_get(
                builder,
                global_get_func_name(*kind),
                value_kind_to_cltype(*kind),
                &args[0],
                dest,
                ctx,
                needs_gc, // Only pointer results need GC update
            )?;
        }

        _ => unreachable!("Non-global function passed to compile_global_call"),
    }

    Ok(())
}
