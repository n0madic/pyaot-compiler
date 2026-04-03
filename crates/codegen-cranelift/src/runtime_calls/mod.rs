//! Runtime function call code generation
//!
//! This module handles code generation for all RuntimeCall instructions,
//! including print functions, string operations, list operations, tuple
//! operations, dictionary operations, and type conversions.

mod cells;
mod class_attrs;
mod compare;
mod conversions;
mod generator;
mod globals;
mod instance;
mod iterator;
mod math;
mod minmax;
mod print;
mod stdlib;
mod string;

use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;

/// Compile a RuntimeCall instruction
pub fn compile_runtime_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        // Print operations
        mir::RuntimeFunc::AssertFail
        | mir::RuntimeFunc::AssertFailObj
        | mir::RuntimeFunc::PrintValue(_)
        | mir::RuntimeFunc::PrintNewline
        | mir::RuntimeFunc::PrintSep
        | mir::RuntimeFunc::Input
        | mir::RuntimeFunc::PrintSetStderr
        | mir::RuntimeFunc::PrintSetStdout
        | mir::RuntimeFunc::PrintFlush => {
            print::compile_print_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // String operations
        mir::RuntimeFunc::MakeStr
        | mir::RuntimeFunc::StrData
        | mir::RuntimeFunc::StrLen
        | mir::RuntimeFunc::StrLenInt
        | mir::RuntimeFunc::StrConcat
        | mir::RuntimeFunc::StrSlice
        | mir::RuntimeFunc::StrSliceStep
        | mir::RuntimeFunc::StrGetChar
        | mir::RuntimeFunc::StrSubscript
        | mir::RuntimeFunc::StrMul
        | mir::RuntimeFunc::StrUpper
        | mir::RuntimeFunc::StrLower
        | mir::RuntimeFunc::StrStrip
        | mir::RuntimeFunc::StrStartsWith
        | mir::RuntimeFunc::StrEndsWith
        | mir::RuntimeFunc::StrSearch(_)
        | mir::RuntimeFunc::StrReplace
        // New string methods
        | mir::RuntimeFunc::StrCount
        | mir::RuntimeFunc::StrSplit
        | mir::RuntimeFunc::StrJoin
        | mir::RuntimeFunc::StrLstrip
        | mir::RuntimeFunc::StrRstrip
        | mir::RuntimeFunc::StrTitle
        | mir::RuntimeFunc::StrCapitalize
        | mir::RuntimeFunc::StrSwapcase
        | mir::RuntimeFunc::StrCenter
        | mir::RuntimeFunc::StrLjust
        | mir::RuntimeFunc::StrRjust
        | mir::RuntimeFunc::StrZfill
        | mir::RuntimeFunc::StrIsDigit
        | mir::RuntimeFunc::StrIsAlpha
        | mir::RuntimeFunc::StrIsAlnum
        | mir::RuntimeFunc::StrIsSpace
        | mir::RuntimeFunc::StrIsUpper
        | mir::RuntimeFunc::StrIsLower
        | mir::RuntimeFunc::StrRemovePrefix
        | mir::RuntimeFunc::StrRemoveSuffix
        | mir::RuntimeFunc::StrSplitLines
        | mir::RuntimeFunc::StrPartition
        | mir::RuntimeFunc::StrRpartition
        | mir::RuntimeFunc::StrExpandTabs
        | mir::RuntimeFunc::StrRsplit
        | mir::RuntimeFunc::StrIsAscii
        | mir::RuntimeFunc::StrEncode
        // StringBuilder for efficient string concatenation
        | mir::RuntimeFunc::MakeStringBuilder
        | mir::RuntimeFunc::StringBuilderAppend
        | mir::RuntimeFunc::StringBuilderToStr
        | mir::RuntimeFunc::MakeBytes => {
            string::compile_string_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // List, Tuple, Dict: migrated to RuntimeFunc::Call (handled by generic handler)

        // Boxing/Unboxing: migrated to RuntimeFunc::Call (handled by generic handler)

        // Type conversion operations
        mir::RuntimeFunc::Convert { .. }
        | mir::RuntimeFunc::StrContains
        | mir::RuntimeFunc::IntToBin
        | mir::RuntimeFunc::IntToHex
        | mir::RuntimeFunc::IntToOct
        | mir::RuntimeFunc::IntFmtBin
        | mir::RuntimeFunc::IntFmtHex
        | mir::RuntimeFunc::IntFmtHexUpper
        | mir::RuntimeFunc::IntFmtOct
        | mir::RuntimeFunc::IntFmtGrouped
        | mir::RuntimeFunc::FloatFmtGrouped
        | mir::RuntimeFunc::ToStringRepr(_, _)
        | mir::RuntimeFunc::TypeName
        | mir::RuntimeFunc::TypeNameExtract
        | mir::RuntimeFunc::ExcClassName
        | mir::RuntimeFunc::FormatValue
        | mir::RuntimeFunc::StrToIntWithBase => {
            conversions::compile_conversion_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Math operations
        mir::RuntimeFunc::PowFloat
        | mir::RuntimeFunc::PowInt
        | mir::RuntimeFunc::RoundToInt
        | mir::RuntimeFunc::RoundToDigits
        | mir::RuntimeFunc::IntToChr
        | mir::RuntimeFunc::ChrToInt => {
            math::compile_math_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Instance (class) operations
        mir::RuntimeFunc::MakeInstance
        | mir::RuntimeFunc::InstanceGetField
        | mir::RuntimeFunc::InstanceSetField
        | mir::RuntimeFunc::GetTypeTag
        | mir::RuntimeFunc::IsinstanceClass
        | mir::RuntimeFunc::IsinstanceClassInherited
        | mir::RuntimeFunc::RegisterClass
        | mir::RuntimeFunc::RegisterClassFields
        | mir::RuntimeFunc::RegisterClassFieldCount
        | mir::RuntimeFunc::ObjectNew
        | mir::RuntimeFunc::RegisterDelFunc
        | mir::RuntimeFunc::RegisterCopyFunc
        | mir::RuntimeFunc::RegisterDeepCopyFunc
        | mir::RuntimeFunc::RegisterMethodName
        | mir::RuntimeFunc::IsSubclass => {
            instance::compile_instance_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Hash + Id: migrated to RuntimeFunc::Call (handled by generic handler)

        // Iterator operations
        mir::RuntimeFunc::MakeIterator { .. }
        | mir::RuntimeFunc::IterNext
        | mir::RuntimeFunc::IterNextNoExc
        | mir::RuntimeFunc::IterIsExhausted
        | mir::RuntimeFunc::IterEnumerate
        | mir::RuntimeFunc::Sorted { .. }
        | mir::RuntimeFunc::ZipNew
        | mir::RuntimeFunc::ZipNext
        | mir::RuntimeFunc::IterZip
        | mir::RuntimeFunc::MapNew
        | mir::RuntimeFunc::FilterNew
        | mir::RuntimeFunc::ReduceNew
        | mir::RuntimeFunc::ChainNew
        | mir::RuntimeFunc::ISliceNew
        | mir::RuntimeFunc::Zip3New
        | mir::RuntimeFunc::ZipNNew => {
            iterator::compile_iterator_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Set ops: migrated to RuntimeFunc::Call (handled by generic handler)

        // Container min/max operations (unified)
        mir::RuntimeFunc::ContainerMinMax {
            container,
            op,
            elem,
        } => {
            minmax::compile_container_minmax(builder, dest, *container, *op, *elem, args, ctx)?;
            Ok(())
        }

        // Comparison operations (unified)
        mir::RuntimeFunc::Compare { kind, op } => {
            compare::compile_compare_call(builder, dest, *kind, *op, args, ctx)?;
            Ok(())
        }

        // Bytes: migrated to RuntimeFunc::Call (handled by generic handler)

        // Object ops: migrated to RuntimeFunc::Call (handled by generic handler)

        // Global variable operations
        mir::RuntimeFunc::GlobalGet(_) | mir::RuntimeFunc::GlobalSet(_) => {
            globals::compile_global_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Class attribute operations
        mir::RuntimeFunc::ClassAttrGet(_) | mir::RuntimeFunc::ClassAttrSet(_) => {
            class_attrs::compile_class_attr_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Cell operations (for nonlocal variables)
        mir::RuntimeFunc::MakeCell(_)
        | mir::RuntimeFunc::CellGet(_)
        | mir::RuntimeFunc::CellSet(_) => {
            cells::compile_cell_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Generator operations
        mir::RuntimeFunc::MakeGenerator
        | mir::RuntimeFunc::GeneratorGetState
        | mir::RuntimeFunc::GeneratorSetState
        | mir::RuntimeFunc::GeneratorGetLocal
        | mir::RuntimeFunc::GeneratorSetLocal
        | mir::RuntimeFunc::GeneratorGetLocalPtr
        | mir::RuntimeFunc::GeneratorSetLocalPtr
        | mir::RuntimeFunc::GeneratorSetLocalType
        | mir::RuntimeFunc::GeneratorSetExhausted
        | mir::RuntimeFunc::GeneratorIsExhausted
        | mir::RuntimeFunc::GeneratorSend
        | mir::RuntimeFunc::GeneratorGetSentValue
        | mir::RuntimeFunc::GeneratorClose
        | mir::RuntimeFunc::GeneratorIsClosing => {
            generator::compile_generator_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Standard library operations (sys, os, re, json)
        // StdlibCall/StdlibAttrGet - unified handlers using definitions (Single Source of Truth)
        // ObjectFieldGet/ObjectMethodCall - generic object field/method access (Single Source of Truth)
        mir::RuntimeFunc::StdlibCall(_)
        | mir::RuntimeFunc::StdlibAttrGet(_)
        | mir::RuntimeFunc::ObjectFieldGet(_)
        | mir::RuntimeFunc::ObjectMethodCall(_) => {
            stdlib::compile_stdlib_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // File I/O: migrated to RuntimeFunc::Call (handled by generic handler)

        // Exception-related operations
        mir::RuntimeFunc::ExcIsinstanceClass
        | mir::RuntimeFunc::ExcRaiseCustom
        | mir::RuntimeFunc::ExcRegisterClassName
        | mir::RuntimeFunc::ExcInstanceStr => {
            compile_exception_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Descriptor-based call (generic handler)
        mir::RuntimeFunc::Call(def) => {
            compile_runtime_func_def(builder, dest, def, args, ctx)?;
            Ok(())
        }

        _ => {
            panic!(
                "codegen: unhandled RuntimeFunc variant: {:?}. \
                 Every RuntimeFunc must have a corresponding match arm in compile_runtime_call.",
                func
            );
        }
    }
}

use crate::gc::update_gc_root_if_needed;
use crate::utils::{
    create_raw_string_data, declare_runtime_function, load_operand, load_operand_as,
};
use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags};
use cranelift_codegen::isa::CallConv;
use cranelift_module::Module;
use pyaot_core_defs::runtime_func_def::{ParamType, ReturnType};
use pyaot_core_defs::RuntimeFuncDef;

/// Convert a descriptor ParamType to a Cranelift type.
fn param_type_to_cltype(pt: ParamType) -> cltypes::Type {
    match pt {
        ParamType::I64 => cltypes::I64,
        ParamType::F64 => cltypes::F64,
        ParamType::I8 => cltypes::I8,
        ParamType::I32 => cltypes::I32,
    }
}

/// Convert a descriptor ReturnType to a Cranelift type.
fn return_type_to_cltype(rt: ReturnType) -> cltypes::Type {
    match rt {
        ReturnType::I64 => cltypes::I64,
        ReturnType::F64 => cltypes::F64,
        ReturnType::I8 => cltypes::I8,
        ReturnType::I32 => cltypes::I32,
    }
}

/// Generic handler: compile any `RuntimeFunc::Call(&RuntimeFuncDef)`.
///
/// Builds the Cranelift signature from the descriptor, loads args with
/// automatic type coercion, emits the call, stores the result, and
/// optionally registers the result as a GC root.
fn compile_runtime_func_def(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    def: &RuntimeFuncDef,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    // Build Cranelift signature from descriptor
    let mut sig = ctx.module.make_signature();
    sig.call_conv = CallConv::SystemV;

    for &pt in def.params {
        sig.params.push(AbiParam::new(param_type_to_cltype(pt)));
    }
    if let Some(rt) = def.returns {
        sig.returns.push(AbiParam::new(return_type_to_cltype(rt)));
    }

    // Declare external function and get a reference for this function
    let func_id = declare_runtime_function(ctx.module, def.symbol, &sig)?;
    let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);

    // Load arguments with type coercion to match expected parameter types
    let arg_vals: Vec<_> = args
        .iter()
        .zip(def.params.iter())
        .map(|(arg, &pt)| load_operand_as(builder, arg, ctx.var_map, param_type_to_cltype(pt)))
        .collect();

    let call_inst = builder.ins().call(func_ref, &arg_vals);

    // Handle return value
    let dest_var = *ctx
        .var_map
        .get(&dest)
        .expect("internal error: local not in var_map - codegen bug");

    if def.returns.is_some() {
        let result = builder.inst_results(call_inst)[0];
        let result_type = builder.func.dfg.value_type(result);

        // Coerce the result to match the dest variable's declared type.
        // Dest variables can be I64 (int/ptr), I8 (bool), or F64 (float).
        let dest_val = builder.use_var(dest_var);
        let dest_type = builder.func.dfg.value_type(dest_val);

        let result_coerced = if result_type == dest_type {
            result
        } else {
            match (result_type, dest_type) {
                (cltypes::I8, cltypes::I64) | (cltypes::I32, cltypes::I64) => {
                    builder.ins().uextend(cltypes::I64, result)
                }
                (cltypes::I64, cltypes::I8) => builder.ins().ireduce(cltypes::I8, result),
                (cltypes::I64, cltypes::I32) => builder.ins().ireduce(cltypes::I32, result),
                (cltypes::F64, cltypes::I64) => {
                    builder.ins().bitcast(cltypes::I64, MemFlags::new(), result)
                }
                (cltypes::I64, cltypes::F64) => {
                    builder.ins().bitcast(cltypes::F64, MemFlags::new(), result)
                }
                _ => result,
            }
        };

        builder.def_var(dest_var, result_coerced);

        if def.gc_roots_result {
            update_gc_root_if_needed(builder, &dest, result_coerced, ctx.gc_frame_data);
        }
    } else {
        // Void function: leave dest variable unchanged.
        // MIR uses the same dest local for the call instruction even when
        // the function has no return value (e.g., TupleSet writes in-place
        // to a tuple that is already stored in the dest local).
    }

    Ok(())
}

/// Compile exception-related runtime calls
fn compile_exception_call(
    builder: &mut FunctionBuilder,
    _dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        mir::RuntimeFunc::ExcRegisterClassName => {
            // rt_exc_register_class_name(class_id: u8, name: *const u8, len: usize)
            let class_id_val = load_operand(builder, &args[0], ctx.var_map);
            let class_id_u8 = builder.ins().ireduce(cltypes::I8, class_id_val);

            // Get string data pointer and length
            if let Operand::Constant(mir::Constant::Str(s)) = &args[1] {
                let str_content = ctx.interner.resolve(*s);
                let str_len = str_content.len();
                let data_id = create_raw_string_data(ctx.module, *s, ctx.interner);
                let gv = ctx.module.declare_data_in_func(data_id, builder.func);
                let ptr = builder.ins().global_value(cltypes::I64, gv);
                let len = builder.ins().iconst(cltypes::I64, str_len as i64);

                let mut sig = ctx.module.make_signature();
                sig.call_conv = CallConv::SystemV;
                sig.params.push(AbiParam::new(cltypes::I8)); // class_id
                sig.params.push(AbiParam::new(cltypes::I64)); // name ptr
                sig.params.push(AbiParam::new(cltypes::I64)); // name len

                let func_id =
                    declare_runtime_function(ctx.module, "rt_exc_register_class_name", &sig)?;
                let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
                builder.ins().call(func_ref, &[class_id_u8, ptr, len]);
            }
        }
        mir::RuntimeFunc::ExcIsinstanceClass => {
            // Handled in exceptions.rs via compile_exc_check_class — no-op here
        }
        mir::RuntimeFunc::ExcRaiseCustom => {
            // Handled in exceptions.rs via compile_raise_custom — no-op here
        }
        mir::RuntimeFunc::ExcInstanceStr => {
            // rt_exc_instance_str(instance: *mut Obj) -> *mut Obj
            let instance_val = load_operand(builder, &args[0], ctx.var_map);

            let mut sig = ctx.module.make_signature();
            sig.call_conv = CallConv::SystemV;
            sig.params.push(AbiParam::new(cltypes::I64)); // instance ptr
            sig.returns.push(AbiParam::new(cltypes::I64)); // result str ptr

            let func_id = declare_runtime_function(ctx.module, "rt_exc_instance_str", &sig)?;
            let func_ref = ctx.module.declare_func_in_func(func_id, builder.func);
            let call = builder.ins().call(func_ref, &[instance_val]);
            let result = builder.inst_results(call)[0];

            // Store result in destination variable
            let var = *ctx
                .var_map
                .get(&_dest)
                .expect("internal error: local not in var_map");
            builder.def_var(var, result);

            // Update GC root if needed (result is a heap string)
            crate::gc::update_gc_root_if_needed(builder, &_dest, result, ctx.gc_frame_data);
        }
        _ => unreachable!("Non-exception function passed to compile_exception_call"),
    }
    Ok(())
}
