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

/// Compile a ContainerMinMax runtime call.
///
/// Int/Float use a unified `rt_{container}_minmax(container, is_min, elem_kind) -> i64`.
/// For Float, the i64 result is bitcast to f64.
/// WithKey uses `rt_{container}_minmax_with_key(container, key_fn, elem_tag, captures, count, is_min) -> *mut Obj`.
pub fn compile_container_minmax(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    container: ContainerKind,
    op: MinMaxOp,
    elem: ElementKind,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    let container_val = load_operand(builder, &args[0], ctx.var_map);
    let is_min_val = builder.ins().iconst(cltypes::I8, op.to_tag() as i64);

    let result_val = match elem {
        ElementKind::Int | ElementKind::Float => {
            // Unified: rt_{container}_minmax(container, is_min, elem_kind) -> i64
            let func_name = format!("rt_{}_minmax", container.name());
            let elem_kind: u8 = if matches!(elem, ElementKind::Float) {
                1
            } else {
                0
            };

            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // container
            sig.params.push(AbiParam::new(cltypes::I8)); // is_min
            sig.params.push(AbiParam::new(cltypes::I8)); // elem_kind
            sig.returns.push(AbiParam::new(cltypes::I64)); // result (i64 or f64-as-bits)

            let func_id = declare_runtime_function(ctx.module, &func_name, &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let elem_kind_val = builder.ins().iconst(cltypes::I8, elem_kind as i64);
            let call_inst = builder
                .ins()
                .call(func_ref, &[container_val, is_min_val, elem_kind_val]);
            let raw = get_call_result(builder, call_inst);

            if matches!(elem, ElementKind::Float) {
                builder
                    .ins()
                    .bitcast(cltypes::F64, cranelift_codegen::ir::MemFlags::new(), raw)
            } else {
                raw
            }
        }
        ElementKind::WithKey => {
            // rt_{container}_minmax_with_key(container, key_fn, elem_tag, captures, count, is_min) -> *mut Obj
            let func_name = format!("rt_{}_minmax_with_key", container.name());

            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // container
            sig.params.push(AbiParam::new(cltypes::I64)); // key_fn
            sig.params.push(AbiParam::new(cltypes::I64)); // elem_tag
            sig.params.push(AbiParam::new(cltypes::I64)); // captures
            sig.params.push(AbiParam::new(cltypes::I64)); // capture_count
            sig.params.push(AbiParam::new(cltypes::I8)); // is_min
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, &func_name, &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let key_fn_val = load_operand(builder, &args[1], ctx.var_map);
            let elem_tag_val = load_operand(builder, &args[2], ctx.var_map);
            let captures_val = load_operand(builder, &args[3], ctx.var_map);
            let capture_count_val = load_operand(builder, &args[4], ctx.var_map);
            let call_inst = builder.ins().call(
                func_ref,
                &[
                    container_val,
                    key_fn_val,
                    elem_tag_val,
                    captures_val,
                    capture_count_val,
                    is_min_val,
                ],
            );
            get_call_result(builder, call_inst)
        }
    };

    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);

    Ok(())
}
