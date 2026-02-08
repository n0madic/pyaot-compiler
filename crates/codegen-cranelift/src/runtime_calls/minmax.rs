//! Container min/max operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{ContainerKind, ElementKind, MinMaxOp, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile a ContainerMinMax runtime call
pub fn compile_container_minmax(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    container: ContainerKind,
    op: MinMaxOp,
    elem: ElementKind,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Build runtime function name: rt_{container}_{op}_{elem}
    // e.g., rt_list_min_int, rt_tuple_max_float, rt_set_min_with_key
    let func_name = format!("rt_{}_{}{}", container.name(), op.name(), elem.suffix());

    // Build signature based on element kind
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    // All container min/max functions take container pointer as first arg
    sig.params.push(AbiParam::new(cltypes::I64)); // container

    // WithKey variant takes key function pointer and elem_tag as args
    if matches!(elem, ElementKind::WithKey) {
        sig.params.push(AbiParam::new(cltypes::I64)); // key_fn
        sig.params.push(AbiParam::new(cltypes::I64)); // elem_tag
    }

    // Return type depends on element kind
    let return_type = match elem {
        ElementKind::Int => cltypes::I64,
        ElementKind::Float => cltypes::F64,
        ElementKind::WithKey => cltypes::I64, // returns *mut Obj
    };
    sig.returns.push(AbiParam::new(return_type));

    let func_id = declare_runtime_function(ctx.module, &func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    // Load arguments
    let container_val = load_operand(builder, &args[0], ctx.var_map);

    let call_inst = if matches!(elem, ElementKind::WithKey) {
        let key_fn_val = load_operand(builder, &args[1], ctx.var_map);
        let elem_tag_val = load_operand(builder, &args[2], ctx.var_map);
        builder
            .ins()
            .call(func_ref, &[container_val, key_fn_val, elem_tag_val])
    } else {
        builder.ins().call(func_ref, &[container_val])
    };

    let result_val = get_call_result(builder, call_inst);
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);

    Ok(())
}
