//! Unified comparison operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{CompareKind, ComparisonOp, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile a unified comparison operation
/// Handles: list equality (int/float/str), tuple comparisons, string equality,
/// bytes equality, and object comparisons (for Union types)
pub fn compile_compare_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    kind: CompareKind,
    op: ComparisonOp,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // All comparison functions take two pointer args and return i8 (bool)
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // a
    sig.params.push(AbiParam::new(cltypes::I64)); // b
    sig.returns.push(AbiParam::new(cltypes::I8)); // result (0 or 1)

    // Get the runtime function name
    let func_name = kind.runtime_func_name(op);

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let a = load_operand(builder, &args[0], ctx.var_map);
    let b = load_operand(builder, &args[1], ctx.var_map);
    let call_inst = builder.ins().call(func_ref, &[a, b]);

    let result_val = get_call_result(builder, call_inst);
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);

    Ok(())
}
