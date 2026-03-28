//! Instance (class) operations code generation

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

/// Compile an instance (class)-related runtime call
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn compile_instance_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::MakeInstance => {
            // rt_make_instance(class_id: u8, field_count: i64) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.params.push(AbiParam::new(cltypes::I64)); // field_count
            sig.returns.push(AbiParam::new(cltypes::I64)); // *mut Obj

            let func_id = declare_runtime_function(ctx.module, "rt_make_instance", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            // Load args - class_id is i64 in MIR but needs to be i8 for runtime
            let class_id_raw = load_operand(builder, &args[0], ctx.var_map);
            let class_id = builder.ins().ireduce(cltypes::I8, class_id_raw);
            let field_count = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[class_id, field_count]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::InstanceGetField => {
            // rt_instance_get_field(inst: *mut Obj, offset: i64) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // inst
            sig.params.push(AbiParam::new(cltypes::I64)); // offset
            sig.returns.push(AbiParam::new(cltypes::I64)); // i64 (raw value or pointer)

            let func_id = declare_runtime_function(ctx.module, "rt_instance_get_field", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let inst = load_operand(builder, &args[0], ctx.var_map);
            let offset = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[inst, offset]);

            let result_val_i64 = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            // Convert i64 result to destination type:
            // - i8 (bool): reduce i64 to i8
            // - f64 (float): bitcast i64 to f64 (restore raw bits)
            // - i64: pass through
            let existing_val = builder.use_var(dest_var);
            let dest_ty = builder.func.dfg.value_type(existing_val);
            let result_val = if dest_ty == cltypes::I8 {
                builder.ins().ireduce(cltypes::I8, result_val_i64)
            } else if dest_ty == cltypes::F64 {
                builder.ins().bitcast(
                    cltypes::F64,
                    cranelift_codegen::ir::MemFlags::new(),
                    result_val_i64,
                )
            } else {
                result_val_i64
            };
            builder.def_var(dest_var, result_val);
            // Only update GC root if field type is a heap type (handled in lowering)
            update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
        }
        mir::RuntimeFunc::InstanceSetField => {
            // rt_instance_set_field(inst: *mut Obj, offset: i64, value: i64)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // inst
            sig.params.push(AbiParam::new(cltypes::I64)); // offset
            sig.params.push(AbiParam::new(cltypes::I64)); // value (raw i64)

            let func_id = declare_runtime_function(ctx.module, "rt_instance_set_field", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let inst = load_operand(builder, &args[0], ctx.var_map);
            let offset = load_operand(builder, &args[1], ctx.var_map);
            let value_raw = load_operand(builder, &args[2], ctx.var_map);
            let value_ty = builder.func.dfg.value_type(value_raw);
            // Convert values to i64 for storage:
            // - i8 (bool/none): zero-extend to i64
            // - f64 (float): bitcast to i64 (preserves raw bits)
            // - i64: pass through
            let value = if value_ty == cltypes::I8 {
                builder.ins().uextend(cltypes::I64, value_raw)
            } else if value_ty == cltypes::F64 {
                builder.ins().bitcast(
                    cltypes::I64,
                    cranelift_codegen::ir::MemFlags::new(),
                    value_raw,
                )
            } else {
                value_raw
            };
            builder.ins().call(func_ref, &[inst, offset, value]);
        }
        mir::RuntimeFunc::GetTypeTag => {
            // rt_get_type_tag(obj: *mut Obj) -> i64
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // obj pointer
            sig.returns.push(AbiParam::new(cltypes::I64)); // type tag as i64

            let func_id = declare_runtime_function(ctx.module, "rt_get_type_tag", &sig)?;
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
        mir::RuntimeFunc::IsinstanceClass => {
            // rt_isinstance_class(obj: *mut Obj, class_id: i64) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // obj pointer
            sig.params.push(AbiParam::new(cltypes::I64)); // class_id
            sig.returns.push(AbiParam::new(cltypes::I8)); // result (0 or 1)

            let func_id = declare_runtime_function(ctx.module, "rt_isinstance_class", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let obj = load_operand(builder, &args[0], ctx.var_map);
            let class_id = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[obj, class_id]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::IsinstanceClassInherited => {
            // rt_isinstance_class_inherited(obj: *mut Obj, target_class_id: i64) -> i8
            // This version walks the parent chain to check inheritance
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // obj pointer
            sig.params.push(AbiParam::new(cltypes::I64)); // target_class_id
            sig.returns.push(AbiParam::new(cltypes::I8)); // result (0 or 1)

            let func_id =
                declare_runtime_function(ctx.module, "rt_isinstance_class_inherited", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let obj = load_operand(builder, &args[0], ctx.var_map);
            let class_id = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[obj, class_id]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::RegisterClass => {
            // rt_register_class(class_id: u8, parent_class_id: u8)
            // Register class inheritance information at module init
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.params.push(AbiParam::new(cltypes::I8)); // parent_class_id

            let func_id = declare_runtime_function(ctx.module, "rt_register_class", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            // Load args - they are i64 in MIR but need to be i8 for runtime
            let class_id_raw = load_operand(builder, &args[0], ctx.var_map);
            let class_id = builder.ins().ireduce(cltypes::I8, class_id_raw);
            let parent_id_raw = load_operand(builder, &args[1], ctx.var_map);
            let parent_id = builder.ins().ireduce(cltypes::I8, parent_id_raw);
            builder.ins().call(func_ref, &[class_id, parent_id]);

            // This function returns void, so just store a dummy value in dest
            let zero = builder.ins().iconst(cltypes::I64, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }
        mir::RuntimeFunc::RegisterClassFields => {
            // rt_register_class_fields(class_id: u8, heap_field_mask: i64)
            // Register which fields are heap objects for GC tracing
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.params.push(AbiParam::new(cltypes::I64)); // heap_field_mask

            let func_id = declare_runtime_function(ctx.module, "rt_register_class_fields", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let class_id_raw = load_operand(builder, &args[0], ctx.var_map);
            let class_id = builder.ins().ireduce(cltypes::I8, class_id_raw);
            let mask = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[class_id, mask]);

            // Void return — store a dummy value
            let zero = builder.ins().iconst(cltypes::I64, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }
        mir::RuntimeFunc::RegisterMethodName => {
            // rt_register_method_name(class_id: i64, name_hash: i64, slot: i64)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // class_id
            sig.params.push(AbiParam::new(cltypes::I64)); // name_hash
            sig.params.push(AbiParam::new(cltypes::I64)); // slot

            let func_id = declare_runtime_function(ctx.module, "rt_register_method_name", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let class_id = load_operand(builder, &args[0], ctx.var_map);
            let name_hash = load_operand(builder, &args[1], ctx.var_map);
            let slot = load_operand(builder, &args[2], ctx.var_map);
            builder.ins().call(func_ref, &[class_id, name_hash, slot]);

            // Void return
            let zero = builder.ins().iconst(cltypes::I64, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }
        mir::RuntimeFunc::IsSubclass => {
            // rt_issubclass(child_tag: i64, parent_tag: i64) -> i8
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // child_tag
            sig.params.push(AbiParam::new(cltypes::I64)); // parent_tag
            sig.returns.push(AbiParam::new(cltypes::I8)); // result (0 or 1)

            let func_id = declare_runtime_function(ctx.module, "rt_issubclass", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let child_tag = load_operand(builder, &args[0], ctx.var_map);
            let parent_tag = load_operand(builder, &args[1], ctx.var_map);
            let call_inst = builder.ins().call(func_ref, &[child_tag, parent_tag]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::RegisterClassFieldCount => {
            // rt_register_class_field_count(class_id: u8, field_count: i64)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.params.push(AbiParam::new(cltypes::I64)); // field_count

            let func_id =
                declare_runtime_function(ctx.module, "rt_register_class_field_count", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let class_id_raw = load_operand(builder, &args[0], ctx.var_map);
            let class_id = builder.ins().ireduce(cltypes::I8, class_id_raw);
            let field_count = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[class_id, field_count]);

            let zero = builder.ins().iconst(cltypes::I64, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }
        mir::RuntimeFunc::ObjectNew => {
            // rt_object_new(class_id: u8) -> *mut Obj
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.returns.push(AbiParam::new(cltypes::I64)); // instance ptr

            let func_id = declare_runtime_function(ctx.module, "rt_object_new", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let class_id_raw = load_operand(builder, &args[0], ctx.var_map);
            let class_id = builder.ins().ireduce(cltypes::I8, class_id_raw);
            let call_inst = builder.ins().call(func_ref, &[class_id]);

            let result_val = get_call_result(builder, call_inst);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, result_val);
        }
        mir::RuntimeFunc::RegisterDelFunc
        | mir::RuntimeFunc::RegisterCopyFunc
        | mir::RuntimeFunc::RegisterDeepCopyFunc => {
            // rt_register_{del,copy,deepcopy}_func(class_id: u8, func_ptr: *const u8)
            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I8)); // class_id
            sig.params.push(AbiParam::new(cltypes::I64)); // func_ptr

            let rt_name = match func {
                mir::RuntimeFunc::RegisterDelFunc => "rt_register_del_func",
                mir::RuntimeFunc::RegisterCopyFunc => "rt_register_copy_func",
                mir::RuntimeFunc::RegisterDeepCopyFunc => "rt_register_deepcopy_func",
                _ => unreachable!(),
            };

            let func_id = declare_runtime_function(ctx.module, rt_name, &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

            let class_id_raw = load_operand(builder, &args[0], ctx.var_map);
            let class_id = builder.ins().ireduce(cltypes::I8, class_id_raw);
            let func_ptr = load_operand(builder, &args[1], ctx.var_map);
            builder.ins().call(func_ref, &[class_id, func_ptr]);

            let zero = builder.ins().iconst(cltypes::I64, 0);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zero);
        }
        _ => unreachable!("Non-instance function passed to compile_instance_call"),
    }

    Ok(())
}
