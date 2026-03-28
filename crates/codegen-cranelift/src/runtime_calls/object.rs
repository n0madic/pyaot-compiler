//! Object operations code generation (Union type dispatch)

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::utils::{declare_runtime_function, get_call_result, load_operand};

/// Compile an object-related runtime call (Union type dispatch)
pub fn compile_object_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::IsTruthy => {
            // rt_is_truthy(obj: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // obj pointer
            sig.returns.push(AbiParam::new(cltypes::I8)); // result (0 or 1)

            let func_id = declare_runtime_function(ctx.module, "rt_is_truthy", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let obj = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[obj]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::ObjContains => {
            // rt_obj_contains(a: *mut Obj, b: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // a
            sig.params.push(AbiParam::new(cltypes::I64)); // b
            sig.returns.push(AbiParam::new(cltypes::I8)); // result (0 or 1)

            let func_id = declare_runtime_function(ctx.module, "rt_obj_contains", &sig)?;
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
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::ObjToStr => {
            // rt_obj_to_str(obj: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // obj pointer
            sig.returns.push(AbiParam::new(cltypes::I64)); // string pointer

            let func_id = declare_runtime_function(ctx.module, "rt_obj_to_str", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let obj_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[obj_val]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::ObjDefaultRepr => {
            // rt_obj_default_repr(obj: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // obj pointer
            sig.returns.push(AbiParam::new(cltypes::I64)); // string pointer

            let func_id = declare_runtime_function(ctx.module, "rt_obj_default_repr", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let obj_val = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[obj_val]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        // Union arithmetic: rt_obj_{add,sub,mul,div,floordiv,mod,pow}(a, b) -> *mut Obj
        mir::RuntimeFunc::ObjAdd
        | mir::RuntimeFunc::ObjSub
        | mir::RuntimeFunc::ObjMul
        | mir::RuntimeFunc::ObjDiv
        | mir::RuntimeFunc::ObjFloorDiv
        | mir::RuntimeFunc::ObjMod
        | mir::RuntimeFunc::ObjPow => {
            let rt_name = match func {
                mir::RuntimeFunc::ObjAdd => "rt_obj_add",
                mir::RuntimeFunc::ObjSub => "rt_obj_sub",
                mir::RuntimeFunc::ObjMul => "rt_obj_mul",
                mir::RuntimeFunc::ObjDiv => "rt_obj_div",
                mir::RuntimeFunc::ObjFloorDiv => "rt_obj_floordiv",
                mir::RuntimeFunc::ObjMod => "rt_obj_mod",
                mir::RuntimeFunc::ObjPow => "rt_obj_pow",
                _ => unreachable!(),
            };
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // a
            sig.params.push(AbiParam::new(cltypes::I64)); // b
            sig.returns.push(AbiParam::new(cltypes::I64)); // result *mut Obj

            let func_id = declare_runtime_function(ctx.module, rt_name, &sig)?;
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
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::AnyGetItem => {
            // rt_any_getitem(obj: *mut Obj, index: i64) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // obj
            sig.params.push(AbiParam::new(cltypes::I64)); // index
            sig.returns.push(AbiParam::new(cltypes::I64)); // result

            let func_id = declare_runtime_function(ctx.module, "rt_any_getitem", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let obj_val = load_operand(builder, &args[0], ctx.var_map);
            let idx_val = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[obj_val, idx_val]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        _ => unreachable!("Non-object function passed to compile_object_call"),
    }

    Ok(())
}
