//! Set operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{Linkage, Module};
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::runtime_helpers::{
    compile_container_copy, compile_container_len, compile_container_void_method,
    compile_make_container, compile_unary_runtime_call,
};
use crate::utils::{declare_runtime_function, load_operand};

/// Compile a set-related runtime call
pub fn compile_set_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::MakeSet => {
            compile_make_container(builder, "rt_make_set", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::SetAdd => {
            // rt_set_add(set: *mut Obj, elem: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // elem

            let func_id = ctx
                .module
                .declare_function("rt_set_add", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_add");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let elem = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[set, elem]);
        }
        mir::RuntimeFunc::SetContains => {
            // rt_set_contains(set: *mut Obj, elem: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // elem
            sig.returns.push(AbiParam::new(cltypes::I8)); // result

            let func_id = ctx
                .module
                .declare_function("rt_set_contains", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_contains");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let elem = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[set, elem]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetRemove => {
            // rt_set_remove(set: *mut Obj, elem: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // elem

            let func_id = ctx
                .module
                .declare_function("rt_set_remove", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_remove");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let elem = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[set, elem]);
        }
        mir::RuntimeFunc::SetDiscard => {
            // rt_set_discard(set: *mut Obj, elem: *mut Obj)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // elem

            let func_id = ctx
                .module
                .declare_function("rt_set_discard", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_discard");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let elem = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[set, elem]);
        }
        mir::RuntimeFunc::SetLen => {
            compile_container_len(builder, "rt_set_len", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::SetClear => {
            compile_container_void_method(builder, "rt_set_clear", &args[0], ctx)?;
        }
        mir::RuntimeFunc::SetCopy => {
            compile_container_copy(builder, "rt_set_copy", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::SetToList => {
            // rt_set_to_list(set: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set pointer
            sig.returns.push(AbiParam::new(cltypes::I64)); // list pointer

            let func_id = ctx
                .module
                .declare_function("rt_set_to_list", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_to_list");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[set]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetUnion => {
            // rt_set_union(a: *mut Obj, b: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set a
            sig.params.push(AbiParam::new(cltypes::I64)); // set b
            sig.returns.push(AbiParam::new(cltypes::I64)); // result set

            let func_id = ctx
                .module
                .declare_function("rt_set_union", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_union");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let a = load_operand(builder, &args[0], ctx.var_map);
            let b = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[a, b]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetIntersection => {
            // rt_set_intersection(a: *mut Obj, b: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set a
            sig.params.push(AbiParam::new(cltypes::I64)); // set b
            sig.returns.push(AbiParam::new(cltypes::I64)); // result set

            let func_id = ctx
                .module
                .declare_function("rt_set_intersection", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_intersection");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let a = load_operand(builder, &args[0], ctx.var_map);
            let b = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[a, b]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetDifference => {
            // rt_set_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set a
            sig.params.push(AbiParam::new(cltypes::I64)); // set b
            sig.returns.push(AbiParam::new(cltypes::I64)); // result set

            let func_id = ctx
                .module
                .declare_function("rt_set_difference", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_difference");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let a = load_operand(builder, &args[0], ctx.var_map);
            let b = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[a, b]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetSymmetricDifference => {
            // rt_set_symmetric_difference(a: *mut Obj, b: *mut Obj) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set a
            sig.params.push(AbiParam::new(cltypes::I64)); // set b
            sig.returns.push(AbiParam::new(cltypes::I64)); // result set

            let func_id = ctx
                .module
                .declare_function("rt_set_symmetric_difference", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_symmetric_difference");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let a = load_operand(builder, &args[0], ctx.var_map);
            let b = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[a, b]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetIssubset => {
            // rt_set_issubset(a: *mut Obj, b: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set a
            sig.params.push(AbiParam::new(cltypes::I64)); // set b
            sig.returns.push(AbiParam::new(cltypes::I8)); // result

            let func_id = ctx
                .module
                .declare_function("rt_set_issubset", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_issubset");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let a = load_operand(builder, &args[0], ctx.var_map);
            let b = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[a, b]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetIssuperset => {
            // rt_set_issuperset(a: *mut Obj, b: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set a
            sig.params.push(AbiParam::new(cltypes::I64)); // set b
            sig.returns.push(AbiParam::new(cltypes::I8)); // result

            let func_id = ctx
                .module
                .declare_function("rt_set_issuperset", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_issuperset");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let a = load_operand(builder, &args[0], ctx.var_map);
            let b = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[a, b]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetIsdisjoint => {
            // rt_set_isdisjoint(a: *mut Obj, b: *mut Obj) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set a
            sig.params.push(AbiParam::new(cltypes::I64)); // set b
            sig.returns.push(AbiParam::new(cltypes::I8)); // result

            let func_id = ctx
                .module
                .declare_function("rt_set_isdisjoint", Linkage::Import, &sig)
                .expect("Failed to declare rt_set_isdisjoint");
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let a = load_operand(builder, &args[0], ctx.var_map);
            let b = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[a, b]);

            let result_val = *builder
                .inst_results(call_inst)
                .first()
                .expect("internal error: call instruction should have return value");
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::SetPop => {
            compile_unary_runtime_call(
                builder,
                "rt_set_pop",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::SetUpdate => {
            // rt_set_update(set: *mut Obj, other: *mut Obj) - void function
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // other

            let func_id = declare_runtime_function(ctx.module, "rt_set_update", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let other = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[set, other]);
        }
        mir::RuntimeFunc::SetIntersectionUpdate => {
            // rt_set_intersection_update(set: *mut Obj, other: *mut Obj) - void function
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // other

            let func_id = declare_runtime_function(ctx.module, "rt_set_intersection_update", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let other = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[set, other]);
        }
        mir::RuntimeFunc::SetDifferenceUpdate => {
            // rt_set_difference_update(set: *mut Obj, other: *mut Obj) - void function
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // other

            let func_id = declare_runtime_function(ctx.module, "rt_set_difference_update", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let other = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[set, other]);
        }
        mir::RuntimeFunc::SetSymmetricDifferenceUpdate => {
            // rt_set_symmetric_difference_update(set: *mut Obj, other: *mut Obj) - void function
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // set
            sig.params.push(AbiParam::new(cltypes::I64)); // other

            let func_id =
                declare_runtime_function(ctx.module, "rt_set_symmetric_difference_update", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let set = load_operand(builder, &args[0], ctx.var_map);
            let other = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[set, other]);
        }
        _ => unreachable!("Non-set function passed to compile_set_call"),
    }

    Ok(())
}
