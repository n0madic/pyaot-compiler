//! Iterator operations code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::gc::update_gc_root_if_needed;
use crate::runtime_helpers::{
    compile_binary_runtime_call, compile_quaternary_runtime_call, compile_quinary_runtime_call,
    compile_senary_runtime_call, compile_ternary_runtime_call, compile_unary_runtime_call,
};
use crate::utils::{declare_runtime_function, load_operand, load_operand_as};

/// Compile an iterator-related runtime call
pub fn compile_iterator_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        // Unified iterator creation
        mir::RuntimeFunc::MakeIterator { source, direction } => {
            // Build runtime function name: rt_iter_{reversed_}{source}
            let func_name = format!("rt_iter_{}{}", direction.prefix(), source.name());

            // Range requires 3 args (start, stop, step), others require 1 arg (container)
            if source.requires_range_args() {
                compile_ternary_runtime_call(
                    builder,
                    &func_name,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    &args[0],
                    &args[1],
                    &args[2],
                    dest,
                    ctx,
                    true,
                )?;
            } else {
                compile_unary_runtime_call(
                    builder,
                    &func_name,
                    cltypes::I64,
                    cltypes::I64,
                    &args[0],
                    dest,
                    ctx,
                    true,
                )?;
            }
        }

        // Iterator operations
        mir::RuntimeFunc::IterNext => {
            compile_unary_runtime_call(
                builder,
                "rt_iter_next",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IterNextNoExc => {
            compile_unary_runtime_call(
                builder,
                "rt_iter_next_no_exc",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IterIsExhausted => {
            compile_unary_runtime_call(
                builder,
                "rt_iter_is_exhausted",
                cltypes::I64,
                cltypes::I8,
                &args[0],
                dest,
                ctx,
                false,
            )?;
        }
        mir::RuntimeFunc::IterEnumerate => {
            compile_binary_runtime_call(
                builder,
                "rt_iter_enumerate",
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

        // Unified sorted operations
        mir::RuntimeFunc::Sorted { source, has_key } => {
            if source.is_range() {
                // Range: rt_sorted_range(start, stop, step, reverse) -> *Obj
                compile_quaternary_runtime_call(
                    builder,
                    "rt_sorted_range",
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    &args[0],
                    &args[1],
                    &args[2],
                    &args[3],
                    dest,
                    ctx,
                    true,
                )?;
            } else if *has_key {
                // With key: rt_sorted_{source}_with_key(container, reverse, key_fn, elem_tag) -> *Obj
                let func_name = format!("rt_sorted_{}_with_key", source.name());
                compile_quaternary_runtime_call(
                    builder,
                    &func_name,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    cltypes::I64,
                    &args[0],
                    &args[1],
                    &args[2],
                    &args[3],
                    dest,
                    ctx,
                    true,
                )?;
            } else if matches!(source, mir::SortableKind::Set | mir::SortableKind::Dict) {
                // Set/Dict without key: rt_sorted_{source}(container, reverse, elem_tag) -> *Obj
                let func_name = format!("rt_sorted_{}", source.name());
                compile_sorted_with_elem_tag(
                    builder, &func_name, &args[0], &args[1], &args[2], dest, ctx,
                )?;
            } else {
                // Without key: rt_sorted_{source}(container, reverse) -> *Obj
                let func_name = format!("rt_sorted_{}", source.name());
                compile_binary_runtime_call(
                    builder,
                    &func_name,
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
        }

        // Zip operations
        mir::RuntimeFunc::ZipNew => {
            compile_binary_runtime_call(
                builder,
                "rt_zip_new",
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
        mir::RuntimeFunc::ZipNext => {
            compile_unary_runtime_call(
                builder,
                "rt_zip_next",
                cltypes::I64,
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::IterZip => {
            // Zip objects are already iterators - return the object itself
            let zip = load_operand(builder, &args[0], ctx.var_map);
            let dest_var = *ctx
                .var_map
                .get(&dest)
                .expect("internal error: local not in var_map - codegen bug");
            builder.def_var(dest_var, zip);
            update_gc_root_if_needed(builder, &dest, zip, ctx.gc_frame_data);
        }

        // Map/Filter operations (with captures support)
        mir::RuntimeFunc::MapNew => {
            // rt_map_new(func_ptr: i64, iter: *mut Obj, captures: *mut Obj, capture_count: i64) -> *mut Obj
            compile_quaternary_runtime_call(
                builder,
                "rt_map_new",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                &args[3],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::FilterNew => {
            // rt_filter_new(func_ptr: i64, iter: *mut Obj, elem_tag: i64, captures: *mut Obj, capture_count: i64) -> *mut Obj
            compile_quinary_runtime_call(
                builder,
                "rt_filter_new",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                &args[3],
                &args[4],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::ReduceNew => {
            // rt_reduce(func_ptr, iter, initial, has_initial, captures, capture_count) -> *mut Obj
            compile_senary_runtime_call(
                builder,
                "rt_reduce",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                &args[3],
                &args[4],
                &args[5],
                dest,
                ctx,
                true,
            )?;
        }

        mir::RuntimeFunc::ChainNew => {
            // rt_chain_new(iters: *mut Obj, num_iters: i64) -> *mut Obj
            compile_binary_runtime_call(
                builder,
                "rt_chain_new",
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
        mir::RuntimeFunc::ISliceNew => {
            // rt_islice_new(iter: *mut Obj, start: i64, stop: i64, step: i64) -> *mut Obj
            compile_quaternary_runtime_call(
                builder,
                "rt_islice_new",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                &args[3],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::Zip3New => {
            compile_ternary_runtime_call(
                builder,
                "rt_zip3_new",
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                &args[0],
                &args[1],
                &args[2],
                dest,
                ctx,
                true,
            )?;
        }
        mir::RuntimeFunc::ZipNNew => {
            compile_binary_runtime_call(
                builder,
                "rt_zipn_new",
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

        _ => unreachable!("Non-iterator function passed to compile_iterator_call"),
    }

    Ok(())
}

/// Compile sorted set/dict calls with elem_tag parameter.
/// Signature: fn(container: *mut Obj, reverse: i64, elem_tag: u8) -> *mut Obj
fn compile_sorted_with_elem_tag(
    builder: &mut FunctionBuilder,
    func_name: &str,
    container_arg: &Operand,
    reverse_arg: &Operand,
    elem_tag_arg: &Operand,
    dest: LocalId,
    ctx: &mut CodegenContext,
) -> Result<()> {
    use cranelift_codegen::ir::{AbiParam, InstBuilder};
    use cranelift_codegen::isa::CallConv;
    use cranelift_module::Module;

    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;
    sig.params.push(AbiParam::new(cltypes::I64)); // container
    sig.params.push(AbiParam::new(cltypes::I64)); // reverse
    sig.params.push(AbiParam::new(cltypes::I8)); // elem_tag
    sig.returns.push(AbiParam::new(cltypes::I64)); // -> *mut Obj

    let func_id = declare_runtime_function(ctx.module, func_name, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    let container = load_operand_as(builder, container_arg, ctx.var_map, cltypes::I64);
    let reverse = load_operand_as(builder, reverse_arg, ctx.var_map, cltypes::I64);
    let elem_tag_i64 = load_operand(builder, elem_tag_arg, ctx.var_map);
    let elem_tag = builder.ins().ireduce(cltypes::I8, elem_tag_i64);
    let call_inst = builder
        .ins()
        .call(func_ref, &[container, reverse, elem_tag]);

    let result_val = builder.inst_results(call_inst)[0];
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: dest local not in var_map - codegen bug");
    builder.def_var(dest_var, result_val);

    update_gc_root_if_needed(builder, &dest, result_val, ctx.gc_frame_data);
    Ok(())
}
