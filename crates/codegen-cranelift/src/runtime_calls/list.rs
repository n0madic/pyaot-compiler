//! List operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::InstBuilder;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;

use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::runtime_helpers::{
    compile_binary_runtime_call, compile_container_copy, compile_container_get,
    compile_container_len, compile_container_void_method, compile_list_binary_to_i64,
    compile_list_binary_to_i8, compile_make_container_with_tag, compile_slice3, compile_slice4,
    compile_ternary_runtime_call, compile_unary_runtime_call, compile_void_runtime_call,
};

/// Compile a list-related runtime call
pub fn compile_list_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::MakeList => {
            compile_make_container_with_tag(
                builder,
                "rt_make_list",
                &args[0],
                &args[1],
                dest,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListPush => {
            compile_void_runtime_call(
                builder,
                "rt_list_push",
                &[cltypes::I64, cltypes::I64],
                args,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListSet => {
            compile_void_runtime_call(
                builder,
                "rt_list_set",
                &[cltypes::I64, cltypes::I64, cltypes::I64],
                args,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListGet => {
            compile_container_get(builder, "rt_list_get", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::ListGetTyped(elem) => {
            let func_name = format!("rt_list_get{}", elem.suffix());
            match elem {
                mir::GetElementKind::Int => {
                    compile_list_binary_to_i64(builder, &func_name, &args[0], &args[1], dest, ctx)?;
                }
                mir::GetElementKind::Float => {
                    use crate::utils::load_operand;
                    use cranelift_codegen::ir::AbiParam;
                    use cranelift_codegen::isa::CallConv;
                    use cranelift_module::{Linkage, Module};

                    let mut sig = ctx.module.make_signature();
                    sig.call_conv = CallConv::SystemV;
                    sig.params.push(AbiParam::new(cltypes::I64)); // list
                    sig.params.push(AbiParam::new(cltypes::I64)); // index
                    sig.returns.push(AbiParam::new(cltypes::F64)); // result

                    let func_id = ctx
                        .module
                        .declare_function(&func_name, Linkage::Import, &sig)
                        .expect("failed to declare rt_list_get_float function");
                    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

                    let list_val = load_operand(builder, &args[0], ctx.var_map);
                    let index_val = load_operand(builder, &args[1], ctx.var_map);
                    let call_inst = builder.ins().call(func_ref, &[list_val, index_val]);
                    let result_val = builder.inst_results(call_inst)[0];

                    let dest_var = *ctx
                        .var_map
                        .get(&dest)
                        .expect("internal error: local not in var_map - codegen bug");
                    builder.def_var(dest_var, result_val);
                }
                mir::GetElementKind::Bool => {
                    compile_list_binary_to_i8(builder, &func_name, &args[0], &args[1], dest, ctx)?;
                }
            }
        }
        mir::RuntimeFunc::ListLen => {
            compile_container_len(builder, "rt_list_len", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::ListSlice => {
            compile_slice3(builder, "rt_list_slice", args, dest, ctx)?;
        }
        mir::RuntimeFunc::ListSliceStep => {
            compile_slice4(builder, "rt_list_slice_step", args, dest, ctx)?;
        }
        mir::RuntimeFunc::ListTailToTuple => {
            compile_container_get(
                builder,
                "rt_list_tail_to_tuple",
                &args[0],
                &args[1],
                dest,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListTailToTupleFloat => {
            compile_container_get(
                builder,
                "rt_list_tail_to_tuple_float",
                &args[0],
                &args[1],
                dest,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListTailToTupleBool => {
            compile_container_get(
                builder,
                "rt_list_tail_to_tuple_bool",
                &args[0],
                &args[1],
                dest,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListAppend => {
            compile_void_runtime_call(
                builder,
                "rt_list_append",
                &[cltypes::I64, cltypes::I64],
                args,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListSetElemTag => {
            compile_void_runtime_call(
                builder,
                "rt_list_set_elem_tag",
                &[cltypes::I64, cltypes::I8],
                args,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListPop => {
            compile_container_get(builder, "rt_list_pop", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::ListInsert => {
            compile_void_runtime_call(
                builder,
                "rt_list_insert",
                &[cltypes::I64, cltypes::I64, cltypes::I64],
                args,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListRemove => {
            // Has return value but ignored for Python semantics
            compile_binary_runtime_call(
                builder,
                "rt_list_remove",
                cltypes::I64,
                cltypes::I64,
                cltypes::I8,
                &args[0],
                &args[1],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::ListClear => {
            compile_container_void_method(builder, "rt_list_clear", &args[0], ctx)?;
        }
        mir::RuntimeFunc::ListIndex => {
            compile_list_binary_to_i64(builder, "rt_list_index", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::ListCount => {
            compile_list_binary_to_i64(builder, "rt_list_count", &args[0], &args[1], dest, ctx)?;
        }
        mir::RuntimeFunc::ListCopy => {
            compile_container_copy(builder, "rt_list_copy", &args[0], dest, ctx)?;
        }
        mir::RuntimeFunc::ListReverse => {
            compile_container_void_method(builder, "rt_list_reverse", &args[0], ctx)?;
        }
        mir::RuntimeFunc::ListExtend => {
            compile_void_runtime_call(
                builder,
                "rt_list_extend",
                &[cltypes::I64, cltypes::I64],
                args,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListSort => {
            // Sort returns None, but has i8 reverse arg
            compile_void_runtime_call(
                builder,
                "rt_list_sort",
                &[cltypes::I64, cltypes::I8],
                args,
                ctx,
            )?;
            // Return None
            let none_val = builder.ins().iconst(cltypes::I8, 0);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, none_val);
        }
        mir::RuntimeFunc::ListSortWithKey => {
            // Sort with key: rt_list_sort_with_key(list, reverse, key_fn, elem_tag, captures, capture_count)
            compile_void_runtime_call(
                builder,
                "rt_list_sort_with_key",
                &[
                    cltypes::I64,
                    cltypes::I8,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                ],
                args,
                ctx,
            )?;
            // Return None
            let none_val = builder.ins().iconst(cltypes::I8, 0);
            let var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(var, none_val);
        }
        // List from other types (all: *Obj -> *Obj)
        mir::RuntimeFunc::ListFromTuple => {
            compile_unary_runtime_call(
                builder,
                "rt_list_from_tuple",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::ListFromStr => {
            compile_unary_runtime_call(
                builder,
                "rt_list_from_str",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::ListFromRange => {
            compile_ternary_runtime_call(
                builder,
                "rt_list_from_range",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::ListFromIter => {
            compile_binary_runtime_call(
                builder,
                "rt_list_from_iter",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::ListFromSet => {
            compile_unary_runtime_call(
                builder,
                "rt_list_from_set",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::ListFromDict => {
            compile_unary_runtime_call(
                builder,
                "rt_list_from_dict",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::ListSliceAssign => {
            // rt_list_slice_assign(list: *mut Obj, start: i64, stop: i64, values: *mut Obj) - void
            compile_void_runtime_call(
                builder,
                "rt_list_slice_assign",
                &[cltypes::I64, cltypes::I64, cltypes::I64, cltypes::I64],
                args,
                ctx,
            )?;
        }
        mir::RuntimeFunc::ListConcat => {
            // rt_list_concat(list1: *mut Obj, list2: *mut Obj) -> *mut Obj
            compile_binary_runtime_call(
                builder,
                "rt_list_concat",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                dest,
                ctx,
                true,
            )?;
        }
        _ => unreachable!("Non-list function passed to compile_list_call"),
    }

    Ok(())
}
